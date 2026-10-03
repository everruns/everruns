//! Form mode elicitation (MCP `2026-07-28`), answered through `ask_user`.
//!
//! An attached server may answer a `tools/call` with an MRTR `input_required`
//! result carrying an `elicitation/create` request in `mode: "form"`: a message
//! and a `requestedSchema` describing the values it wants. Everruns answers it
//! by turning the schema into an `ask_user` question set, letting the session's
//! own surface collect the answer, and returning that answer as the
//! `ElicitResult` of a retry.
//!
//! The design of record is `knowledge/integrations/mcp-form-elicitation.md`.
//! Three rules shape this module:
//!
//! - **The server does not author the attribution.** Every question's header is
//!   composed here from the configured server name; server strings only reach
//!   body positions (D2, TM-TOOL-043).
//! - **Credentials are refused, not collected.** A form answer rides a tool
//!   result into the event log, and `ask_user`'s `Secret` kind hands back a
//!   `session:` reference a remote server cannot use. So a credential-shaped
//!   property makes the whole elicitation unanswerable, and the error names URL
//!   mode, which the MCP specification provides for exactly this (D3,
//!   TM-TOOL-044).
//! - **Schemas are attacker-controlled input.** Anything out of profile or over
//!   a bound is refused, never trimmed, so nobody is shown a silently shortened
//!   question (D4, TM-TOOL-046).
//!
//! Like URL mode, a turn cannot block on a person. The first call finds no
//! answer and reports [`FormElicitationPending`]; the engine parks the turn on
//! an `ask_user` card; the API that collects the answer records it durably; and
//! the retry that follows finds it here.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// Most properties one elicitation may ask for.
pub const MAX_FORM_PROPERTIES: usize = 16;
/// Most members one enum property may offer.
pub const MAX_FORM_ENUM_MEMBERS: usize = 32;
/// Longest property name accepted. Names become question ids.
pub const MAX_FORM_PROPERTY_NAME_CHARS: usize = 64;
/// Longest property title, and longest enum value or label.
pub const MAX_FORM_TITLE_CHARS: usize = 200;
/// Longest property description, and longest elicitation message.
pub const MAX_FORM_TEXT_CHARS: usize = 2000;
/// `ask_user`'s own header limit. The attribution badge lives there.
const HEADER_CHARS: usize = 16;

/// How long a recorded answer stays usable. Matches URL mode's consent window.
pub const FORM_ANSWER_TTL: chrono::Duration = chrono::Duration::minutes(30);

/// Why an elicitation cannot be answered at all. The call fails with this, and
/// nothing is put in front of a person.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FormRefusal {
    /// THREAT[TM-TOOL-044]: a credential would reach the event log and model
    /// context. URL mode is the channel the specification provides for it.
    #[error(
        "property '{property}' asks for a credential; Everruns does not collect credentials \
         through form elicitation. The server should use URL mode elicitation instead"
    )]
    Credential { property: String },
    /// A property the restricted profile does not allow.
    #[error("property '{property}' is outside the form elicitation profile: {reason}")]
    OutOfProfile { property: String, reason: String },
    /// The schema as a whole is malformed or over a bound.
    #[error("requested schema {0}")]
    Schema(String),
}

/// What kind of value a property wants.
#[derive(Debug, Clone, PartialEq)]
pub enum FormPropertyKind {
    /// A closed set. `labels` are what a person sees, `values` what the server
    /// receives; they are index-aligned and equal when the schema supplied no
    /// labels.
    Enum {
        values: Vec<String>,
        labels: Vec<String>,
    },
    /// Free text. `format` is stated to the person and left to the server to
    /// enforce.
    Text { format: Option<String> },
    /// Yes or no.
    Boolean { default: Option<bool> },
    /// A number typed as text and re-validated on the way back.
    Number {
        integer: bool,
        minimum: Option<f64>,
        maximum: Option<f64>,
    },
}

/// One property of a validated `requestedSchema`.
#[derive(Debug, Clone, PartialEq)]
pub struct FormProperty {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub kind: FormPropertyKind,
    pub required: bool,
}

/// A `requestedSchema` that passed every rule in this module.
#[derive(Debug, Clone, PartialEq)]
pub struct FormSchema {
    /// Sorted by name so the projection and the fingerprint do not depend on
    /// how the server happened to order its object keys.
    pub properties: Vec<FormProperty>,
}

