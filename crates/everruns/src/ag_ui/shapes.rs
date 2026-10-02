//! Interrupt and answer shapes: how `ask_user` questions and tool approvals
//! become AG-UI interrupts, and how resume entries become their answers.

use serde::Deserialize;
use serde_json::{Value, json};

use super::*;

/// The interrupt an `ask_user` question set becomes.
///
/// Its id is the tool call id. A set with a `secret` question gets
/// [`SECRET_REASON`] and no schema; any other gets [`ASK_USER_REASON`], the
/// questions as prose in `message`, the questions themselves under
/// `metadata.everruns.questions`, and a `responseSchema` for the answer.
///
/// ```
/// use everruns::ag_ui::{ASK_USER_REASON, question_interrupt};
/// use everruns::ask_user::Question;
///
/// let questions: Vec<Question> = serde_json::from_value(serde_json::json!([{
///     "id": "target",
///     "header": "Target",
///     "question": "Where should I deploy?",
///     "options": [
///         { "label": "Staging", "description": "Safe" },
///         { "label": "Production", "description": "Live" },
///     ],
/// }]))?;
/// let interrupt = question_interrupt("call_1", &questions);
/// assert_eq!(interrupt.id, "call_1");
/// assert_eq!(interrupt.reason, ASK_USER_REASON);
/// assert!(interrupt.response_schema.is_some());
/// # Ok::<(), serde_json::Error>(())
/// ```
pub fn question_interrupt(tool_call_id: &str, questions: &[Question]) -> Interrupt {
    let secret = questions
        .iter()
        .any(|question| question.kind == QuestionKind::Secret);
    let message = questions
        .iter()
        .map(|question| question.question.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let mut interrupt = Interrupt {
        message: Some(message),
        ..Interrupt::new(
            tool_call_id,
            if secret {
                SECRET_REASON
            } else {
                ASK_USER_REASON
            },
        )
    };
    if !secret {
        interrupt.response_schema = as_object(answer_schema(tool_call_id, questions));
        interrupt.metadata = Some(everruns_metadata(json!({ "questions": questions })));
    }
    interrupt
}

/// The interrupt a tool call waiting on approval becomes.
///
/// Its id and `toolCallId` are the call's id; `metadata.everruns` carries the
/// tool name and arguments so the client can show what it approves.
///
/// ```
/// use everruns::ToolCall;
/// use everruns::ag_ui::{TOOL_APPROVAL_REASON, approval_interrupt};
///
/// let call = ToolCall {
///     id: "call_1".into(),
///     name: "deploy".into(),
///     arguments: serde_json::json!({ "env": "production" }),
/// };
/// let interrupt = approval_interrupt(&call);
/// assert_eq!(interrupt.reason, TOOL_APPROVAL_REASON);
/// assert_eq!(interrupt.tool_call_id.as_deref(), Some("call_1"));
/// ```
pub fn approval_interrupt(call: &ToolCall) -> Interrupt {
    Interrupt {
        message: Some(format!("Allow the agent to run {}?", call.name)),
        tool_call_id: Some(call.id.clone()),
        response_schema: as_object(json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["decision"],
            "properties": {
                "decision": {
                    "enum": ["allow", "allow_always", "reject", "reject_always"],
                    "description": "`*_always` applies to every later call of this tool in the session.",
                },
            },
        })),
        metadata: Some(everruns_metadata(json!({
            "tool": call.name,
            "arguments": call.arguments,
        }))),
        ..Interrupt::new(call.id.clone(), TOOL_APPROVAL_REASON)
    }
}

/// The question-answers body an `ask_user` resume entry carries.
#[derive(Deserialize)]
struct QuestionAnswers {
    #[serde(default)]
    tool_call_id: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    answers: Vec<SubmittedAnswer>,
}

#[derive(Deserialize)]
struct SubmittedAnswer {
    id: String,
    #[serde(default)]
    selected: Vec<String>,
    #[serde(default)]
    other_text: Option<String>,
}

