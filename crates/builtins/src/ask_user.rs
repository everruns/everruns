use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::{
    FinalizedToolCallRejection, FinalizedToolCallsContext, FinalizedToolCallsHook,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::capabilities::{Capability, CapabilityLocalization};
use crate::tool_types::{
    ClientSideTool, DeferrablePolicy, HUMAN_INTENT_ARGUMENT, ToolCall, ToolDefinition, ToolHints,
};
use crate::tools::{Tool, ToolExecutionResult};

pub const ASK_USER_CAPABILITY_ID: &str = "ask_user";
// Defined in `everruns-provider` so the engine can recognise the call without
// depending on this crate; re-exported here so capability authors keep one path.
pub use crate::tool_types::ASK_USER_TOOL_NAME;
pub const DEFAULT_ASK_USER_TIMEOUT_SECONDS: u64 = 300;
pub const MAX_ASK_USER_QUESTIONS: usize = 4;
pub const MAX_ASK_USER_OPTIONS: usize = 6;
pub const MAX_ASK_USER_HEADER_CHARS: usize = 16;
pub const MAX_ASK_USER_SECRET_NAME_CHARS: usize = 255;
/// Scheme of the handle a secret answer returns. The value itself never leaves
/// the encrypted session-secret store, so the model is handed a name to resolve
/// rather than a credential to carry (EVE-1058).
pub const SESSION_SECRET_REF_PREFIX: &str = "session:";

/// The handle a resolved secret question returns in place of the value.
pub fn session_secret_ref(name: &str) -> String {
    format!("{SESSION_SECRET_REF_PREFIX}{name}")
}

/// How long before `expires_at` the card starts counting down, at the default
/// timeout. Shorter windows scale this down rather than nudging before the
/// question was even asked; see `deadlines_for`.
pub const ASK_USER_NUDGE_LEAD_SECONDS: u64 = 60;

fn default_timeout_seconds() -> u64 {
    DEFAULT_ASK_USER_TIMEOUT_SECONDS
}

/// The three deadlines the server stamps on a normalized `ask_user` call.
///
/// Emitted server-side (EVE-1056) rather than derived by each surface: the
/// sweep resolves the call at `expires_at`, so a client that guessed its own
/// deadline would render a countdown the server never agreed to.
///
/// The nudge keeps its shipped shape at the default 300s timeout — 60s before
/// expiry, the intended four-minute mark — but scales with the window below
/// that, because a fixed 60s lead on a 10s timeout would put the nudge before
/// `asked_at`.
pub fn deadlines_for(
    asked_at: chrono::DateTime<chrono::Utc>,
    timeout_seconds: u64,
) -> (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>) {
    let seconds = timeout_seconds.min(i64::MAX as u64) as i64;
    let expires_at = asked_at + chrono::Duration::seconds(seconds);
    // `div_ceil` keeps a 1s timeout from collapsing the lead to zero.
    let lead = ASK_USER_NUDGE_LEAD_SECONDS.min(timeout_seconds.div_ceil(5)) as i64;
    let nudge_at = expires_at - chrono::Duration::seconds(lead);
    (nudge_at, expires_at)
}

fn default_allow_other() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AskUserQuestionKind {
    #[default]
    Choice,
    /// Collect a credential. The answer carries a `secret_ref`, never a value:
    /// an `ask_user` answer is a tool result, so a value here would be
    /// plaintext in the event log *and* permanently in model context
    /// (TM-AGENT-016).
    Secret,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserOption {
    pub label: String,
    pub description: String,
    #[serde(default, rename = "default")]
    pub is_default: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserQuestion {
    #[serde(default)]
    pub kind: AskUserQuestionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default = "default_allow_other")]
    pub allow_other: bool,
    /// Offered choices. A `secret` question carries none.
    #[serde(default)]
    pub options: Vec<AskUserOption>,
    /// Name to store the credential under, on a `secret` question only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_name: Option<String>,
    /// What the credential will be used for, on a `secret` question only.
    /// Required, because nobody should type a key without being told why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserRequest {
    pub questions: Vec<AskUserQuestion>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
    /// When the question was asked. Stamped by normalization.
    ///
    /// These three are server-authoritative: whatever the model sent is
    /// discarded and overwritten, because the deadline the sweep acts on must
    /// not be one the caller chose for itself. They are `Option` only so a
    /// pre-normalization payload deserializes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_at: Option<String>,
    /// When the surface should start warning that time is running out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nudge_at: Option<String>,
    /// When the server resolves the call with declared defaults (EVE-1056).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskUserStatus {
    Answered,
    Declined,
    Cancelled,
    TimedOut,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskUserAnsweredBy {
    User,
    Timeout,
    Unattended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserAnswer {
    pub id: String,
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(default)]
    pub other_text: Option<String>,
    /// Handle to the stored credential answering a `secret` question.
    ///
    /// There is deliberately no `value` field on this type — not empty,
    /// absent — so no code path can carry a collected secret into a tool
    /// result. Tools resolve the name against the session secret store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserResult {
    pub status: AskUserStatus,
    pub answered_by: AskUserAnsweredBy,
    pub answers: Vec<AskUserAnswer>,
}
/// A host that can answer structured questions while a tool call is in flight.
#[async_trait]
pub trait AskUser: Send + Sync {
    /// Ask the host to answer one normalized batch of questions.
    async fn ask(&self, questions: &[AskUserQuestion]) -> AskUserResult;
}
#[async_trait]
impl<T: AskUser + ?Sized> AskUser for Arc<T> {
    async fn ask(&self, questions: &[AskUserQuestion]) -> AskUserResult {
        self.as_ref().ask(questions).await
    }
}

/// An unattended responder that applies declared defaults or the first option.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultsResponder;

#[async_trait]
impl AskUser for DefaultsResponder {
    async fn ask(&self, questions: &[AskUserQuestion]) -> AskUserResult {
        // A "default credential" is meaningless, so a secret question has no
        // unattended answer to apply. Declining says so; proceeding without the
        // credential is then the model's explicit decision (EVE-1058).
        if questions_ask_for_a_secret(questions) {
            return AskUserResult {
                status: AskUserStatus::Declined,
                answered_by: AskUserAnsweredBy::Unattended,
                answers: Vec::new(),
            };
        }
        AskUserResult {
            status: AskUserStatus::Answered,
            answered_by: AskUserAnsweredBy::Unattended,
            answers: declared_defaults(questions),
        }
    }
}

/// The answers an unanswered batch resolves to: each question's declared
/// default, or its first option when the model declared none.
///
/// Shared by the unattended responder and the deadline sweep (EVE-1056) so the
/// two cannot drift. Callers must check [`questions_ask_for_a_secret`] first —
/// a credential has no default, and this would hand back an empty `selected`
/// that reads as an answer.
pub fn declared_defaults(questions: &[AskUserQuestion]) -> Vec<AskUserAnswer> {
    questions
        .iter()
        .map(|question| {
            let mut selected = question
                .options
                .iter()
                .filter(|option| option.is_default)
                .map(|option| option.label.clone())
                .collect::<Vec<_>>();
            if selected.is_empty() {
                selected.extend(question.options.first().map(|option| option.label.clone()));
            }
            AskUserAnswer {
                id: question.id.clone().unwrap_or_default(),
                selected,
                other_text: None,
                secret_ref: None,
            }
        })
        .collect()
}

/// Whether any question in a batch collects a credential.
pub fn questions_ask_for_a_secret(questions: &[AskUserQuestion]) -> bool {
    questions
        .iter()
        .any(|question| question.kind == AskUserQuestionKind::Secret)
}

pub fn validate_ask_user_request(request: &AskUserRequest) -> Result<(), String> {
    if !(1..=MAX_ASK_USER_QUESTIONS).contains(&request.questions.len()) {
        return Err(format!(
            "questions must contain between 1 and {MAX_ASK_USER_QUESTIONS} items"
        ));
    }
    if !(1..=DEFAULT_ASK_USER_TIMEOUT_SECONDS).contains(&request.timeout_seconds) {
        return Err(format!(
            "timeout_seconds must be between 1 and {DEFAULT_ASK_USER_TIMEOUT_SECONDS}"
        ));
    }

    let mut ids = HashSet::new();
    for (index, question) in request.questions.iter().enumerate() {
        let position = index + 1;
        if let Some(id) = question.id.as_deref() {
            if id.trim().is_empty() {
                return Err(format!("question {position} id must not be empty"));
            }
            if !ids.insert(id) {
                return Err(format!("question ids must be unique; duplicate {id:?}"));
            }
        }
        if question.header.trim().is_empty() {
            return Err(format!("question {position} header must not be empty"));
        }
        if question.header.chars().count() > MAX_ASK_USER_HEADER_CHARS {
            return Err(format!(
                "question {position} header must not exceed {MAX_ASK_USER_HEADER_CHARS} characters"
            ));
        }
        if question.question.trim().is_empty() {
            return Err(format!("question {position} text must not be empty"));
        }

        if question.kind == AskUserQuestionKind::Secret {
            // One secret per call, alone. A batch mixing a credential with
            // choices has no single honest unattended outcome (declining it
            // would throw away answerable choices, answering it would claim a
            // credential nobody supplied), and the card is a password field
            // rather than a form.
            if request.questions.len() != 1 {
                return Err("a secret question must be the only question in the call".to_string());
            }
            if !question.options.is_empty() {
                return Err(format!(
                    "question {position} is a secret and must not offer options"
                ));
            }
            let secret_name = question
                .secret_name
                .as_deref()
                .ok_or_else(|| format!("question {position} secret_name is required"))?;
            if secret_name.trim().is_empty()
                || secret_name.chars().count() > MAX_ASK_USER_SECRET_NAME_CHARS
            {
                return Err(format!(
                    "question {position} secret_name must be between 1 and {MAX_ASK_USER_SECRET_NAME_CHARS} non-whitespace characters"
                ));
            }
            // Nobody should be asked to type a credential without being told
            // what it will be used for.
            if question
                .purpose
                .as_deref()
                .is_none_or(|purpose| purpose.trim().is_empty())
            {
                return Err(format!("question {position} purpose must not be empty"));
            }
            continue;
        }

        if question.secret_name.is_some() || question.purpose.is_some() {
            return Err(format!(
                "question {position} is a choice and must not carry secret_name or purpose"
            ));
        }
        if !(2..=MAX_ASK_USER_OPTIONS).contains(&question.options.len()) {
            return Err(format!(
                "question {position} options must contain between 2 and {MAX_ASK_USER_OPTIONS} items"
            ));
        }

        let mut labels = HashSet::new();
        let mut default_count = 0;
        for option in &question.options {
            if option.label.trim().is_empty() {
                return Err(format!(
                    "question {position} option labels must not be empty"
                ));
            }
            if option.description.trim().is_empty() {
                return Err(format!(
                    "question {position} option descriptions must not be empty"
                ));
            }
            if !labels.insert(option.label.as_str()) {
                return Err(format!(
                    "question {position} option labels must be unique; duplicate {:?}",
                    option.label
                ));
            }
            default_count += usize::from(option.is_default);
        }
        if !question.multi_select && default_count > 1 {
            return Err(format!(
                "question {position} single-select options may mark at most one default"
            ));
        }
    }
    Ok(())
}

pub fn normalize_ask_user_arguments(arguments: &Value) -> Result<Value, String> {
    let human_intent = arguments.get(HUMAN_INTENT_ARGUMENT).cloned();
    let mut contract_arguments = arguments.clone();
    if let Value::Object(object) = &mut contract_arguments {
        object.remove(HUMAN_INTENT_ARGUMENT);
    }
    let mut request: AskUserRequest = serde_json::from_value(contract_arguments)
        .map_err(|error| format!("invalid ask_user arguments: {error}"))?;
    validate_ask_user_request(&request)
        .map_err(|error| format!("invalid ask_user arguments: {error}"))?;

    let mut used_ids: HashSet<String> = request
        .questions
        .iter()
        .filter_map(|question| question.id.clone())
        .collect();
    for (index, question) in request.questions.iter_mut().enumerate() {
        if question.kind == AskUserQuestionKind::Secret {
            // `allow_other` defaults to true, and free text is exactly the trap
            // this kind exists to close: it would carry the typed credential
            // into the tool result. There is no free-text or multi-select path
            // for a secret, so normalize rather than reject a model that left
            // the defaults alone.
            question.allow_other = false;
            question.multi_select = false;
        }
        if question.id.is_some() {
            continue;
        }
        let base = format!("question_{}", index + 1);
        let mut generated = base.clone();
        let mut suffix = 2;
        while used_ids.contains(&generated) {
            generated = format!("{base}_{suffix}");
            suffix += 1;
        }
        used_ids.insert(generated.clone());
        question.id = Some(generated);
    }

    // Stamped last and unconditionally, so a model that supplied its own
    // deadlines does not get to keep them.
    let asked_at = chrono::Utc::now();
    let (nudge_at, expires_at) = deadlines_for(asked_at, request.timeout_seconds);
    request.asked_at = Some(asked_at.to_rfc3339());
    request.nudge_at = Some(nudge_at.to_rfc3339());
    request.expires_at = Some(expires_at.to_rfc3339());

    let mut normalized = serde_json::to_value(request)
        .map_err(|error| format!("failed to normalize ask_user arguments: {error}"))?;
    if let (Some(human_intent), Value::Object(object)) = (human_intent, &mut normalized) {
        object.insert(HUMAN_INTENT_ARGUMENT.to_string(), human_intent);
    }
    Ok(normalized)
}

#[derive(Clone)]
enum AskUserStrategy {
    ClientSide,
    InProcess(Arc<dyn AskUser>),
}

/// Structured questions executed by either a client or an in-process host.
#[derive(Clone)]
pub struct AskUserCapability {
    strategy: AskUserStrategy,
}

impl AskUserCapability {
    /// Execute questions inside the current process with `responder`.
    pub fn new(responder: impl AskUser + 'static) -> Self {
        Self {
            strategy: AskUserStrategy::InProcess(Arc::new(responder)),
        }
    }

    /// Park the turn until a client submits a correlated tool result.
    pub fn client_side() -> Self {
        Self {
            strategy: AskUserStrategy::ClientSide,
        }
    }
}

impl Default for AskUserCapability {
    fn default() -> Self {
        Self::new(DefaultsResponder)
    }
}

impl Capability for AskUserCapability {
    fn id(&self) -> &str {
        ASK_USER_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Ask User"
    }

    fn description(&self) -> &str {
        "Lets an agent ask structured choice questions, or collect a credential, through its host."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Запитати користувача",
            "Дає агенту змогу поставити структуровані запитання з варіантами відповіді через хост.",
        )]
    }

    fn icon(&self) -> Option<&str> {
        Some("message-circle-question")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "`ask_user` handles decisions/preferences. Ask only when blocked; batch questions, and never ask what code or context answers. Put likely options first; timeout uses default/first, and `answered_by` names its source. Do not re-ask a declined question. Never use `ask_user` as a consent gate: destructive, irreversible, or outward-facing actions require `request_approval`, which does not auto-resolve. For A2A `input_required` unanswered, ask the user and relay with `message_task`; never answer for them. For a credential use `kind: \"secret\"` with `secret_name`/`purpose`, alone in the call — never ask for one in prose or an option. It returns a `secret_ref`, never the value, and never auto-resolves.",
        )
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        match &self.strategy {
            AskUserStrategy::ClientSide => vec![ToolDefinition::ClientSide(
                ClientSideTool::new(
                    ASK_USER_TOOL_NAME,
                    "Ask the user 1–4 structured choice questions, or collect one credential, then wait for the answer. Use for decisions and preferences, never for consent to destructive, irreversible, or outward-facing actions.",
                    ask_user_parameters_schema(),
                )
                .with_display_name("Ask User")
                .with_category("Core")
                .with_deferrable(DeferrablePolicy::Never)
                .with_hints(ask_user_tool_hints()),
            )],
            AskUserStrategy::InProcess(_) => self
                .tools()
                .iter()
                .map(|tool| {
                    let mut definition = tool.to_definition();
                    if let ToolDefinition::Builtin(tool) = &mut definition {
                        tool.category = Some("Core".to_string());
                    }
                    definition
                })
                .collect(),
        }
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        match &self.strategy {
            AskUserStrategy::ClientSide => vec![],
            AskUserStrategy::InProcess(responder) => vec![Box::new(AskUserTool {
                responder: responder.clone(),
            })],
        }
    }

    fn finalized_tool_calls_hook(
        &self,
        _config: &Value,
    ) -> Option<Arc<dyn FinalizedToolCallsHook>> {
        Some(Arc::new(AskUserFinalizedToolCallsHook))
    }
}

