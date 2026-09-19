use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::{FinalizedToolCallsContext, FinalizedToolCallsHook};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::capabilities::{Capability, CapabilityLocalization};
use crate::tool_types::{
    ClientSideTool, DeferrablePolicy, HUMAN_INTENT_ARGUMENT, ToolCall, ToolDefinition, ToolHints,
};

pub const ASK_USER_CAPABILITY_ID: &str = "ask_user";
pub const ASK_USER_TOOL_NAME: &str = "ask_user";
pub const DEFAULT_ASK_USER_TIMEOUT_SECONDS: u64 = 300;
pub const MAX_ASK_USER_QUESTIONS: usize = 4;
pub const MAX_ASK_USER_OPTIONS: usize = 6;
pub const MAX_ASK_USER_HEADER_CHARS: usize = 16;

fn default_timeout_seconds() -> u64 {
    DEFAULT_ASK_USER_TIMEOUT_SECONDS
}

fn default_allow_other() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AskUserQuestionKind {
    #[default]
    Choice,
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
    pub options: Vec<AskUserOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserRequest {
    pub questions: Vec<AskUserQuestion>,
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
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
    pub selected: Vec<String>,
    pub other_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskUserResult {
    pub status: AskUserStatus,
    pub answered_by: AskUserAnsweredBy,
    pub answers: Vec<AskUserAnswer>,
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
    validate_ask_user_request(&request)?;

    let mut used_ids: HashSet<String> = request
        .questions
        .iter()
        .filter_map(|question| question.id.clone())
        .collect();
    for (index, question) in request.questions.iter_mut().enumerate() {
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

    let mut normalized = serde_json::to_value(request)
        .map_err(|error| format!("failed to normalize ask_user arguments: {error}"))?;
    if let (Some(human_intent), Value::Object(object)) = (human_intent, &mut normalized) {
        object.insert(HUMAN_INTENT_ARGUMENT.to_string(), human_intent);
    }
    Ok(normalized)
}

pub struct AskUserCapability;

impl Capability for AskUserCapability {
    fn id(&self) -> &str {
        ASK_USER_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Ask User"
    }

    fn description(&self) -> &str {
        "Lets an agent ask structured choice questions and pause until the client submits an answer."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Запитати користувача",
            "Дає агенту змогу поставити структуровані запитання з варіантами відповіді та призупинити роботу, доки клієнт не надішле відповідь.",
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
            "Use `ask_user` for decisions and preferences, batching related questions into one pause. Order options most-applicable-first; a timeout uses the marked default or the first option, and `answered_by` says whether a person answered. Do not re-ask a declined question. Never use `ask_user` as a consent gate: permission for destructive, irreversible, or outward-facing actions requires `request_approval`, which does not auto-resolve.",
        )
    }

    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition::ClientSide(
            ClientSideTool::new(
                ASK_USER_TOOL_NAME,
                "Ask the user 1–4 structured choice questions, then wait for the answer. Use for decisions and preferences, never for consent to destructive, irreversible, or outward-facing actions.",
                ask_user_parameters_schema(),
            )
            .with_display_name("Ask User")
            .with_category("Core")
            .with_deferrable(DeferrablePolicy::Never)
            .with_hints(
                ToolHints::default()
                    .with_readonly(true)
                    .with_destructive(false)
                    .with_open_world(false),
            ),
        )]
    }

    fn finalized_tool_calls_hook(
        &self,
        _config: &Value,
    ) -> Option<Arc<dyn FinalizedToolCallsHook>> {
        Some(Arc::new(AskUserFinalizedToolCallsHook))
    }
}

struct AskUserFinalizedToolCallsHook;

#[async_trait]
impl FinalizedToolCallsHook for AskUserFinalizedToolCallsHook {
    async fn apply(&self, _context: &FinalizedToolCallsContext<'_>, calls: &mut [ToolCall]) {
        for call in calls {
            if call.name == ASK_USER_TOOL_NAME
                && let Ok(arguments) = normalize_ask_user_arguments(&call.arguments)
            {
                call.arguments = arguments;
            }
        }
    }
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
                        "kind": {"type": "string", "enum": ["choice"], "default": "choice"},
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
                        "options": {
                            "type": "array",
                            "minItems": 2,
                            "maxItems": MAX_ASK_USER_OPTIONS,
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
                    "required": ["header", "question", "options"],
                    "additionalProperties": false
                }
            },
            "timeout_seconds": {
                "type": "integer",
                "minimum": 1,
                "maximum": DEFAULT_ASK_USER_TIMEOUT_SECONDS,
                "default": DEFAULT_ASK_USER_TIMEOUT_SECONDS,
                "description": "How long the client waits before applying a default. May shorten but not exceed the platform ceiling."
            }
        },
        "required": ["questions"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn definition_is_an_undeferrable_read_only_client_tool() {
        let capability = AskUserCapability;
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
        let definition = AskUserCapability.tool_definitions().remove(0);
        let schema = definition.parameters();
        let questions = &schema["properties"]["questions"];
        assert_eq!(questions["minItems"], 1);
        assert_eq!(questions["maxItems"], MAX_ASK_USER_QUESTIONS);
        assert_eq!(
            questions["items"]["properties"]["kind"]["enum"],
            json!(["choice"])
        );
        assert_eq!(
            questions["items"]["properties"]["header"]["maxLength"],
            MAX_ASK_USER_HEADER_CHARS
        );
        assert_eq!(questions["items"]["properties"]["options"]["minItems"], 2);
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
            request(
                json!({"kind": "secret", "header": "Token", "question": "What is it?", "options": [option("One", false), option("Two", false)]}),
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
        let capability = AskUserCapability;
        let prompt = capability.system_prompt_addition().unwrap();
        assert!(prompt.contains("request_approval"));
        assert!(prompt.contains("Never use `ask_user` as a consent gate"));
        assert!(prompt.contains("Do not re-ask a declined question"));
        assert_eq!(
            capability.localized_name(Some("uk-UA")),
            "Запитати користувача"
        );
    }
}