const YES: &str = "Yes";
const NO: &str = "No";

/// Credential vocabulary for property names and titles.
///
/// Matched against whole words after splitting on separators and camelCase,
/// so `pin` catches `pin` and `userPin` but not `spinner`.
const CREDENTIAL_WORDS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "passcode",
    "pwd",
    "secret",
    "token",
    "apikey",
    "credential",
    "credentials",
    "pin",
    "otp",
    "totp",
    "mfa",
    "2fa",
    "cvv",
    "cvc",
    "ssn",
];

/// Adjacent word pairs that name a credential although neither word does.
const CREDENTIAL_PAIRS: &[(&str, &str)] = &[
    ("api", "key"),
    ("access", "key"),
    ("private", "key"),
    ("secret", "key"),
    ("card", "number"),
    ("security", "code"),
    ("verification", "code"),
    ("one", "time"),
];

fn words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for ch in text.chars() {
        if !ch.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_lower = false;
            continue;
        }
        if ch.is_uppercase() && previous_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        previous_lower = ch.is_lowercase() || ch.is_ascii_digit();
        current.extend(ch.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Whether a name or title reads as asking for a credential.
pub fn names_a_credential(text: &str) -> bool {
    let words = words(text);
    words
        .iter()
        .any(|word| CREDENTIAL_WORDS.contains(&word.as_str()))
        || words.windows(2).any(|pair| {
            CREDENTIAL_PAIRS
                .iter()
                .any(|(first, second)| pair[0] == *first && pair[1] == *second)
        })
}

fn bounded(
    property: &str,
    field: &str,
    value: Option<&str>,
    max: usize,
) -> Result<(), FormRefusal> {
    match value {
        Some(text) if text.chars().count() > max => Err(FormRefusal::OutOfProfile {
            property: property.to_string(),
            reason: format!("{field} is longer than {max} characters"),
        }),
        _ => Ok(()),
    }
}

fn valid_property_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_FORM_PROPERTY_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// Validate a server's `requestedSchema` against the restricted profile.
///
/// One flat object whose properties are each a string, an enum of strings, a
/// boolean, a number, or an integer. Everything else is refused naming the
/// property, and so is anything credential-shaped.
pub fn parse_requested_schema(schema: &Value) -> Result<FormSchema, FormRefusal> {
    let object = schema
        .as_object()
        .ok_or_else(|| FormRefusal::Schema("is not an object".to_string()))?;
    if object.get("type").and_then(Value::as_str) != Some("object") {
        return Err(FormRefusal::Schema("must have type \"object\"".to_string()));
    }
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| FormRefusal::Schema("has no properties".to_string()))?;
    if properties.is_empty() {
        return Err(FormRefusal::Schema("has no properties".to_string()));
    }
    if properties.len() > MAX_FORM_PROPERTIES {
        return Err(FormRefusal::Schema(format!(
            "asks for {} properties; at most {MAX_FORM_PROPERTIES} are accepted",
            properties.len()
        )));
    }
    let required: Vec<&str> = object
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let mut parsed = Vec::with_capacity(properties.len());
    for (name, definition) in properties {
        if !valid_property_name(name) {
            return Err(FormRefusal::OutOfProfile {
                property: name.chars().take(MAX_FORM_PROPERTY_NAME_CHARS).collect(),
                reason: format!(
                    "names must be 1 to {MAX_FORM_PROPERTY_NAME_CHARS} letters, digits, '_', \
                     '-' or '.'"
                ),
            });
        }
        let definition = definition
            .as_object()
            .ok_or_else(|| FormRefusal::OutOfProfile {
                property: name.clone(),
                reason: "definition is not an object".to_string(),
            })?;
        let text = |key: &str| {
            definition
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let title = text("title");
        let description = text("description");
        bounded(name, "title", title.as_deref(), MAX_FORM_TITLE_CHARS)?;
        bounded(
            name,
            "description",
            description.as_deref(),
            MAX_FORM_TEXT_CHARS,
        )?;

        // THREAT[TM-TOOL-044]: checked before the type, so a credential is
        // refused as a credential whatever shape it arrives in.
        let format = text("format");
        if format.as_deref() == Some("password")
            || definition.get("writeOnly").and_then(Value::as_bool) == Some(true)
            || names_a_credential(name)
            || title.as_deref().is_some_and(names_a_credential)
        {
            return Err(FormRefusal::Credential {
                property: name.clone(),
            });
        }
        if definition.contains_key("$ref") {
            return Err(FormRefusal::OutOfProfile {
                property: name.clone(),
                reason: "$ref is not supported".to_string(),
            });
        }

        let kind = match definition.get("type").and_then(Value::as_str) {
            Some("string") => match definition.get("enum") {
                Some(members) => enum_kind(name, members, definition.get("enumNames"))?,
                None => {
                    bounded(name, "format", format.as_deref(), MAX_FORM_TITLE_CHARS)?;
                    FormPropertyKind::Text { format }
                }
            },
            Some("boolean") => FormPropertyKind::Boolean {
                default: definition.get("default").and_then(Value::as_bool),
            },
            Some(kind @ ("number" | "integer")) => FormPropertyKind::Number {
                integer: kind == "integer",
                minimum: definition.get("minimum").and_then(Value::as_f64),
                maximum: definition.get("maximum").and_then(Value::as_f64),
            },
            Some(other) => {
                return Err(FormRefusal::OutOfProfile {
                    property: name.clone(),
                    reason: format!("type \"{other}\" is not a primitive"),
                });
            }
            None => {
                return Err(FormRefusal::OutOfProfile {
                    property: name.clone(),
                    reason: "has no type".to_string(),
                });
            }
        };
        parsed.push(FormProperty {
            required: required.contains(&name.as_str()),
            name: name.clone(),
            title,
            description,
            kind,
        });
    }
    parsed.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(FormSchema { properties: parsed })
}

fn enum_kind(
    property: &str,
    members: &Value,
    names: Option<&Value>,
) -> Result<FormPropertyKind, FormRefusal> {
    let refuse = |reason: String| FormRefusal::OutOfProfile {
        property: property.to_string(),
        reason,
    };
    let members = members
        .as_array()
        .ok_or_else(|| refuse("enum is not an array".to_string()))?;
    if members.is_empty() {
        return Err(refuse("enum is empty".to_string()));
    }
    if members.len() > MAX_FORM_ENUM_MEMBERS {
        return Err(refuse(format!(
            "enum has {} members; at most {MAX_FORM_ENUM_MEMBERS} are accepted",
            members.len()
        )));
    }
    let values = members
        .iter()
        .map(|member| member.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| refuse("enum members must be strings".to_string()))?;
    let labels = match names.and_then(Value::as_array) {
        Some(names) if names.len() == values.len() => names
            .iter()
            .map(|name| name.as_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| refuse("enumNames must be strings".to_string()))?,
        Some(_) => return Err(refuse("enumNames does not match enum".to_string())),
        None => values.clone(),
    };
    for text in values.iter().chain(labels.iter()) {
        if text.trim().is_empty() || text.chars().count() > MAX_FORM_TITLE_CHARS {
            return Err(refuse(format!(
                "enum members and names must be 1 to {MAX_FORM_TITLE_CHARS} characters"
            )));
        }
    }
    let mut seen = std::collections::HashSet::new();
    if !labels.iter().all(|label| seen.insert(label)) {
        return Err(refuse("enum labels must be unique".to_string()));
    }
    Ok(FormPropertyKind::Enum { values, labels })
}

/// The attribution badge: the configured server name, never a schema string.
fn attribution_header(server_name: &str) -> String {
    if server_name.chars().count() <= HEADER_CHARS {
        return server_name.to_string();
    }
    let mut header: String = server_name.chars().take(HEADER_CHARS - 1).collect();
    header.push('…');
    header
}

fn number_text(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        value.to_string()
    }
}

