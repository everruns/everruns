//! Each `tools` call a script makes, recorded in the session timeline.
//!
//! Decisions:
//!
//! - **Its own event, not a tool event.** `tool.started` / `tool.completed`
//!   materialize as conversation messages, and a call the model never made
//!   would break the tool-call pairing the conversation history relies on. A
//!   `tool.nested_call` names the `bash` call it ran in, so a viewer can show
//!   it under that call without it ever reaching the model.
//! - **One event per call, when it ends.** A script's calls are short and
//!   many; one finished record each keeps the timeline readable.
//! - **Best effort.** A call never fails because its record could not be
//!   written, exactly like `tool.progress`.

use std::time::Duration;

use everruns_contracts::runtime::events::{EventRequest, ToolNestedCallData};
use everruns_contracts::runtime::tool_context::ToolContext;
use serde_json::Value;

/// Longest input preview a record keeps, in characters.
const INPUT_PREVIEW_CHARS: usize = 400;

/// How a nested call ended.
pub(super) enum Status<'a> {
    Completed,
    Failed(&'a str),
    NeedsApproval,
    Refused(&'a str),
}

pub(super) struct Call<'a> {
    pub call_id: &'a str,
    pub tool_name: &'a str,
    pub command: &'a str,
    pub input: &'a Value,
    pub status: Status<'a>,
    pub duration: Option<Duration>,
}

pub(super) async fn record(context: &ToolContext, call: Call<'_>) {
    let (Some(emitter), Some(event_context), Some(parent)) = (
        &context.event_emitter,
        &context.event_context,
        &context.tool_call_id,
    ) else {
        return;
    };
    let data = data(parent, call);
    if let Err(error) = emitter
        .emit(EventRequest::new(
            context.session_id,
            event_context.clone(),
            data,
        ))
        .await
    {
        tracing::debug!(%error, parent_tool_call_id = %parent, "failed to emit tool.nested_call");
    }
}

fn data(parent: &str, call: Call<'_>) -> ToolNestedCallData {
    let (status, error) = match call.status {
        Status::Completed => ("completed", None),
        Status::Failed(message) => ("failed", Some(message.to_string())),
        Status::NeedsApproval => ("needs_approval", None),
        Status::Refused(message) => ("refused", Some(message.to_string())),
    };
    ToolNestedCallData {
        parent_tool_call_id: parent.to_string(),
        call_id: call.call_id.to_string(),
        tool_name: call.tool_name.to_string(),
        command: call.command.to_string(),
        status: status.to_string(),
        input_preview: preview(call.input),
        error,
        duration_ms: call
            .duration
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
    }
}

fn preview(input: &Value) -> String {
    let text = input.to_string();
    if text.chars().count() <= INPUT_PREVIEW_CHARS {
        return text;
    }
    let mut short: String = text.chars().take(INPUT_PREVIEW_CHARS).collect();
    short.push('…');
    short
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_record_names_its_parent_call_and_outcome() {
        let input = json!({"repo": "acme/web"});
        let data = data(
            "call_bash_1",
            Call {
                call_id: "call_bash_1:tools:0:mcp_github__delete_branch",
                tool_name: "mcp_github__delete_branch",
                command: "github delete-branch",
                input: &input,
                status: Status::Refused("blocked by guardrail"),
                duration: None,
            },
        );
        assert_eq!(data.parent_tool_call_id, "call_bash_1");
        assert_eq!(data.status, "refused");
        assert_eq!(data.error.as_deref(), Some("blocked by guardrail"));
        assert_eq!(data.input_preview, r#"{"repo":"acme/web"}"#);
        assert_eq!(data.duration_ms, None);
    }

    #[test]
    fn a_long_input_is_shortened() {
        let input = json!({"body": "x".repeat(1000)});
        let shown = preview(&input);
        assert_eq!(shown.chars().count(), INPUT_PREVIEW_CHARS + 1);
        assert!(shown.ends_with('…'));
    }
}
