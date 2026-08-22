// Task failure formatting helpers
// Decision: Persist a single human-readable failure string that keeps both
// task metadata and the full anyhow error chain for durable task surfaces.

use anyhow::Error;
use everruns_provider::error::AgentLoopError;
use everruns_provider::user_facing_error::{
    UserFacingError, UserFacingErrorContext, classify_runtime_error_message,
};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskFailureSummary {
    pub(crate) session_id: Option<String>,
    pub(crate) tool_identifiers: Vec<String>,
    pub(crate) error_chain: String,
    pub(crate) persisted_message: String,
}

pub(crate) fn summarize_task_failure(
    task_id: Uuid,
    workflow_id: Option<Uuid>,
    activity_type: &str,
    attempt: u32,
    max_attempts: Option<u32>,
    input: &Value,
    error: &Error,
) -> TaskFailureSummary {
    let session_id = extract_session_id(input);
    let tool_identifiers = extract_tool_identifiers(input);
    let error_chain = format_error_chain(error);

    let mut metadata = vec![
        format!("activity_type={activity_type}"),
        format!("task_id={task_id}"),
    ];
    if let Some(workflow_id) = workflow_id {
        metadata.push(format!("workflow_id={workflow_id}"));
    }
    if let Some(session_id) = session_id.as_deref() {
        metadata.push(format!("session_id={session_id}"));
    }
    let attempt_value = max_attempts
        .map(|max_attempts| format!("{attempt}/{max_attempts}"))
        .unwrap_or_else(|| attempt.to_string());
    metadata.push(format!("attempt={attempt_value}"));
    if !tool_identifiers.is_empty() {
        metadata.push(format!("tools={}", tool_identifiers.join(",")));
    }

    let persisted_message = format!("{} | error_chain={error_chain}", metadata.join(" "));

    TaskFailureSummary {
        session_id,
        tool_identifiers,
        error_chain,
        persisted_message,
    }
}

pub(crate) fn user_facing_failure(error: &str) -> UserFacingError {
    let error_chain = error.split("error_chain=").nth(1).unwrap_or(error).trim();
    classify_runtime_error_message(error_chain, &UserFacingErrorContext::default())
}

/// Preserve typed retry semantics through anyhow's activity error boundary.
pub(crate) fn is_non_retryable_task_error(error: &Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<AgentLoopError>()
            .is_some_and(AgentLoopError::is_non_retryable)
    })
}

#[cfg(test)]
pub(crate) fn user_facing_failure_message(error: &str) -> String {
    user_facing_failure(error).fallback_message()
}

fn format_error_chain(error: &Error) -> String {
    let mut messages = Vec::new();

    for cause in error.chain() {
        let message = cause.to_string();
        if message.is_empty() {
            continue;
        }
        if messages.last().is_some_and(|last| last == &message) {
            continue;
        }
        messages.push(message);
    }

    if messages.is_empty() {
        error.to_string()
    } else {
        messages.join(": ")
    }
}

fn extract_session_id(input: &Value) -> Option<String> {
    input
        .get("session_id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            session_context(input).and_then(|context| {
                context
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            })
        })
}

fn extract_tool_identifiers(input: &Value) -> Vec<String> {
    let tool_calls = input
        .get("tool_calls")
        .or_else(|| {
            input
                .get("act_input")
                .and_then(|value| value.get("tool_calls"))
        })
        .and_then(Value::as_array);

    let mut identifiers = tool_calls
        .into_iter()
        .flatten()
        .filter_map(|tool_call| {
            let name = tool_call.get("name").and_then(Value::as_str);
            let id = tool_call.get("id").and_then(Value::as_str);

            match (id, name) {
                (Some(id), Some(name)) => Some(format!("{id}:{name}")),
                (None, Some(name)) => Some(name.to_string()),
                (Some(id), None) => Some(id.to_string()),
                (None, None) => None,
            }
        })
        .collect::<Vec<_>>();

    const MAX_TOOL_IDENTIFIERS: usize = 5;
    if identifiers.len() > MAX_TOOL_IDENTIFIERS {
        let remaining = identifiers.len() - MAX_TOOL_IDENTIFIERS;
        identifiers.truncate(MAX_TOOL_IDENTIFIERS);
        identifiers.push(format!("+{remaining} more"));
    }

    identifiers
}