impl FormSchema {
    /// The schema as `ask_user` questions, one per property, in schema order.
    ///
    /// THREAT[TM-TOOL-043]: `header` is composed from `server_name`, the name
    /// the operator configured. Server-supplied titles and descriptions only
    /// fill the question body and option descriptions.
    pub fn questions(&self, server_name: &str) -> Vec<Value> {
        let header = attribution_header(server_name);
        self.properties
            .iter()
            .map(|property| {
                let mut text = property
                    .title
                    .clone()
                    .unwrap_or_else(|| property.name.clone());
                if let Some(description) = property.description.as_deref()
                    && !description.trim().is_empty()
                {
                    text.push_str(": ");
                    text.push_str(description);
                }
                let (kind, options) = match &property.kind {
                    FormPropertyKind::Enum { values, labels } => (
                        "choice",
                        labels
                            .iter()
                            .zip(values)
                            .map(|(label, value)| {
                                json!({
                                    "label": label,
                                    "description": if label == value {
                                        String::new()
                                    } else {
                                        value.clone()
                                    },
                                })
                            })
                            .collect::<Vec<_>>(),
                    ),
                    FormPropertyKind::Boolean { default } => (
                        "choice",
                        vec![
                            json!({"label": YES, "description": "", "default": *default == Some(true)}),
                            json!({"label": NO, "description": "", "default": *default == Some(false)}),
                        ],
                    ),
                    FormPropertyKind::Text { format } => {
                        if let Some(format) = format {
                            text.push_str(&format!(" (format: {format})"));
                        }
                        ("text", Vec::new())
                    }
                    FormPropertyKind::Number {
                        integer,
                        minimum,
                        maximum,
                    } => {
                        let noun = if *integer { "a whole number" } else { "a number" };
                        let range = match (minimum, maximum) {
                            (Some(min), Some(max)) => {
                                format!(" from {} to {}", number_text(*min), number_text(*max))
                            }
                            (Some(min), None) => format!(", at least {}", number_text(*min)),
                            (None, Some(max)) => format!(", at most {}", number_text(*max)),
                            (None, None) => String::new(),
                        };
                        text.push_str(&format!(" ({noun}{range})"));
                        ("text", Vec::new())
                    }
                };
                json!({
                    "kind": kind,
                    "id": property.name,
                    "header": header,
                    "question": text,
                    "multi_select": false,
                    // A server's enum is closed; free text beside it would be a
                    // value the server never offered.
                    "allow_other": kind == "text",
                    "options": options,
                })
            })
            .collect()
    }

