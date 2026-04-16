// Task failure formatting helpers
// Decision: Persist a single human-readable failure string that keeps both
// task metadata and the full anyhow error chain for durable task surfaces.

use anyhow::Error;
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
    session_context(input).and_then(|context| {
        context
            .get("session_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
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
}