/// The [`Outcome`] a resume entry answers an `ask_user` interrupt with.
///
/// An abandoned entry declines the set. A resolved entry's payload is the
/// question-answers body the interrupt's `responseSchema` describes, checked
/// against the questions asked: every question answered once, only offered
/// options, one selection on a single-select question. A credential is never
/// accepted.
///
/// ```
/// use everruns::ag_ui::{ResumeEntry, ResumeStatus, question_outcome};
/// use everruns::ask_user::{Question, Status};
///
/// let questions: Vec<Question> = serde_json::from_value(serde_json::json!([{
///     "id": "target",
///     "header": "Target",
///     "question": "Where should I deploy?",
///     "options": [
///         { "label": "Staging", "description": "Safe" },
///         { "label": "Production", "description": "Live" },
///     ],
/// }]))?;
/// let entry = ResumeEntry {
///     interrupt_id: "call_1".into(),
///     status: ResumeStatus::Resolved,
///     payload: Some(serde_json::json!({
///         "answers": [{ "id": "target", "selected": ["Staging"] }],
///     })),
///     metadata: None,
/// };
/// let outcome = question_outcome(&entry, "call_1", &questions)?;
/// assert_eq!(outcome.status, Status::Answered);
/// assert_eq!(outcome.answers[0].selected, ["Staging"]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Errors
///
/// [`AgUiError::InvalidInput`] when the payload is missing, malformed, names
/// another question set, or does not answer the questions asked.
pub fn question_outcome(
    entry: &ResumeEntry,
    tool_call_id: &str,
    questions: &[Question],
) -> Result<Outcome, AgUiError> {
    let declined = Outcome {
        status: Status::Declined,
        answered_by: AnsweredBy::User,
        answers: Vec::new(),
    };
    if entry.status == ResumeStatus::Cancelled {
        return Ok(declined);
    }
    if questions
        .iter()
        .any(|question| question.kind == QuestionKind::Secret)
    {
        return Err(invalid(
            "a credential cannot be sent over AG-UI; abandon the interrupt instead",
        ));
    }
    let payload = entry
        .payload
        .clone()
        .ok_or_else(|| invalid("a resolved entry needs a payload"))?;
    let body: QuestionAnswers = serde_json::from_value(payload)
        .map_err(|error| invalid(format!("invalid ask_user answer: {error}")))?;
    if body
        .tool_call_id
        .as_deref()
        .is_some_and(|id| id != tool_call_id)
    {
        return Err(invalid("the answer names a different question set"));
    }
    match body.status.as_deref() {
        None | Some("answered") => {}
        Some("declined") => return Ok(declined),
        Some(other) => return Err(invalid(format!("unknown answer status {other:?}"))),
    }
    let answers: Vec<Answer> = body
        .answers
        .into_iter()
        .map(|answer| Answer {
            id: answer.id,
            selected: answer.selected,
            other_text: answer.other_text,
            secret_ref: None,
        })
        .collect();
    validate_answers(questions, &answers).map_err(AgUiError::InvalidInput)?;
    Ok(Outcome {
        status: Status::Answered,
        answered_by: AnsweredBy::User,
        answers,
    })
}

/// The decision a resume entry answers a tool-approval interrupt with.
///
/// An abandoned entry rejects the call; a resolved one reads
/// `payload.decision`.
///
/// ```
/// use everruns::ag_ui::{ResumeEntry, ResumeStatus, approval_decision};
/// use everruns::approval::ApprovalDecision;
///
/// let entry = ResumeEntry {
///     interrupt_id: "call_1".into(),
///     status: ResumeStatus::Resolved,
///     payload: Some(serde_json::json!({ "decision": "allow" })),
///     metadata: None,
/// };
/// assert_eq!(approval_decision(&entry)?, ApprovalDecision::Allow);
/// # Ok::<(), everruns::ag_ui::AgUiError>(())
/// ```
///
/// # Errors
///
/// [`AgUiError::InvalidInput`] when a resolved entry has no known decision.
pub fn approval_decision(entry: &ResumeEntry) -> Result<ApprovalDecision, AgUiError> {
    if entry.status == ResumeStatus::Cancelled {
        return Ok(ApprovalDecision::Reject);
    }
    let decision = entry
        .payload
        .as_ref()
        .and_then(|payload| payload.get("decision"))
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("a tool approval needs a decision"))?;
    match decision {
        "allow" => Ok(ApprovalDecision::Allow),
        "allow_always" => Ok(ApprovalDecision::AllowAlways),
        "reject" => Ok(ApprovalDecision::Reject),
        "reject_always" => Ok(ApprovalDecision::RejectAlways),
        other => Err(invalid(format!("unknown decision {other:?}"))),
    }
}

