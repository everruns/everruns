//! OpenAI hosted tools on the Open Responses driver (EVE-1115).
//!
//! Request side: render [`OpenAiHostedTools`] into the `tools` array, or fail
//! when this endpoint cannot run them. Response side: hosted calls arrive as
//! `web_search_call` output items and `response.web_search_call.*` events.
//! They are provider-executed, so they never become agent tool calls; the
//! stream reports them as `HostedToolCall` progress events, never as agent
//! tool calls, and the answer text carries the citations.

use serde_json::Value;

use crate::driver_registry::LlmCallConfig;
use crate::error::{AgentLoopError, Result};
use std::collections::BTreeMap;

use crate::driver_registry::{HostedToolCall, HostedToolCallStatus, LlmStreamEvent};
use crate::openai_hosted_tools::{
    OPENAI_MCP_APPROVAL_TOOL, OpenAiHostedTools, count_hosted_tool_calls, hosted_call_tool,
};

use super::OpenResponsesProtocolChatDriver;
use super::wire::ResponsesInputItem;

impl OpenResponsesProtocolChatDriver {
    /// Wire entries for the hosted tools this call asked for.
    ///
    /// An endpoint without hosted tools (an OpenAI-compatible gateway) errors
    /// rather than sending a request that silently lacks them.
    pub(crate) fn hosted_tools_for(&self, config: &LlmCallConfig) -> Result<Vec<Value>> {
        let requested =
            OpenAiHostedTools::from_driver_options(&config.driver_options).map_err(|error| {
                AgentLoopError::Configuration(format!("invalid OpenAI hosted tools: {error}"))
            })?;
        let Some(requested) = requested else {
            return Ok(Vec::new());
        };
        if !self.hosted_tools {
            return Err(AgentLoopError::Configuration(
                "OpenAI hosted tools need the OpenAI or Azure OpenAI Responses API; \
                 this provider does not run them"
                    .to_string(),
            ));
        }
        Ok(requested.wire_tools())
    }
}

/// Hosted call counts from a terminal `response` object, for
/// [`crate::driver_registry::LlmCompletionMetadata::hosted_tool_calls`].
pub(crate) fn hosted_tool_calls(response: &Value) -> BTreeMap<String, u32> {
    response
        .get("output")
        .and_then(Value::as_array)
        .map(|output| count_hosted_tool_calls(output))
        .unwrap_or_default()
}

/// Map a hosted call's `response.output_item.added` / `.done` frame to a
/// [`LlmStreamEvent::HostedToolCall`]. `None` for every other frame.
pub(crate) fn hosted_call_event(event_data: &str) -> Option<LlmStreamEvent> {
    // Cheap pre-filter: text deltas dominate the stream.
    if !event_data.contains("_call") || !event_data.contains("response.output_item.") {
        return None;
    }
    let frame: Value = serde_json::from_str(event_data).ok()?;
    let done = match frame.get("type").and_then(Value::as_str)? {
        "response.output_item.added" => false,
        "response.output_item.done" => true,
        _ => return None,
    };
    let item = frame.get("item")?;
    let tool = hosted_call_tool(item.get("type").and_then(Value::as_str)?)?;
    let failed = matches!(
        item.get("status").and_then(Value::as_str),
        Some("failed" | "incomplete")
    );
    let status = match (done, failed) {
        (_, true) => HostedToolCallStatus::Failed,
        (true, false) => HostedToolCallStatus::Completed,
        (false, false) => HostedToolCallStatus::InProgress,
    };
    let summary = call_summary(item).map(|detail| detail.chars().take(200).collect());
    Some(LlmStreamEvent::HostedToolCall(HostedToolCall {
        id: item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        tool: tool.to_string(),
        status,
        summary,
    }))
}

/// `(approval_request_id, arguments JSON)` for an `mcp_approval_request`
/// output item, surfaced as a synthetic [`OPENAI_MCP_APPROVAL_TOOL`] call.
pub(crate) fn mcp_approval_call(item: &Value) -> Option<(String, String)> {
    crate::openai_hosted_tools::mcp_approval_call(item).map(|(id, args)| (id, args.to_string()))
}