    /// Stable digest of what was asked.
    ///
    /// A recorded answer carries the fingerprint of the questions the person
    /// saw, and is honoured only when the retry asks the same thing. A server
    /// that swaps its schema between the ask and the retry gets a fresh prompt,
    /// not answers given to a different question.
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        for property in &self.properties {
            hasher.update(property.name.as_bytes());
            hasher.update([0]);
            hasher.update(property.title.as_deref().unwrap_or_default().as_bytes());
            hasher.update([0]);
            hasher.update(
                property
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .as_bytes(),
            );
            hasher.update([0]);
            hasher.update(format!("{:?}", property.kind).as_bytes());
            hasher.update([u8::from(property.required), 0xff]);
        }
        let digest = hasher.finalize();
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Turn recorded answers into `ElicitResult.content`.
    ///
    /// Every property needs an answer that fits its type. Anything missing or
    /// non-conforming is an error, which the handler treats as "ask again"
    /// rather than coercing a value the person did not give (D4, D5).
    pub fn content_from_answers(
        &self,
        answers: &BTreeMap<String, FormAnswer>,
    ) -> Result<Map<String, Value>, String> {
        let mut content = Map::new();
        for property in &self.properties {
            let name = &property.name;
            let answer = answers
                .get(name)
                .ok_or_else(|| format!("'{name}' was not answered"))?;
            let text = answer
                .other_text
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty());
            let value = match &property.kind {
                FormPropertyKind::Enum { values, labels } => {
                    let [label] = answer.selected.as_slice() else {
                        return Err(format!("'{name}' needs exactly one choice"));
                    };
                    let index = labels
                        .iter()
                        .position(|candidate| candidate == label)
                        .ok_or_else(|| format!("'{name}' was not offered {label:?}"))?;
                    Value::String(values[index].clone())
                }
                FormPropertyKind::Boolean { .. } => match answer.selected.as_slice() {
                    [label] if label == YES => Value::Bool(true),
                    [label] if label == NO => Value::Bool(false),
                    _ => return Err(format!("'{name}' needs yes or no")),
                },
                FormPropertyKind::Text { .. } => Value::String(
                    text.ok_or_else(|| format!("'{name}' has no text"))?
                        .to_string(),
                ),
                FormPropertyKind::Number {
                    integer,
                    minimum,
                    maximum,
                } => {
                    let text = text.ok_or_else(|| format!("'{name}' has no number"))?;
                    let number: f64 = text
                        .parse()
                        .ok()
                        .filter(|number: &f64| number.is_finite())
                        .ok_or_else(|| format!("'{name}' is not a number"))?;
                    if minimum.is_some_and(|min| number < min)
                        || maximum.is_some_and(|max| number > max)
                    {
                        return Err(format!("'{name}' is out of range"));
                    }
                    if *integer {
                        if number.fract() != 0.0 || number.abs() > i64::MAX as f64 {
                            return Err(format!("'{name}' is not a whole number"));
                        }
                        json!(number as i64)
                    } else {
                        json!(number)
                    }
                }
            };
            content.insert(name.clone(), value);
        }
        Ok(content)
    }
}