/// The JSON Schema of an `ask_user` answer: the same shape the Everruns
/// server advertises for its question-answers request.
fn answer_schema(tool_call_id: &str, questions: &[Question]) -> Value {
    let answers: Vec<Value> = questions
        .iter()
        .map(|question| {
            let mut properties = serde_json::Map::new();
            properties.insert(
                "id".to_string(),
                json!({ "const": question.id.clone().unwrap_or_default() }),
            );
            if question.kind == QuestionKind::Text {
                properties.insert(
                    "other_text".to_string(),
                    json!({ "type": "string", "description": "The free-form answer." }),
                );
                return json!({
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "other_text"],
                    "properties": properties,
                    "description": question.question,
                });
            }
            let labels: Vec<&str> = question
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect();
            let mut selected = json!({ "type": "array", "items": { "enum": labels } });
            if !question.multi_select
                && let Some(object) = selected.as_object_mut()
            {
                object.insert("maxItems".to_string(), json!(1));
            }
            properties.insert("selected".to_string(), selected);
            if question.allow_other {
                properties.insert(
                    "other_text".to_string(),
                    json!({
                        "type": ["string", "null"],
                        "description": "Free text, when none of the options fit.",
                    }),
                );
            }
            json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["id"],
                "properties": properties,
                "description": question.question,
            })
        })
        .collect();
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["answers"],
        "properties": {
            "tool_call_id": { "const": tool_call_id },
            "status": {
                "enum": ["answered", "declined"],
                "default": "answered",
                "description": "`declined` is a finished decision the agent must not re-ask.",
            },
            "answers": {
                "type": "array",
                "minItems": answers.len(),
                "maxItems": answers.len(),
                "description": "One answer per asked question, in any order.",
                "items": { "oneOf": answers },
            },
        },
    })
}

/// Every question answered once, with offered options only, as the Everruns
/// server checks a question-answers request.
fn validate_answers(questions: &[Question], answers: &[Answer]) -> Result<(), String> {
    for answer in answers {
        if !questions
            .iter()
            .any(|question| question.id.as_deref() == Some(answer.id.as_str()))
        {
            return Err(format!("no question with id {:?} was asked", answer.id));
        }
    }
    for question in questions {
        let id = question.id.as_deref().unwrap_or_default();
        let mut matching = answers.iter().filter(|answer| answer.id == id);
        let answer = matching
            .next()
            .ok_or_else(|| format!("question {id:?} was not answered"))?;
        if matching.next().is_some() {
            return Err(format!("question {id:?} was answered more than once"));
        }
        for label in &answer.selected {
            if !question.options.iter().any(|option| &option.label == label) {
                return Err(format!(
                    "question {id:?} was not asked with option {label:?}"
                ));
            }
        }
        let other = answer
            .other_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if other.is_some() && question.kind != QuestionKind::Text && !question.allow_other {
            return Err(format!("question {id:?} does not allow free text"));
        }
        if answer.selected.is_empty() && other.is_none() {
            return Err(format!("question {id:?} has no selection and no free text"));
        }
        if !question.multi_select && answer.selected.len() > 1 {
            return Err(format!("question {id:?} is single-select"));
        }
    }
    Ok(())
}

fn as_object(value: Value) -> Option<serde_json::Map<String, Value>> {
    match value {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Our keys go under `everruns`; `ag-ui` is reserved for the protocol.
fn everruns_metadata(value: Value) -> everruns_ag_ui::Metadata {
    let mut metadata = everruns_ag_ui::Metadata::new();
    metadata.insert("everruns".to_string(), value);
    metadata
}