/// Turn replayed approval calls back into OpenAI's items: the synthetic call
/// becomes the `mcp_approval_request` it came from, and its result becomes an
/// `mcp_approval_response`. Only a result of exactly `{"approve": true}`
/// approves; anything else, an error included, denies.
pub(crate) fn replay_mcp_approvals(items: Vec<ResponsesInputItem>) -> Vec<ResponsesInputItem> {
    let mut approval_ids = std::collections::HashSet::new();
    items
        .into_iter()
        .map(|item| match item {
            ResponsesInputItem::FunctionCall {
                call_id,
                name,
                arguments,
                ..
            } if name == OPENAI_MCP_APPROVAL_TOOL => {
                let args: Value = serde_json::from_str(&arguments).unwrap_or_default();
                let field = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or_default();
                let request = serde_json::json!({
                    "type": "mcp_approval_request",
                    "id": call_id,
                    "server_label": field("server_label"),
                    "name": field("name"),
                    "arguments": field("arguments"),
                });
                approval_ids.insert(call_id);
                ResponsesInputItem::ProviderItem(request)
            }
            ResponsesInputItem::FunctionCallOutput {
                call_id, output, ..
            } if approval_ids.contains(&call_id) => {
                let approve = serde_json::from_str::<Value>(&output)
                    .ok()
                    .and_then(|result| result.get("approve").and_then(Value::as_bool))
                    .unwrap_or(false);
                ResponsesInputItem::ProviderItem(serde_json::json!({
                    "type": "mcp_approval_response",
                    "approval_request_id": call_id,
                    "approve": approve,
                }))
            }
            other => other,
        })
        .collect()
}