fn session_context(input: &Value) -> Option<&Value> {
    input.get("context").or_else(|| {
        input
            .get("act_input")
            .and_then(|value| value.get("context"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_task_failure_preserves_anyhow_chain_and_act_metadata() {
        let task_id = Uuid::parse_str("019d9861-c0f7-7ab1-967e-ddacc25f0690").unwrap();
        let workflow_id = Some(Uuid::parse_str("019d9861-b17a-71d3-b929-eb0e7ae2dc55").unwrap());
        let input = json!({
            "org_id": 1,
            "act_input": {
                "context": {
                    "session_id": "session_019d9861b17a71d3b929eb0e7ae2dc55",
                    "turn_id": "turn_123",
                    "input_message_id": "msg_123"
                },
                "tool_calls": [
                    {"id": "call_1", "name": "web.search"},
                    {"id": "call_2", "name": "linear.fetch"}
                ]
            }
        });
        let error = anyhow::anyhow!("tool call 1 failed").context("ActAtom execution failed");

        let summary =
            summarize_task_failure(task_id, workflow_id, "act", 2, Some(5), &input, &error);

        assert_eq!(
            summary.error_chain,
            "ActAtom execution failed: tool call 1 failed"
        );
        assert_eq!(
            summary.session_id.as_deref(),
            Some("session_019d9861b17a71d3b929eb0e7ae2dc55")
        );
        assert_eq!(
            summary.tool_identifiers,
            vec!["call_1:web.search", "call_2:linear.fetch"]
        );
        assert!(summary.persisted_message.contains("activity_type=act"));
        assert!(
            summary
                .persisted_message
                .contains(&format!("task_id={task_id}"))
        );
        assert!(
            summary
                .persisted_message
                .contains("workflow_id=019d9861-b17a-71d3-b929-eb0e7ae2dc55")
        );
        assert!(
            summary
                .persisted_message
                .contains("session_id=session_019d9861b17a71d3b929eb0e7ae2dc55")
        );
        assert!(summary.persisted_message.contains("attempt=2/5"));
        assert!(
            summary
                .persisted_message
                .contains("tools=call_1:web.search,call_2:linear.fetch")
        );
        assert!(
            summary
                .persisted_message
                .contains("error_chain=ActAtom execution failed: tool call 1 failed")
        );
    }

    #[test]
    fn summarize_task_failure_reads_unwrapped_act_input_shape() {
        let task_id = Uuid::nil();
        let input = json!({
            "context": {
                "session_id": "session_123"
            },
            "tool_calls": [
                {"name": "commentary"}
            ]
        });
        let error = anyhow::anyhow!("inner").context("outer");

        let summary = summarize_task_failure(task_id, None, "act", 1, Some(3), &input, &error);

        assert_eq!(summary.session_id.as_deref(), Some("session_123"));
        assert_eq!(summary.tool_identifiers, vec!["commentary"]);
        assert!(summary.persisted_message.contains("attempt=1/3"));
        assert!(
            summary
                .persisted_message
                .contains("error_chain=outer: inner")
        );
    }

    #[test]
    fn summarize_task_failure_reads_top_level_durable_turn_session_id() {
        let input = json!({
            "session_id": "session_top_level",
            "context": {
                "session_id": "session_nested"
            }
        });
        let error = anyhow::anyhow!("inner");

        let summary =
            summarize_task_failure(Uuid::nil(), None, "reason", 1, Some(2), &input, &error);

        assert_eq!(summary.session_id.as_deref(), Some("session_top_level"));
        assert!(
            summary
                .persisted_message
                .contains("session_id=session_top_level")
        );
    }

    #[test]
    fn typed_model_configuration_failure_is_non_retryable() {
        let error = anyhow::Error::new(AgentLoopError::model_not_configured())
            .context("ReasonAtom execution failed");

        assert!(is_non_retryable_task_error(&error));
    }

    #[test]
    fn transient_failure_remains_retryable() {
        let error = anyhow::anyhow!("provider temporarily unavailable");

        assert!(!is_non_retryable_task_error(&error));
    }

    #[test]
    fn user_facing_failure_message_extracts_budget_copy_from_persisted_error() {
        let message = user_facing_failure_message(
            "activity_type=reason | error_chain=ReasonAtom execution failed: Budget exhausted. 100.00 tokens spent reached the 100.00 tokens limit. Increase the budget to continue.",
        );

        assert_eq!(
            message,
            "Budget exhausted. 100.00 tokens spent reached the 100.00 tokens limit. Increase the budget to continue."
        );
    }

    #[test]
    fn user_facing_failure_message_recognizes_rate_limit_errors() {
        let message = user_facing_failure_message(
            "activity_type=reason | error_chain=ReasonAtom execution failed: OpenAI API error (429): Rate limit exceeded",
        );

        assert_eq!(
            message,
            "Rate limited by the AI provider. Please wait a moment."
        );
    }

    #[test]
    fn user_facing_failure_message_recognizes_model_unavailable_errors() {
        let message = user_facing_failure_message(
            "activity_type=reason | error_chain=ReasonAtom execution failed: Model not available: gpt-99",
        );

        assert_eq!(
            message,
            "The model `gpt-99` is not available. It may have been removed, renamed, or your API key may not have access to it. Please select a different model."
        );
    }

    #[test]
    fn user_facing_failure_message_recognizes_request_too_large_errors() {
        let message = user_facing_failure_message(
            "activity_type=reason | error_chain=ReasonAtom execution failed: Request too large: OpenAI API error (429): Request too large for gpt-4o",
        );

        assert_eq!(
            message,
            "The conversation has become too long for the model to process. Please start a new session or reduce the context size."
        );
    }

    #[test]
    fn user_facing_failure_message_recognizes_provider_auth_errors() {
        let message = user_facing_failure_message(
            "activity_type=reason | error_chain=ReasonAtom execution failed: OpenAI API error (401): Invalid authentication",
        );

        assert_eq!(
            message,
            "There is a misconfiguration with the AI provider. Please contact support."
        );
    }

    #[test]
    fn user_facing_failure_message_recognizes_provider_5xx_errors() {
        let message = user_facing_failure_message(
            "activity_type=reason | error_chain=ReasonAtom execution failed: OpenAI API error (503): Service unavailable",
        );

        assert_eq!(
            message,
            "The AI provider is experiencing issues. Please try again shortly."
        );
    }

    #[test]
    fn user_facing_failure_message_preserves_soft_limit_copy() {
        // Soft-limit copy is authored downstream (budgeting). The task-error
        // extractor must pass it through verbatim rather than falling back to
        // the generic copy.
        let message = user_facing_failure_message(
            "activity_type=reason | error_chain=ReasonAtom execution failed: Soft limit reached. Continue to keep going past the budget.",
        );

        assert_eq!(
            message,
            "Soft limit reached. Continue to keep going past the budget."
        );
    }

    #[test]
    fn user_facing_failure_message_falls_back_for_unclassified_errors() {
        // Anything the classifier cannot recognise must funnel to the
        // generic copy so the UI never shows raw error chains.
        let message = user_facing_failure_message(
            "activity_type=reason | error_chain=mysterious bug: something internal broke",
        );

        assert_eq!(
            message,
            "I encountered an error while processing your request. Please try again later."
        );
    }

    #[test]
    fn user_facing_failure_message_normalizes_rate_limit_variants() {
        // Providers surface rate limits in a few shapes; all collapse to the
        // same user-facing copy.
        for raw in [
            "activity_type=reason | error_chain=ReasonAtom execution failed: Rate limit exceeded",
            "activity_type=reason | error_chain=Too Many Requests",
            "activity_type=reason | error_chain=OpenAI API error (429): slow down",
        ] {
            assert_eq!(
                user_facing_failure_message(raw),
                "Rate limited by the AI provider. Please wait a moment.",
                "unexpected mapping for: {raw}"
            );
        }
    }

    /// Tool identifiers are capped so a pathological turn with many tool
    /// calls cannot blow up the persisted failure message or leak every tool
    /// name into the log line. The extractor truncates and records an
    /// `+N more` marker instead.
    #[test]
    fn summarize_task_failure_truncates_long_tool_identifier_lists() {
        let mut tool_calls = Vec::new();
        for i in 0..10 {
            tool_calls.push(json!({"name": format!("tool_{i}")}));
        }
        let input = json!({
            "context": { "session_id": "session_truncate" },
            "tool_calls": tool_calls,
        });
        let error = anyhow::anyhow!("boom");

        let summary = summarize_task_failure(Uuid::nil(), None, "act", 1, Some(1), &input, &error);

        assert_eq!(summary.tool_identifiers.len(), 6);
        assert_eq!(summary.tool_identifiers[0], "tool_0");
        assert_eq!(summary.tool_identifiers[5], "+5 more");
        assert!(
            summary.persisted_message.contains("+5 more"),
            "persisted message should surface truncation marker: {}",
            summary.persisted_message
        );
    }

    /// Missing attempt limits still produce a readable summary — the
    /// attempt field is written as a plain counter rather than `N/None`.
    #[test]
    fn summarize_task_failure_without_max_attempts_emits_plain_counter() {
        let input = json!({
            "context": { "session_id": "session_plain" },
            "tool_calls": []
        });
        let error = anyhow::anyhow!("retryable transient");

        let summary = summarize_task_failure(Uuid::nil(), None, "reason", 4, None, &input, &error);

        assert!(
            summary.persisted_message.contains("attempt=4"),
            "expected plain counter, got: {}",
            summary.persisted_message
        );
        assert!(
            !summary.persisted_message.contains("attempt=4/"),
            "should omit the `/max` suffix when max_attempts is None: {}",
            summary.persisted_message
        );
    }
}
