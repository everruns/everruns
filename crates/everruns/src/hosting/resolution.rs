//! Shared validation after command deduplication, before any answer is committed.
use super::*;
use everruns_contracts::error::AgentLoopError;
use everruns_core::builtins::ask_user::{
    AskUserAnswer as Answer, AskUserAnsweredBy, AskUserQuestion as Question,
    AskUserQuestionKind as QuestionKind, AskUserRequest, AskUserResult, AskUserStatus,
};

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
    for answer in answers {
        if answer.secret_ref.is_some() {
            return Err("client answers cannot supply secret references".into());
        }
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
        if question.kind == QuestionKind::Secret {
            return Err(format!(
                "question {id:?} asks for a secret, which this client-answer port does not store"
            ));
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

/// Validate the whole pending-result batch before writing any member.
pub fn validate_tool_resolutions(
    results: &[ToolCompletedData],
    pending: &[crate::ToolCall],
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