/// One recorded answer, in `ask_user`'s own answer shape.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormAnswer {
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(default)]
    pub other_text: Option<String>,
}

/// What the person did with the questions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormAnswerAction {
    Accept,
    /// Declined, dismissed, timed out, or nobody could be asked. The server is
    /// told `decline` in every case: an empty `accept` would read as an answer
    /// (D5, TM-TOOL-047).
    Decline,
}

/// The durable form of an answer, written by the API that collected it and
/// read by the MCP client on the retry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredFormAnswer {
    /// Re-checked on read so a key collision fails closed.
    pub server: String,
    pub tool: String,
    /// [`FormSchema::fingerprint`] of the questions the person answered.
    pub fingerprint: String,
    pub action: FormAnswerAction,
    /// Keyed by property name, which is also the question id.
    #[serde(default)]
    pub answers: BTreeMap<String, FormAnswer>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl StoredFormAnswer {
    pub fn new(
        server: &str,
        tool: &str,
        fingerprint: &str,
        action: FormAnswerAction,
        answers: BTreeMap<String, FormAnswer>,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Self {
        Self {
            server: server.to_string(),
            tool: tool.to_string(),
            fingerprint: fingerprint.to_string(),
            action,
            answers,
            expires_at: now + FORM_ANSWER_TTL,
        }
    }

    /// Honour the record only for the pairing and questions it names, and
    /// only while fresh.
    pub fn applies_to(
        &self,
        server: &str,
        tool: &str,
        fingerprint: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> bool {
        self.server == server
            && self.tool == tool
            && self.fingerprint == fingerprint
            && self.expires_at > now
    }
}

/// Session-storage key an answer for `server`/`tool` is recorded under.
///
/// THREAT[TM-TOOL-034]: the prefix is reserved from the model-facing
/// `kv_store` tool and the storage listing, so only the question-answer API
/// writes it.
pub fn form_answer_storage_key(server: &str, tool: &str) -> String {
    fn fold(part: &str) -> String {
        part.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }
    format!(
        "{}{}/{}",
        crate::capabilities::MCP_ELICITATION_FORM_KV_PREFIX,
        fold(server),
        fold(tool)
    )
}

/// Durable, single-use store of recorded answers.
#[async_trait]
pub trait FormAnswerStore: Send + Sync {
    /// Consume the answer recorded for `server`/`tool`, if any. Destructive by
    /// contract: one answer is sent at most once.
    async fn take_form_answer(&self, server: &str, tool: &str) -> Result<Option<StoredFormAnswer>>;
}

/// A form elicitation the server sent, validated and projected.
#[derive(Debug, Clone)]
pub struct FormElicitation {
    pub server_name: String,
    pub tool_name: String,
    pub key: String,
    pub message: String,
    pub schema: FormSchema,
}

/// What a handler decided.
#[derive(Debug, Clone, PartialEq)]
pub enum FormOutcome {
    Accept(Map<String, Value>),
    Decline,
    /// Nobody has answered yet; put the questions in front of a person.
    Ask,
}

/// Answers form mode elicitations for a host. A host that injects none leaves
/// form mode undeclared, so a compliant server cannot ask (D7).
#[async_trait]
pub trait FormElicitationHandler: Send + Sync {
    async fn answer_form(&self, elicitation: &FormElicitation) -> Result<FormOutcome>;
}

/// Handler for hosts that pause a turn and collect answers out of band: it
/// sends what a person recorded, and asks when nothing usable is recorded.
pub struct StoredFormAnswers {
    store: Arc<dyn FormAnswerStore>,
}

impl StoredFormAnswers {
    pub fn new(store: Arc<dyn FormAnswerStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl FormElicitationHandler for StoredFormAnswers {
    async fn answer_form(&self, elicitation: &FormElicitation) -> Result<FormOutcome> {
        // An unreachable store must not turn into an answer nobody gave.
        let record = match self
            .store
            .take_form_answer(&elicitation.server_name, &elicitation.tool_name)
            .await
        {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(
                    server = %elicitation.server_name,
                    tool = %elicitation.tool_name,
                    %error,
                    "Could not read a recorded form answer; asking again"
                );
                None
            }
        };
        let fingerprint = elicitation.schema.fingerprint();
        let Some(record) = record.filter(|record| {
            record.applies_to(
                &elicitation.server_name,
                &elicitation.tool_name,
                &fingerprint,
                chrono::Utc::now(),
            )
        }) else {
            return Ok(FormOutcome::Ask);
        };
        match record.action {
            FormAnswerAction::Decline => Ok(FormOutcome::Decline),
            FormAnswerAction::Accept => {
                match elicitation.schema.content_from_answers(&record.answers) {
                    Ok(content) => Ok(FormOutcome::Accept(content)),
                    Err(reason) => {
                        tracing::warn!(
                            server = %elicitation.server_name,
                            tool = %elicitation.tool_name,
                            %reason,
                            "Recorded form answer does not fit the schema; asking again"
                        );
                        Ok(FormOutcome::Ask)
                    }
                }
            }
        }
    }
}

/// A form elicitation that stopped a tool call until a person answers.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("MCP server '{server_name}' has questions for you before tool '{tool_name}' can run")]
pub struct FormElicitationPending {
    pub server_name: String,
    pub tool_name: String,
    pub message: String,
    /// `ask_user` questions, already attributed.
    pub questions: Vec<Value>,
    pub fingerprint: String,
}

impl FormElicitation {
    pub fn pending(&self) -> FormElicitationPending {
        FormElicitationPending {
            server_name: self.server_name.clone(),
            tool_name: self.tool_name.clone(),
            message: self.message.clone(),
            questions: self.schema.questions(&self.server_name),
            fingerprint: self.schema.fingerprint(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(properties: Value) -> Value {
        json!({"type": "object", "properties": properties})
    }

    #[test]
    fn projects_the_flat_profile_with_everruns_owned_attribution() {
        let parsed = parse_requested_schema(&json!({
            "type": "object",
            "properties": {
                "plan": {"type": "string", "title": "Plan", "enum": ["pro", "team"],
                         "enumNames": ["Pro", "Team"], "description": "Which plan to buy"},
                "seats": {"type": "integer", "title": "Seats", "minimum": 1, "maximum": 50},
                "notify": {"type": "boolean", "title": "Notify me", "default": true},
                "email": {"type": "string", "format": "email"}
            },
            "required": ["plan", "seats"]
        }))
        .expect("in profile");

        let questions = parsed.questions("billing-server-with-a-long-name");
        assert_eq!(questions.len(), 4);
        // Sorted by name: email, notify, plan, seats.
        let ids: Vec<&str> = questions
            .iter()
            .map(|q| q["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["email", "notify", "plan", "seats"]);
        for question in &questions {
            assert_eq!(question["header"], "billing-server-…");
        }
        assert_eq!(questions[0]["kind"], "text");
        assert_eq!(questions[0]["question"], "email (format: email)");
        assert_eq!(questions[1]["options"][0]["label"], "Yes");
        assert_eq!(questions[1]["options"][0]["default"], true);
        assert_eq!(questions[2]["kind"], "choice");
        assert_eq!(questions[2]["allow_other"], false);
        assert_eq!(questions[2]["question"], "Plan: Which plan to buy");
        assert_eq!(questions[2]["options"][1]["label"], "Team");
        assert_eq!(questions[2]["options"][1]["description"], "team");
        assert_eq!(
            questions[3]["question"],
            "Seats (a whole number from 1 to 50)"
        );
        assert!(
            parsed
                .properties
                .iter()
                .any(|p| p.name == "plan" && p.required)
        );
    }

    #[test]
    fn credential_shaped_properties_make_the_elicitation_unanswerable() {
        for (name, definition) in [
            (
                "secret_value",
                json!({"type": "string", "format": "password"}),
            ),
            ("hidden", json!({"type": "string", "writeOnly": true})),
            ("password", json!({"type": "string"})),
            ("userPin", json!({"type": "string"})),
            ("api_key", json!({"type": "string"})),
            ("apiKey", json!({"type": "string"})),
            (
                "confirm",
                json!({"type": "string", "title": "Confirm your password"}),
            ),
            ("card", json!({"type": "string", "title": "Card number"})),
            (
                "choice",
                json!({"type": "string", "title": "Access token", "enum": ["a"]}),
            ),
        ] {
            let refusal = parse_requested_schema(&schema(json!({ name: definition })))
                .expect_err("credential must be refused");
            assert_eq!(
                refusal,
                FormRefusal::Credential {
                    property: name.to_string()
                },
                "{name}"
            );
            assert!(refusal.to_string().contains("URL mode"));
        }
        // Words that merely contain a credential word are not credentials.
        for name in ["spinner", "tokenizer_choice", "keyboard", "opinion"] {
            assert!(
                parse_requested_schema(&schema(json!({ name: {"type": "string"} }))).is_ok(),
                "{name} must not be treated as a credential"
            );
        }
    }

    #[test]
    fn out_of_profile_and_over_bound_schemas_are_refused_not_trimmed() {
        let cases = [
            schema(json!({"tags": {"type": "array", "items": {"type": "string"}}})),
            schema(json!({"address": {"type": "object", "properties": {}}})),
            schema(json!({"linked": {"$ref": "#/defs/x", "type": "string"}})),
            schema(json!({"untyped": {}})),
            schema(json!({"bad name": {"type": "string"}})),
            schema(json!({"mixed": {"type": "string", "enum": ["a", 1]}})),
            schema(json!({"named": {"type": "string", "enum": ["a", "b"], "enumNames": ["A"]}})),
            json!({"type": "array"}),
            schema(json!({})),
        ];
        for case in cases {
            assert!(
                parse_requested_schema(&case).is_err(),
                "{case} must be refused"
            );
        }

        let many: Map<String, Value> = (0..=MAX_FORM_PROPERTIES)
            .map(|i| (format!("p{i}"), json!({"type": "string"})))
            .collect();
        assert!(matches!(
            parse_requested_schema(&schema(Value::Object(many))),
            Err(FormRefusal::Schema(_))
        ));
        let members: Vec<String> = (0..=MAX_FORM_ENUM_MEMBERS)
            .map(|i| format!("m{i}"))
            .collect();
        assert!(matches!(
            parse_requested_schema(&schema(
                json!({"pick": {"type": "string", "enum": members}})
            )),
            Err(FormRefusal::OutOfProfile { .. })
        ));
        let long = "x".repeat(MAX_FORM_TEXT_CHARS + 1);
        assert!(matches!(
            parse_requested_schema(&schema(
                json!({"q": {"type": "string", "description": long}})
            )),
            Err(FormRefusal::OutOfProfile { .. })
        ));
    }

    fn answer(selected: &[&str], text: Option<&str>) -> FormAnswer {
        FormAnswer {
            selected: selected.iter().map(|s| s.to_string()).collect(),
            other_text: text.map(str::to_string),
        }
    }

    #[test]
    fn answers_convert_to_typed_content_and_bad_ones_are_refused() {
        let parsed = parse_requested_schema(&schema(json!({
            "plan": {"type": "string", "enum": ["pro", "team"], "enumNames": ["Pro", "Team"]},
            "seats": {"type": "integer", "minimum": 1, "maximum": 50},
            "ratio": {"type": "number"},
            "notify": {"type": "boolean"},
            "name": {"type": "string"}
        })))
        .unwrap();
        let good = BTreeMap::from([
            ("plan".to_string(), answer(&["Team"], None)),
            ("seats".to_string(), answer(&[], Some(" 12 "))),
            ("ratio".to_string(), answer(&[], Some("0.5"))),
            ("notify".to_string(), answer(&["No"], None)),
            ("name".to_string(), answer(&[], Some("Ada"))),
        ]);
        let content = parsed.content_from_answers(&good).expect("fits");
        assert_eq!(
            Value::Object(content),
            json!({"plan": "team", "seats": 12, "ratio": 0.5, "notify": false, "name": "Ada"})
        );

        let with = |name: &str, replacement: FormAnswer| {
            let mut answers = good.clone();
            answers.insert(name.to_string(), replacement);
            parsed.content_from_answers(&answers)
        };
        assert!(with("seats", answer(&[], Some("51"))).is_err());
        assert!(with("seats", answer(&[], Some("2.5"))).is_err());
        assert!(with("seats", answer(&[], Some("twelve"))).is_err());
        assert!(with("ratio", answer(&[], Some("NaN"))).is_err());
        assert!(
            with("plan", answer(&["team"], None)).is_err(),
            "value is not a label"
        );
        assert!(with("notify", answer(&[], None)).is_err());
        assert!(with("name", answer(&[], Some("   "))).is_err());
        let mut missing = good.clone();
        missing.remove("name");
        assert!(parsed.content_from_answers(&missing).is_err());
    }

    #[test]
    fn fingerprint_ignores_key_order_but_not_content() {
        let a = parse_requested_schema(&schema(json!({
            "a": {"type": "string"}, "b": {"type": "boolean"}
        })))
        .unwrap();
        let b = parse_requested_schema(&schema(json!({
            "b": {"type": "boolean"}, "a": {"type": "string"}
        })))
        .unwrap();
        let c = parse_requested_schema(&schema(json!({
            "a": {"type": "string", "title": "Something else"}, "b": {"type": "boolean"}
        })))
        .unwrap();
        assert_eq!(a.fingerprint(), b.fingerprint());
        assert_ne!(a.fingerprint(), c.fingerprint());
    }

    struct OneShot(std::sync::Mutex<Option<StoredFormAnswer>>);

    #[async_trait]
    impl FormAnswerStore for OneShot {
        async fn take_form_answer(&self, _: &str, _: &str) -> Result<Option<StoredFormAnswer>> {
            Ok(self.0.lock().expect("lock").take())
        }
    }

    fn elicitation(schema_value: Value) -> FormElicitation {
        FormElicitation {
            server_name: "billing".to_string(),
            tool_name: "buy".to_string(),
            key: "details".to_string(),
            message: "Pick a plan".to_string(),
            schema: parse_requested_schema(&schema_value).unwrap(),
        }
    }

    #[tokio::test]
    async fn stored_answers_are_sent_once_and_only_for_the_questions_asked() {
        let asked = elicitation(schema(json!({"plan": {"type": "string", "enum": ["pro"]}})));
        let record = |fingerprint: &str, action| {
            StoredFormAnswer::new(
                "billing",
                "buy",
                fingerprint,
                action,
                BTreeMap::from([("plan".to_string(), answer(&["pro"], None))]),
                chrono::Utc::now(),
            )
        };

        let handler = StoredFormAnswers::new(Arc::new(OneShot(std::sync::Mutex::new(None))));
        assert_eq!(handler.answer_form(&asked).await.unwrap(), FormOutcome::Ask);

        let store = Arc::new(OneShot(std::sync::Mutex::new(Some(record(
            &asked.schema.fingerprint(),
            FormAnswerAction::Accept,
        )))));
        let handler = StoredFormAnswers::new(store);
        assert_eq!(
            handler.answer_form(&asked).await.unwrap(),
            FormOutcome::Accept(json!({"plan": "pro"}).as_object().unwrap().clone())
        );
        assert_eq!(
            handler.answer_form(&asked).await.unwrap(),
            FormOutcome::Ask,
            "one answer is sent at most once"
        );

        // A server that changed its questions after the person answered gets
        // a fresh prompt, not the old answer.
        let handler = StoredFormAnswers::new(Arc::new(OneShot(std::sync::Mutex::new(Some(
            record("stale", FormAnswerAction::Accept),
        )))));
        assert_eq!(handler.answer_form(&asked).await.unwrap(), FormOutcome::Ask);

        let handler = StoredFormAnswers::new(Arc::new(OneShot(std::sync::Mutex::new(Some(
            record(&asked.schema.fingerprint(), FormAnswerAction::Decline),
        )))));
        assert_eq!(
            handler.answer_form(&asked).await.unwrap(),
            FormOutcome::Decline
        );
    }

    #[test]
    fn stored_answers_expire_and_stay_scoped() {
        let now = chrono::Utc::now();
        let record = StoredFormAnswer::new(
            "billing",
            "buy",
            "fp",
            FormAnswerAction::Accept,
            BTreeMap::new(),
            now,
        );
        assert!(record.applies_to("billing", "buy", "fp", now));
        assert!(!record.applies_to("other", "buy", "fp", now));
        assert!(!record.applies_to("billing", "refund", "fp", now));
        assert!(!record.applies_to("billing", "buy", "fp", now + FORM_ANSWER_TTL));
        assert_eq!(
            form_answer_storage_key("acme/billing", "buy"),
            "mcp/elicitation-form/acme_billing/buy"
        );
    }
}
