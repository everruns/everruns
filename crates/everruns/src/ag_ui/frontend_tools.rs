//! Frontend tools: bounding a run's `RunAgentInput.tools` and matching the
//! next run's trailing `tool` messages to the calls a turn parked on.

use super::*;

/// Most frontend tools one run may declare.
const MAX_FRONTEND_TOOLS: usize = 64;
/// Longest frontend tool description, in characters.
const MAX_DESCRIPTION_CHARS: usize = 4096;
/// Largest serialized parameter schema of one frontend tool.
const MAX_SCHEMA_BYTES: usize = 16 * 1024;
/// Largest result one `tool` message may carry.
const MAX_RESULT_BYTES: usize = 256 * 1024;

/// A run's frontend tools as client-side tool definitions, bounded like the
/// Everruns server bounds them.
// THREAT[TM-DOS-044]: the definitions a consumer puts in the model's context
// are bounded in count, name, description and schema size.
// THREAT[TM-CLIENT-004]: the reserved `mcp_` prefix is refused, so a consumer
// cannot declare a tool that shadows an MCP tool.
pub(super) fn frontend_definitions(tools: &[WireTool]) -> Result<Vec<ToolDefinition>, AgUiError> {
    if tools.len() > MAX_FRONTEND_TOOLS {
        return Err(invalid(format!(
            "at most {MAX_FRONTEND_TOOLS} tools per run"
        )));
    }
    let mut seen = HashSet::with_capacity(tools.len());
    tools
        .iter()
        .map(|tool| {
            let valid_name = !tool.name.is_empty()
                && tool.name.len() <= 64
                && tool
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'));
            if !valid_name || !seen.insert(tool.name.as_str()) {
                return Err(invalid(
                    "tool names must be unique and match ^[A-Za-z0-9_-]{1,64}$",
                ));
            }
            if tool.name.starts_with("mcp_") {
                return Err(invalid("tool names must not use the reserved mcp_ prefix"));
            }
            if tool.description.chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(invalid(format!(
                    "tool descriptions are limited to {MAX_DESCRIPTION_CHARS} characters"
                )));
            }
            let parameters = tool
                .parameters
                .clone()
                .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
            if !parameters.is_object() || parameters.to_string().len() > MAX_SCHEMA_BYTES {
                return Err(invalid(format!(
                    "tool parameters must be a JSON Schema object under {MAX_SCHEMA_BYTES} bytes"
                )));
            }
            Ok(ToolDefinition::ClientSide(ClientSideTool::new(
                tool.name.clone(),
                tool.description.clone(),
                parameters,
            )))
        })
        .collect()
}

/// One frontend tool result a `tool` message carries.
pub(super) struct FrontendResult {
    tool_call_id: String,
    result: Option<Value>,
    error: Option<String>,
}

/// The `tool` messages after the last message of any other role: the results
/// a run that continues after frontend tool calls carries. Empty unless the
/// input ends with a `tool` message.
pub(super) fn trailing_results(messages: &[Message]) -> Vec<FrontendResult> {
    let mut results: Vec<FrontendResult> = messages
        .iter()
        .rev()
        .map_while(|message| match message {
            Message::Tool(tool) => Some(FrontendResult {
                tool_call_id: tool.tool_call_id.clone(),
                result: tool.error.is_none().then(|| {
                    let text = tool.content.to_text();
                    serde_json::from_str(&text).unwrap_or(Value::String(text))
                }),
                error: tool.error.clone(),
            }),
            _ => None,
        })
        .collect();
    results.reverse();
    results
}

impl Session {
    /// The session's parked calls to one of `frontend`, in the order the
    /// model made them, as wire tool calls.
    pub(super) fn pending_frontend_calls(&self, frontend: &HashSet<String>) -> Vec<WireToolCall> {
        self.parked_tool_calls()
            .map(|parked| {
                parked
                    .tool_calls
                    .into_iter()
                    .filter(|call| frontend.contains(&call.name))
                    .map(|call| {
                        WireToolCall::function(call.id, call.name, call.arguments.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Record a run's frontend tool results on the parked turn and resume
    /// it. Results for calls that are not parked are ignored with a warning;
    /// while a parked frontend call has no result, nothing is recorded and
    /// the run reports the calls again.
    pub(super) async fn submit_frontend_results(
        &self,
        frontend: &HashSet<String>,
        results: Vec<FrontendResult>,
        open_interrupts: impl Fn() -> Vec<Interrupt>,
    ) -> Result<Start, AgUiError> {
        let pending = self.pending_frontend_calls(frontend);
        let (results, unrecognised): (Vec<_>, Vec<_>) = results
            .into_iter()
            .partition(|result| pending.iter().any(|call| call.id == result.tool_call_id));
        for result in &unrecognised {
            tracing::warn!(
                session_id = %self.session_id(),
                tool_call_id = %result.tool_call_id,
                "AG-UI tool message answers no parked frontend tool call; ignoring it"
            );
        }
        if pending.is_empty() {
            return Ok(Start::Empty);
        }
        let result_for = |id: &str| results.iter().find(|result| result.tool_call_id == id);
        if pending.iter().any(|call| result_for(&call.id).is_none()) {
            return Ok(Start::Park(pending, open_interrupts()));
        }
        if results.iter().any(|result| {
            result
                .result
                .as_ref()
                .map_or(0, |value| value.to_string().len())
                + result.error.as_ref().map_or(0, String::len)
                > MAX_RESULT_BYTES
        }) {
            return Err(invalid(format!(
                "a tool result is limited to {MAX_RESULT_BYTES} bytes"
            )));
        }
        let completed = pending
            .iter()
            .filter_map(|call| {
                let result = result_for(&call.id)?;
                let id = call.id.clone();
                let name = call.function.name.clone();
                Some(match &result.error {
                    Some(error) => {
                        ToolCompletedData::failure(id, name, "error".into(), error.clone(), None)
                    }
                    None => ToolCompletedData::success(
                        id,
                        name,
                        result
                            .result
                            .as_ref()
                            .map(|value| vec![ContentPart::tool_result_text(value)])
                            .unwrap_or_default(),
                        None,
                    ),
                })
            })
            .collect();
        self.resume_tool_results(completed).await?;
        Ok(Start::Follow)
    }
}