/// A one-line detail for a hosted call item, once the provider reports it.
fn call_summary(item: &Value) -> Option<String> {
    let action = item.get("action");
    let field = |key: &str| action.and_then(|a| a.get(key)).and_then(Value::as_str);
    let first_line = |text: &str| {
        text.lines()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .map(str::to_string)
    };
    match item.get("type").and_then(Value::as_str)? {
        // `search` carries `query`, `open_page` a `url`, `find` a `pattern` in a `url`.
        "web_search_call" => field("query").or_else(|| field("url")).map(str::to_string),
        "code_interpreter_call" => item
            .get("code")
            .and_then(Value::as_str)
            .and_then(first_line),
        "shell_call" => action
            .and_then(|a| a.get("commands"))
            .and_then(Value::as_array)
            .map(|commands| {
                commands
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|commands| !commands.is_empty()),
        "mcp_call" => Some(format!(
            "{}: {}",
            item.get("server_label")
                .and_then(Value::as_str)
                .unwrap_or("mcp"),
            item.get("name").and_then(Value::as_str)?
        )),
        "file_search_call" => item
            .get("queries")
            .and_then(Value::as_array)
            .and_then(|queries| queries.first())
            .and_then(Value::as_str)
            .map(str::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(frame: Value) -> Option<HostedToolCall> {
        match hosted_call_event(&frame.to_string())? {
            LlmStreamEvent::HostedToolCall(call) => Some(call),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn web_search_item_frames_become_hosted_call_events() {
        let started = event(json!({
            "type": "response.output_item.added", "output_index": 0,
            "item": { "type": "web_search_call", "id": "ws_1", "status": "in_progress" }
        }))
        .unwrap();
        assert_eq!(started.status, HostedToolCallStatus::InProgress);
        assert_eq!(started.tool, "web_search");
        assert_eq!(started.summary, None);

        let done = event(json!({
            "type": "response.output_item.done", "output_index": 0,
            "item": { "type": "web_search_call", "id": "ws_1", "status": "completed",
                      "action": { "type": "search", "query": "everruns release" } }
        }))
        .unwrap();
        assert_eq!(done.id, "ws_1");
        assert_eq!(done.status, HostedToolCallStatus::Completed);
        assert_eq!(done.summary.as_deref(), Some("everruns release"));

        let failed = event(json!({
            "type": "response.output_item.done",
            "item": { "type": "web_search_call", "id": "ws_2", "status": "failed" }
        }))
        .unwrap();
        assert_eq!(failed.status, HostedToolCallStatus::Failed);
    }

    #[test]
    fn container_and_file_calls_carry_their_detail() {
        let done = |item: Value| {
            event(json!({ "type": "response.output_item.done", "item": item })).unwrap()
        };
        let code = done(json!({ "type": "code_interpreter_call", "id": "ci_1",
                                "status": "completed", "code": "\nimport math\nmath.pi\n" }));
        assert_eq!(
            (code.tool.as_str(), code.summary.as_deref()),
            ("code_interpreter", Some("import math"))
        );
        let shell = done(
            json!({ "type": "shell_call", "id": "sh_1", "status": "completed",
                                 "action": { "commands": ["ls", "uname -s"] } }),
        );
        assert_eq!(
            (shell.tool.as_str(), shell.summary.as_deref()),
            ("shell", Some("ls; uname -s"))
        );
        let files = done(
            json!({ "type": "file_search_call", "id": "fs_1", "status": "completed",
                                 "queries": ["refund policy"] }),
        );
        assert_eq!(
            (files.tool.as_str(), files.summary.as_deref()),
            ("file_search", Some("refund policy"))
        );
        // The shell's output item is part of the same call, not another call.
        assert!(
            event(json!({ "type": "response.output_item.done",
                              "item": { "type": "shell_call_output", "id": "sho_1" } }))
            .is_none()
        );
    }

    #[test]
    fn mcp_calls_are_hosted_calls_and_approvals_are_not() {
        let call = event(json!({ "type": "response.output_item.done", "item": {
            "type": "mcp_call", "id": "mcp_1", "status": "completed",
            "server_label": "deepwiki", "name": "ask_question" } }))
        .unwrap();
        assert_eq!(
            (call.tool.as_str(), call.summary.as_deref()),
            ("mcp", Some("deepwiki: ask_question"))
        );
        assert!(
            event(json!({ "type": "response.output_item.done", "item": {
            "type": "mcp_approval_request", "id": "mcpr_1" } }))
            .is_none()
        );
    }

    #[test]
    fn approval_replays_as_request_and_response() {
        let args = json!({ "server_label": "deepwiki", "name": "ask", "arguments": "{}" });
        let call = |id: &str| ResponsesInputItem::FunctionCall {
            r#type: "function_call".into(),
            call_id: id.into(),
            name: OPENAI_MCP_APPROVAL_TOOL.into(),
            arguments: args.to_string(),
        };
        let output = |id: &str, output: &str| ResponsesInputItem::FunctionCallOutput {
            r#type: "function_call_output".into(),
            call_id: id.into(),
            output: output.into(),
        };
        let items = replay_mcp_approvals(vec![
            call("mcpr_1"),
            output("mcpr_1", r#"{"approve":true}"#),
            call("mcpr_2"),
            output("mcpr_2", "denied by user"),
            output("fc_1", r#"{"approve":true}"#),
        ]);
        let wire: Vec<Value> = items
            .iter()
            .map(|i| serde_json::to_value(i).unwrap())
            .collect();
        assert_eq!(
            wire[0],
            json!({ "type": "mcp_approval_request", "id": "mcpr_1", "server_label": "deepwiki",
                    "name": "ask", "arguments": "{}" })
        );
        assert_eq!(
            wire[1],
            json!({ "type": "mcp_approval_response", "approval_request_id": "mcpr_1", "approve": true })
        );
        assert_eq!(wire[3]["approve"], false);
        // An ordinary function output is untouched.
        assert_eq!(wire[4]["type"], "function_call_output");
    }

    #[test]
    fn other_frames_are_not_hosted_calls() {
        for frame in [
            json!({ "type": "response.output_item.done",
                    "item": { "type": "function_call", "id": "fc_1", "call_id": "c", "name": "f", "arguments": "{}" } }),
            json!({ "type": "response.web_search_call.searching", "item_id": "ws_1" }),
            json!({ "type": "response.output_text.delta", "delta": "a web_search_call" }),
        ] {
            assert!(event(frame).is_none());
        }
    }

    #[test]
    fn terminal_response_counts_hosted_calls() {
        let counts = hosted_tool_calls(&json!({ "output": [
            { "type": "web_search_call", "id": "ws_1" },
            { "type": "message", "id": "m" },
        ]}));
        assert_eq!(counts.get("web_search_call"), Some(&1));
        assert!(hosted_tool_calls(&json!({})).is_empty());
    }
}
