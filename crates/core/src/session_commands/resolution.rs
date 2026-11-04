//! Shared validation after command deduplication, before any answer is committed.
use super::*;
use crate::builtins::ask_user::{
    AskUserAnswer as Answer, AskUserAnsweredBy, AskUserQuestion as Question,
    AskUserQuestionKind as QuestionKind, AskUserRequest, AskUserResult, AskUserStatus,
};
use everruns_contracts::error::AgentLoopError;

/// Execution API command for one retained `ask_user` request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QuestionAnswersRequest {
    /// Retain with `turn_id` and `tool_call_id` for a retry after an uncertain reply.
    #[serde(default)]
    pub command_id: Option<uuid::Uuid>,
    /// Exact turn that asked, rather than whichever turn currently waits.
    #[serde(default)]
    pub turn_id: Option<everruns_contracts::typed_id::TurnId>,
    /// Canonical question-set identity; required for stable commands.
    #[serde(default)]
    pub tool_call_id: Option<String>,
    /// Answered or declined; unattended statuses cannot be supplied by a client.
    #[serde(default = "answered")]
    pub status: AskUserStatus,
    /// Structured non-secret answers.
    #[serde(default)]
    pub answers: Vec<Answer>,
}
fn answered() -> AskUserStatus {
    AskUserStatus::Answered
}

/// Validate non-secret client answers against the retained question set.
pub fn validate_question_answers(
    questions: &[Question],
    answers: &[Answer],
) -> std::result::Result<(), String> {
    normalize_question_answers(questions, answers, false).map(|_| ())
}

/// Validate the whole pending-result batch before writing any member.
pub fn validate_tool_resolutions(
    results: &[ToolCompletedData],
    pending: &[ToolCall],
    now: everruns_contracts::rt::UtcTimestamp,
) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    if results.is_empty()
        || results.len() > 256
        || serde_json::to_vec(results)
            .map_err(|error| AgentLoopError::config(error.to_string()))?
            .len()
            > 1024 * 1024
    {
        return Err(AgentLoopError::config("invalid tool result batch"));
    }
    for result in results {
        let call = pending
            .iter()
            .find(|call| call.id == result.tool_call_id && call.name == result.tool_name)
            .ok_or_else(|| AgentLoopError::config("tool result does not match a pending call"))?;
        if !seen.insert(&result.tool_call_id)
            || call.name == everruns_contracts::tool_types::APPROVE_TOOL_CALL_TOOL
        {
            return Err(AgentLoopError::config("invalid or approval tool result"));
        }
        if call.name == "ask_user" {
            let request: AskUserRequest = serde_json::from_value(call.arguments.clone())
                .map_err(|error| AgentLoopError::config(error.to_string()))?;
            if request.expires_at.as_ref().is_none_or(|expires| {
                expires
                    .parse::<everruns_contracts::rt::UtcTimestamp>()
                    .map_or(true, |expires| expires <= now)
            }) {
                return Err(AgentLoopError::config("question request expired"));
            }
            let parts = result
                .result
                .as_ref()
                .ok_or_else(|| AgentLoopError::config("question answer needs an outcome"))?;
            if !result.success || parts.len() != 1 {
                return Err(AgentLoopError::config("invalid question outcome"));
            }
            let text = parts[0].as_text().ok_or_else(|| {
                AgentLoopError::config("question outcome must be structured JSON")
            })?;
            let outcome: AskUserResult = serde_json::from_str(text)
                .map_err(|error| AgentLoopError::config(error.to_string()))?;
            if outcome.answered_by != AskUserAnsweredBy::User
                || !matches!(
                    outcome.status,
                    AskUserStatus::Answered | AskUserStatus::Declined
                )
            {
                return Err(AgentLoopError::config("invalid client question status"));
            }
            if outcome
                .answers
                .iter()
                .any(|answer| answer.secret_ref.is_some())
            {
                return Err(AgentLoopError::config(
                    "client answers cannot supply secret references",
                ));
            }
            if outcome.status == AskUserStatus::Answered {
                validate_question_answers(&request.questions, &outcome.answers)
                    .map_err(AgentLoopError::config)?;
            } else if !outcome.answers.is_empty() {
                return Err(AgentLoopError::config(
                    "declined questions cannot carry answers",
                ));
            }
        }
    }
    Ok(())
}