fn ask_user_tool_hints() -> ToolHints {
    ToolHints::default()
        .with_readonly(true)
        .with_destructive(false)
        .with_open_world(false)
}

struct AskUserTool {
    responder: Arc<dyn AskUser>,
}

#[async_trait]
impl Tool for AskUserTool {
    fn name(&self) -> &str {
        ASK_USER_TOOL_NAME
    }

    fn display_name(&self) -> Option<&str> {
        Some("Ask User")
    }

    fn description(&self) -> &str {
        "Ask the user 1–4 structured choice questions, or collect one credential. Use for decisions and preferences, never for consent to destructive, irreversible, or outward-facing actions."
    }

    fn parameters_schema(&self) -> Value {
        ask_user_parameters_schema()
    }

    fn hints(&self) -> ToolHints {
        ask_user_tool_hints()
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        DeferrablePolicy::Never
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let normalized = match normalize_ask_user_arguments(&arguments) {
            Ok(arguments) => arguments,
            Err(error) => return ToolExecutionResult::tool_error(error),
        };
        let mut contract_arguments = normalized;
        if let Value::Object(object) = &mut contract_arguments {
            object.remove(HUMAN_INTENT_ARGUMENT);
        }
        let request = match serde_json::from_value::<AskUserRequest>(contract_arguments) {
            Ok(request) => request,
            Err(error) => {
                return ToolExecutionResult::internal_error_msg(format!(
                    "normalized ask_user arguments were invalid: {error}"
                ));
            }
        };
        match serde_json::to_value(self.responder.ask(&request.questions).await) {
            Ok(outcome) => ToolExecutionResult::success(outcome),
            Err(error) => ToolExecutionResult::internal_error_msg(format!(
                "ask_user responder returned an invalid outcome: {error}"
            )),
        }
    }
}
struct AskUserFinalizedToolCallsHook;