/// Validate and normalize answers in canonical question order. A host may
/// enable references to already encrypted secrets; it still verifies their
/// existence and any product-specific text policy before committing.
pub fn normalize_question_answers(
    questions: &[Question],
    submitted: &[Answer],
    allow_secret_references: bool,
) -> std::result::Result<Vec<Answer>, String> {
    for answer in submitted {
        if !questions
            .iter()
            .any(|question| question.id.as_deref() == Some(answer.id.as_str()))
        {
            return Err(format!("no question with id {:?} was asked", answer.id));
        }
    }

    let mut validated = Vec::with_capacity(questions.len());
    for question in questions {
        let id = question.id.as_deref().unwrap_or_default();
        let matches: Vec<&Answer> = submitted.iter().filter(|answer| answer.id == id).collect();
        if matches.len() > 1 {
            return Err(format!("question {id:?} was answered more than once"));
        }
        // Every asked question needs an answer. A partial set would reach the
        // model as a question it asked and nobody addressed, which it cannot
        // tell apart from one the person deliberately skipped.
        let answer = matches
            .first()
            .ok_or_else(|| format!("question {id:?} was not answered"))?;

        if question.kind == QuestionKind::Secret {
            if !allow_secret_references {
                return Err(format!(
                    "question {id:?} asks for a secret, which this client-answer port does not store"
                ));
            }
            // THREAT[TM-AGENT-016]: the only thing a secret answer may carry is
            // a reference to a secret that was already stored encrypted. Every
            // value-bearing field is refused here rather than ignored, so a
            // client that put the credential in one gets an error instead of a
            // result that quietly persisted it to the event log.
            if !answer.selected.is_empty() || answer.other_text.is_some() {
                return Err(format!(
                    "question {id:?} is a secret; its answer carries only secret_ref"
                ));
            }
            let expected = crate::builtins::ask_user::session_secret_ref(
                question.secret_name.as_deref().unwrap_or_default(),
            );
            match answer.secret_ref.as_deref() {
                Some(reference) if reference == expected => {}
                Some(_) => {
                    return Err(format!("question {id:?} was not asked for that secret"));
                }
                None => return Err(format!("question {id:?} has no secret_ref")),
            }
            validated.push(Answer {
                id: id.to_string(),
                selected: Vec::new(),
                other_text: None,
                secret_ref: Some(expected),
            });
            continue;
        }
        // A choice answer cannot mint a handle to a secret nobody asked for.
        if answer.secret_ref.is_some() {
            return Err(format!("question {id:?} is not a secret question"));
        }
        if question.kind == QuestionKind::Text && !answer.selected.is_empty() {
            return Err(format!(
                "question {id:?} is text and must not select options"
            ));
        }

        for label in &answer.selected {
            if !question.options.iter().any(|option| &option.label == label) {
                return Err(format!(
                    "question {id:?} was not asked with option {label:?}"
                ));
            }
        }
        if answer.selected.len()
            != answer
                .selected
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
        {
            return Err(format!("question {id:?} selects the same option twice"));
        }

        let other = answer
            .other_text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty());
        if other.is_some() && question.kind != QuestionKind::Text && !question.allow_other {
            return Err(format!("question {id:?} does not allow free text"));
        }

        // An answer has to say something. Free text stands in for a selection,
        // which is the whole point of `allow_other`.
        if answer.selected.is_empty() && other.is_none() {
            return Err(format!("question {id:?} has no selection and no free text"));
        }
        if !question.multi_select && answer.selected.len() > 1 {
            return Err(format!(
                "question {id:?} is single-select and carries {} selections",
                answer.selected.len()
            ));
        }

        validated.push(Answer {
            id: id.to_string(),
            selected: answer.selected.clone(),
            other_text: other.map(str::to_string),
            secret_ref: None,
        });
    }
    Ok(validated)
}