#[async_trait]
impl FinalizedToolCallsHook for AskUserFinalizedToolCallsHook {
    async fn apply(&self, _context: &FinalizedToolCallsContext<'_>, calls: &mut [ToolCall]) {
        let _ = normalize_ask_user_calls(calls);
    }

    async fn apply_with_rejections(
        &self,
        _context: &FinalizedToolCallsContext<'_>,
        calls: &mut [ToolCall],
    ) -> Vec<FinalizedToolCallRejection> {
        normalize_ask_user_calls(calls)
    }
}

fn normalize_ask_user_calls(calls: &mut [ToolCall]) -> Vec<FinalizedToolCallRejection> {
    let mut rejections = Vec::new();
    for call in calls {
        if call.name != ASK_USER_TOOL_NAME {
            continue;
        }
        match normalize_ask_user_arguments(&call.arguments) {
            Ok(arguments) => call.arguments = arguments,
            Err(error) => rejections.push(FinalizedToolCallRejection {
                tool_call_id: call.id.clone(),
                error,
            }),
        }
    }
    rejections
}
fn ask_user_parameters_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_ASK_USER_QUESTIONS,
                "items": {
                    "type": "object",
                    "properties": {
                        "kind": {
                            "type": "string",
                            "enum": ["choice", "secret"],
                            "default": "choice",
                            "description": "`choice` offers options. `secret` collects one credential and must be the only question in the call; its answer returns a reference, never the value."
                        },
                        "id": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Stable answer correlation key. Generated when omitted."
                        },
                        "header": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": MAX_ASK_USER_HEADER_CHARS,
                            "description": "Short chip label."
                        },
                        "question": {"type": "string", "minLength": 1},
                        "multi_select": {"type": "boolean", "default": false},
                        "allow_other": {
                            "type": "boolean",
                            "default": true,
                            "description": "Allow a free-text answer."
                        },
                        "secret_name": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": MAX_ASK_USER_SECRET_NAME_CHARS,
                            "description": "Required on a `secret` question: the session-secret name to store the credential under. Tools resolve it by this name."
                        },
                        "purpose": {
                            "type": "string",
                            "minLength": 1,
                            "description": "Required on a `secret` question: what the credential will be used for."
                        },
                        "options": {
                            "type": "array",
                            "minItems": 0,
                            "maxItems": MAX_ASK_USER_OPTIONS,
                            "description": "Required on a `choice` question, which offers 2 to 6. A `secret` question offers none.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "label": {"type": "string", "minLength": 1},
                                    "description": {"type": "string", "minLength": 1},
                                    "default": {"type": "boolean", "default": false}
                                },
                                "required": ["label", "description"],
                                "additionalProperties": false
                            }
                        }
                    },
                    "required": ["header", "question"],
                    "additionalProperties": false
                }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 1,
                "maximum": DEFAULT_ASK_USER_TIMEOUT_SECONDS,
                "default": DEFAULT_ASK_USER_TIMEOUT_SECONDS,
                "description": "How long the client waits before applying a default. May shorten but not exceed the platform ceiling."
            },
            // Declared because the schema validates the *normalized* call, which
            // carries the deadlines normalization stamped on it (EVE-1056), and
            // `additionalProperties: false` would otherwise reject it. They are
            // read-only: anything supplied here is discarded and replaced.
            "asked_at": {
                "type": "string",
                "format": "date-time",
                "readOnly": true,
                "description": "Set by the server. When the question was asked; ignored if supplied."
            },
            "nudge_at": {
                "type": "string",
                "format": "date-time",
                "readOnly": true,
                "description": "Set by the server. When the surface starts warning time is short; ignored if supplied."
            },
            "expires_at": {
                "type": "string",
                "format": "date-time",
                "readOnly": true,
                "description": "Set by the server. When the declared defaults are applied; ignored if supplied."
            }
        },
        "required": ["questions"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn option(label: &str, is_default: bool) -> Value {
        json!({
            "label": label,
            "description": format!("{label} description"),
            "default": is_default
        })
    }

    fn question(options: Vec<Value>) -> Value {
        json!({
            "header": "Target",
            "question": "Which environment should I use?",
            "options": options
        })
    }

    fn request(question: Value) -> Value {
        json!({"questions": [question]})
    }

    fn parse_stamp(normalized: &Value, field: &str) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::parse_from_rfc3339(normalized[field].as_str().unwrap())
            .unwrap()
            .with_timezone(&chrono::Utc)
    }

    /// The shipped card derived the nudge as `expires_at - 60s`, which at the
    /// default timeout is the intended four-minute mark. Emitting the deadline
    /// server-side must not move it.
    #[test]
    fn the_default_timeout_keeps_its_four_minute_nudge() {
        let asked_at = chrono::Utc::now();
        let (nudge_at, expires_at) = deadlines_for(asked_at, DEFAULT_ASK_USER_TIMEOUT_SECONDS);

        assert_eq!(expires_at - asked_at, chrono::Duration::seconds(300));
        assert_eq!(expires_at - nudge_at, chrono::Duration::seconds(60));
        assert_eq!(nudge_at - asked_at, chrono::Duration::seconds(240));
    }

    /// A fixed 60s lead on a short window would put the nudge at or before the
    /// moment the question was asked, so the card would open already warning.
    #[test]
    fn a_short_timeout_scales_the_nudge_instead_of_preceding_the_question() {
        for timeout_seconds in [1, 2, 5, 10, 60, 120, 299] {
            let asked_at = chrono::Utc::now();
            let (nudge_at, expires_at) = deadlines_for(asked_at, timeout_seconds);

            assert!(
                nudge_at >= asked_at,
                "{timeout_seconds}s nudged before the question was asked"
            );
            assert!(
                nudge_at < expires_at,
                "{timeout_seconds}s left no countdown at all"
            );
        }
    }

    #[test]
    fn normalization_stamps_the_deadlines_the_server_will_act_on() {
        let before = chrono::Utc::now();
        let normalized = normalize_ask_user_arguments(&json!({
            "questions": [question(vec![option("Staging", false), option("Prod", false)])],
            "timeout_seconds": 120,
        }))
        .unwrap();
        let after = chrono::Utc::now();

        let asked_at = parse_stamp(&normalized, "asked_at");
        let nudge_at = parse_stamp(&normalized, "nudge_at");
        let expires_at = parse_stamp(&normalized, "expires_at");

        assert!(asked_at >= before && asked_at <= after);
        assert_eq!(expires_at - asked_at, chrono::Duration::seconds(120));
        assert!(nudge_at > asked_at && nudge_at < expires_at);
    }

    /// The sweep resolves the call at `expires_at`, so a model that sets its
    /// own would be choosing when it stops waiting for a human.
    #[test]
    fn a_model_supplied_deadline_is_overwritten() {
        let normalized = normalize_ask_user_arguments(&json!({
            "questions": [question(vec![option("Staging", false), option("Prod", false)])],
            "timeout_seconds": 60,
            "asked_at": "2000-01-01T00:00:00Z",
            "nudge_at": "2000-01-01T00:00:00Z",
            "expires_at": "2099-01-01T00:00:00Z",
        }))
        .unwrap();

        let asked_at = parse_stamp(&normalized, "asked_at");
        let expires_at = parse_stamp(&normalized, "expires_at");

        assert!(asked_at.year() > 2000, "asked_at kept the model's value");
        assert_eq!(expires_at - asked_at, chrono::Duration::seconds(60));
    }

    /// A normalized call has to survive a round trip, because that is exactly
    /// what every reader of the persisted event does. `deny_unknown_fields`
    /// makes this the test that catches a stamped field nobody declared.
    #[test]
    fn a_normalized_call_deserializes_again() {
        let normalized = normalize_ask_user_arguments(&request(question(vec![
            option("Staging", false),
            option("Prod", false),
        ])))
        .unwrap();

        let parsed: AskUserRequest = serde_json::from_value(normalized).unwrap();
        assert!(parsed.expires_at.is_some());
    }

    /// The parameters schema is `additionalProperties: false` and validates the
    /// *normalized* call, not just what the model wrote. A field stamped by
    /// normalization but not declared there gets the whole call rejected before
    /// the tool ever runs, which is silent from the model's side.
    #[test]
    fn every_normalized_field_is_declared_in_the_schema() {
        let normalized = normalize_ask_user_arguments(&request(question(vec![
            option("Staging", false),
            option("Prod", false),
        ])))
        .unwrap();
        let schema = ask_user_parameters_schema();
        let declared = schema["properties"].as_object().unwrap();

        assert_eq!(schema["additionalProperties"], json!(false));
        for field in normalized.as_object().unwrap().keys() {
            assert!(
                declared.contains_key(field),
                "normalization stamps {field:?}, which the schema would reject"
            );
        }
        for field in ["asked_at", "nudge_at", "expires_at"] {
            assert_eq!(
                declared[field]["readOnly"],
                json!(true),
                "{field} is server-owned and must say so"
            );
        }
    }

    #[test]
    fn declared_defaults_prefer_the_declared_option_then_the_first() {
        let normalized = normalize_ask_user_arguments(&json!({
            "questions": [
                question(vec![option("Staging", false), option("Prod", true)]),
                question(vec![option("Alpha", false), option("Beta", false)]),
            ]
        }))
        .unwrap();
        let request: AskUserRequest = serde_json::from_value(normalized).unwrap();

        let answers = declared_defaults(&request.questions);

        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].selected, vec!["Prod".to_string()]);
        assert_eq!(answers[1].selected, vec!["Alpha".to_string()]);
        // A default answer never carries free text or a credential handle.
        assert!(answers.iter().all(|answer| answer.other_text.is_none()));
        assert!(answers.iter().all(|answer| answer.secret_ref.is_none()));
    }

    #[test]
    fn definition_is_an_undeferrable_read_only_client_tool() {
        let capability = AskUserCapability::client_side();
        assert_eq!(capability.category(), Some("Core"));
        assert!(capability.tools().is_empty());
        let definitions = capability.tool_definitions();
        let [ToolDefinition::ClientSide(definition)] = definitions.as_slice() else {
            panic!("ask_user must be a client-side tool");
        };
        assert_eq!(definition.name, ASK_USER_TOOL_NAME);
        assert_eq!(definition.display_name.as_deref(), Some("Ask User"));
        assert_eq!(definition.category.as_deref(), Some("Core"));
        assert_eq!(definition.deferrable, DeferrablePolicy::Never);
        assert_eq!(definition.hints.readonly, Some(true));
        assert_eq!(definition.hints.destructive, Some(false));
    }

    #[test]
    fn definition_schema_carries_the_contract_limits() {
        let definition = AskUserCapability::client_side()
            .tool_definitions()
            .remove(0);
        let schema = definition.parameters();
        let questions = &schema["properties"]["questions"];
        assert_eq!(questions["minItems"], 1);
        assert_eq!(questions["maxItems"], MAX_ASK_USER_QUESTIONS);
        assert_eq!(
            questions["items"]["properties"]["kind"]["enum"],
            json!(["choice", "secret"])
        );
        assert_eq!(
            questions["items"]["properties"]["header"]["maxLength"],
            MAX_ASK_USER_HEADER_CHARS
        );
        // A `secret` question offers none, so the 2-option floor for a choice
        // is enforced in validation rather than in the shared schema.
        assert_eq!(questions["items"]["properties"]["options"]["minItems"], 0);
        assert_eq!(
            questions["items"]["properties"]["secret_name"]["maxLength"],
            MAX_ASK_USER_SECRET_NAME_CHARS
        );
        assert_eq!(
            questions["items"]["properties"]["options"]["maxItems"],
            MAX_ASK_USER_OPTIONS
        );
        assert_eq!(
            schema["properties"]["timeout_seconds"]["maximum"],
            DEFAULT_ASK_USER_TIMEOUT_SECONDS
        );
    }

    #[test]
    fn validation_accepts_the_confirmed_default_rules() {
        for value in [
            request(question(vec![
                option("Staging", false),
                option("Prod", false),
            ])),
            request(question(vec![
                option("Staging", true),
                option("Prod", false),
            ])),
            request(json!({
                "header": "Targets",
                "question": "Which environments should I use?",
                "multi_select": true,
                "options": [option("Staging", true), option("Prod", true)]
            })),
        ] {
            assert!(normalize_ask_user_arguments(&value).is_ok(), "{value}");
        }
    }

    #[test]
    fn validation_rejects_contract_violations() {
        let valid = question(vec![option("Staging", false), option("Prod", false)]);
        let cases = [
            json!({"questions": []}),
            json!({"questions": [valid.clone(), valid.clone(), valid.clone(), valid.clone(), valid.clone()]}),
            request(
                json!({"header": "12345678901234567", "question": "Too long?", "options": [option("Yes", false), option("No", false)]}),
            ),
            request(question(vec![option("Only", false)])),
            request(question(vec![option("Yes", true), option("No", true)])),
            // A secret that offers options, has no name, has no purpose, or
            // shares the call with another question.
            request(
                json!({"kind": "secret", "header": "Token", "question": "What is it?", "secret_name": "TOKEN", "purpose": "Why", "options": [option("One", false), option("Two", false)]}),
            ),
            request(
                json!({"kind": "secret", "header": "Token", "question": "What is it?", "purpose": "Why"}),
            ),
            request(
                json!({"kind": "secret", "header": "Token", "question": "What is it?", "secret_name": "TOKEN"}),
            ),
            request(
                json!({"kind": "secret", "header": "Token", "question": "What is it?", "secret_name": "  ", "purpose": "Why"}),
            ),
            json!({"questions": [
                {"kind": "secret", "header": "Token", "question": "What is it?", "secret_name": "TOKEN", "purpose": "Why"},
                valid.clone()
            ]}),
            // A choice question may not carry the secret fields.
            request(
                json!({"header": "Target", "question": "Which?", "secret_name": "TOKEN", "options": [option("One", false), option("Two", false)]}),
            ),
            json!({"questions": [valid.clone()], "timeout_seconds": 301}),
            json!({"questions": [valid.clone()], "unexpected": true}),
            request(question(vec![option("Same", false), option("Same", false)])),
        ];
        for value in cases {
            assert!(normalize_ask_user_arguments(&value).is_err(), "{value}");
        }
    }

    #[test]
    fn normalization_generates_ids_and_materializes_defaults() {
        let value = json!({
            "human_intent": "Asking where to deploy",
            "questions": [
                {"id": "question_2", "header": "First", "question": "First?", "options": [option("A", false), option("B", false)]},
                {"header": "Second", "question": "Second?", "options": [option("C", false), option("D", false)]}
            ]
        });
        let normalized = normalize_ask_user_arguments(&value).unwrap();
        assert_eq!(normalized["questions"][0]["id"], "question_2");
        assert_eq!(normalized["questions"][1]["id"], "question_2_2");
        assert_eq!(normalized["questions"][1]["kind"], "choice");
        assert_eq!(normalized["questions"][1]["allow_other"], true);
        assert_eq!(
            normalized["timeout_seconds"],
            DEFAULT_ASK_USER_TIMEOUT_SECONDS
        );
        assert_eq!(normalized["human_intent"], "Asking where to deploy");
    }

    #[test]
    fn result_contract_round_trips_all_outcomes() {
        for status in [
            AskUserStatus::Answered,
            AskUserStatus::Declined,
            AskUserStatus::Cancelled,
            AskUserStatus::TimedOut,
        ] {
            let result = AskUserResult {
                status,
                answered_by: AskUserAnsweredBy::User,
                answers: vec![AskUserAnswer {
                    id: "target".to_string(),
                    selected: vec!["Staging".to_string()],
                    other_text: None,
                    secret_ref: None,
                }],
            };
            let encoded = serde_json::to_value(&result).unwrap();
            assert_eq!(
                serde_json::from_value::<AskUserResult>(encoded).unwrap(),
                result
            );
        }
    }

    #[test]
    fn prompt_and_localization_preserve_the_safety_boundary() {
        let capability = AskUserCapability::client_side();
        let prompt = capability.system_prompt_addition().unwrap();
        assert!(prompt.contains("request_approval"));
        assert!(prompt.contains("Never use `ask_user` as a consent gate"));
        assert!(prompt.contains("Do not re-ask a declined question"));
        assert!(prompt.contains("Ask only when blocked"));
        assert!(prompt.contains("never ask what code or context answers"));
        assert!(prompt.contains("`input_required`"));
        assert!(prompt.contains("`message_task`"));
        assert!(prompt.contains("never answer for them"));
        assert_eq!(
            capability.localized_name(Some("uk-UA")),
            "Запитати користувача"
        );
    }

    #[tokio::test]
    async fn defaults_responder_returns_declared_defaults_without_waiting() {
        let capability = AskUserCapability::default();
        let tools = capability.tools();
        let [tool] = tools.as_slice() else {
            panic!("default ask_user strategy must contribute one tool");
        };

        let ToolExecutionResult::Success(result) = tool
            .execute(json!({
                "questions": [
                    {
                        "header": "Target",
                        "question": "Where should I deploy?",
                        "options": [option("Staging", true), option("Production", false)]
                    },
                    {
                        "header": "Regions",
                        "question": "Which regions?",
                        "multi_select": true,
                        "options": [option("US", true), option("EU", true)]
                    },
                    {
                        "header": "Format",
                        "question": "Which format?",
                        "options": [option("JSON", false), option("YAML", false)]
                    }
                ]
            }))
            .await
        else {
            panic!("default responder must return a successful tool result");
        };
        let outcome: AskUserResult = serde_json::from_value(result).unwrap();

        assert_eq!(outcome.status, AskUserStatus::Answered);
        assert_eq!(outcome.answered_by, AskUserAnsweredBy::Unattended);
        assert_eq!(
            outcome.answers,
            vec![
                AskUserAnswer {
                    id: "question_1".to_string(),
                    selected: vec!["Staging".to_string()],
                    other_text: None,
                    secret_ref: None,
                },
                AskUserAnswer {
                    id: "question_2".to_string(),
                    selected: vec!["US".to_string(), "EU".to_string()],
                    other_text: None,
                    secret_ref: None,
                },
                AskUserAnswer {
                    id: "question_3".to_string(),
                    selected: vec!["JSON".to_string()],
                    other_text: None,
                    secret_ref: None,
                },
            ]
        );
    }

    /// EVE-1058: a credential has no default, so nothing answers for the person.
    #[tokio::test]
    async fn a_secret_question_never_auto_resolves() {
        let capability = AskUserCapability::default();
        let tools = capability.tools();
        let [tool] = tools.as_slice() else {
            panic!("default ask_user strategy must contribute one tool");
        };

        let arguments = json!({
            "questions": [{
                "kind": "secret",
                "header": "Stripe key",
                "question": "Which Stripe restricted key should I use?",
                "secret_name": "STRIPE_API_KEY",
                "purpose": "Read-only charge lookups."
            }]
        });
        let ToolExecutionResult::Success(result) = tool.execute(arguments).await else {
            panic!("the responder must return a successful tool result");
        };
        let outcome: AskUserResult = serde_json::from_value(result).unwrap();

        assert_eq!(outcome.status, AskUserStatus::Declined);
        assert_eq!(outcome.answered_by, AskUserAnsweredBy::Unattended);
        assert!(outcome.answers.is_empty());
    }

    #[test]
    fn a_secret_question_has_no_free_text_or_multi_select_path() {
        let normalized = normalize_ask_user_arguments(&json!({
            "questions": [{
                "kind": "secret",
                "header": "Stripe key",
                "question": "Which key?",
                "multi_select": true,
                "allow_other": true,
                "secret_name": "STRIPE_API_KEY",
                "purpose": "Read-only charge lookups."
            }]
        }))
        .expect("a secret question with the defaults left alone is accepted");

        // Free text is the trap this kind exists to close: it would carry the
        // typed credential into the tool result.
        assert_eq!(normalized["questions"][0]["allow_other"], false);
        assert_eq!(normalized["questions"][0]["multi_select"], false);
        assert_eq!(normalized["questions"][0]["id"], "question_1");
        assert_eq!(normalized["questions"][0]["kind"], "secret");
    }

    #[test]
    fn a_secret_answer_has_no_value_field_to_put_a_credential_in() {
        let encoded = serde_json::to_value(AskUserAnswer {
            id: "stripe_key".to_string(),
            selected: Vec::new(),
            other_text: None,
            secret_ref: Some(session_secret_ref("STRIPE_API_KEY")),
        })
        .unwrap();
        assert_eq!(encoded["secret_ref"], "session:STRIPE_API_KEY");
        assert!(encoded.get("value").is_none());
        // `deny_unknown_fields` means a client cannot add one either.
        assert!(
            serde_json::from_value::<AskUserAnswer>(json!({
                "id": "stripe_key", "value": "rk_live_verysecret"
            }))
            .is_err()
        );
    }

    #[test]
    fn prompt_guidance_points_at_the_secret_kind_instead_of_prose() {
        let capability = AskUserCapability::client_side();
        let prompt = capability.system_prompt_addition().unwrap();
        assert!(prompt.contains(r#"kind: "secret""#), "{prompt}");
        assert!(prompt.contains("never ask for one in prose or an option"));
        assert!(prompt.contains("never the value"));
        assert!(prompt.contains("never auto-resolves"));
    }

    /// EVE-1057: `DefaultsResponder` and `unattended_ask_user_result` answer an
    /// unanswerable question set the same way.
    ///
    /// Two implementations exist because the engine must be able to resolve one
    /// without depending on this crate, and the contract types live here. This
    /// is what stops them drifting: a model that marked an option `default`
    /// would otherwise get that option from one path and the first option from
    /// the other, depending only on which surface could not be asked.
    #[tokio::test]
    async fn defaults_responder_matches_unattended_result() {
        let cases = vec![
            // A declared default, not first in the list.
            serde_json::json!({"questions": [{
                "kind": "choice", "id": "target", "header": "Target",
                "question": "Where?", "multi_select": false, "allow_other": true,
                "options": [
                    {"label": "Staging", "description": "Safe."},
                    {"label": "Production", "description": "Live.", "default": true}
                ]
            }]}),
            // No default at all: the first option wins.
            serde_json::json!({"questions": [{
                "kind": "choice", "id": "target", "header": "Target",
                "question": "Where?", "multi_select": false, "allow_other": true,
                "options": [
                    {"label": "Staging", "description": "Safe."},
                    {"label": "Production", "description": "Live."}
                ]
            }]}),
            // Multi-select with several defaults: all of them are taken.
            serde_json::json!({"questions": [{
                "kind": "choice", "id": "checks", "header": "Checks",
                "question": "Which?", "multi_select": true, "allow_other": false,
                "options": [
                    {"label": "Lint", "description": "Fast.", "default": true},
                    {"label": "Tests", "description": "Slow.", "default": true},
                    {"label": "Docs", "description": "Rare."}
                ]
            }]}),
            // A secret question: neither path may invent a credential.
            serde_json::json!({"questions": [{
                "kind": "secret", "id": "stripe_key", "header": "Stripe key",
                "question": "Which Stripe restricted key should I use?",
                "multi_select": false, "allow_other": false, "options": [],
                "secret_name": "STRIPE_API_KEY",
                "purpose": "Read-only charge lookups."
            }]}),
            // Two questions at once.
            serde_json::json!({"questions": [
                {"kind": "choice", "id": "a", "header": "A", "question": "A?",
                 "multi_select": false, "allow_other": true,
                 "options": [{"label": "A1", "description": "x", "default": true},
                             {"label": "A2", "description": "y"}]},
                {"kind": "choice", "id": "b", "header": "B", "question": "B?",
                 "multi_select": false, "allow_other": true,
                 "options": [{"label": "B1", "description": "x"},
                             {"label": "B2", "description": "y"}]}
            ]}),
        ];

        for arguments in cases {
            let request: AskUserRequest =
                serde_json::from_value(arguments.clone()).expect("fixture parses");
            let typed = DefaultsResponder.ask(&request.questions).await;
            let typed_json = serde_json::to_value(&typed).expect("serialises");
            let engine_json = crate::tool_types::unattended_ask_user_result(&arguments);
            assert_eq!(engine_json, typed_json, "diverged on {arguments}");
        }
    }
}
