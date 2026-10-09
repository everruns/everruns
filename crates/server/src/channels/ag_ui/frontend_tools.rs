// AG-UI frontend tools over the AG-UI endpoint.
//
// Spec: knowledge/integrations/ag-ui.md. `RunAgentInput.tools` are tools the
// consumer executes. They become the session's client-side tools, so the turn
// parks on a call to one exactly as it does for a client-side tool declared
// through the session API. The parked calls stream to the consumer as
// TOOL_CALL_START/ARGS/END and the run finishes as a success naming them in
// `pendingToolCallIds`; the consumer's next run carries the results as trailing
// `tool` messages, which land on the same waiting-turn resolution
// `POST /v1/sessions/{id}/tool-results` uses.
//
// THREAT[TM-CLIENT-004]: definitions go through the session API's client-side
// filter, so a consumer cannot declare a tool under the reserved `mcp_` prefix
// and shadow a worker-executed guardrail endpoint.
// THREAT[TM-DOS-044]: the definitions an anonymous consumer can put in the
// model's context are bounded in count, name, description and schema size.
// THREAT[TM-LLM-020]: a `tool` message is used only as the result of a call
// the agent made to one of this run's frontend tools and that is parked right
// now. Any other `tool` message is dropped, never written to the transcript.

use std::collections::HashSet;

use crate::records::SessionStatus;
use everruns_contracts::tool_types::{ClientSideTool, ToolDefinition};
use everruns_core::ag_ui::{Message as AgUiMessage, Tool as AgUiTool, ToolCall as AgUiToolCall};
use everruns_core::events::ToolCallRequestedData;
use serde_json::Value;

use crate::api::question_answers::QUESTION_LOOKBACK_EVENTS;
use crate::api::tool_results::{ClientToolResult, tool_results_plan};
use crate::channels::ag_ui::interrupts::{ParkedCalls, ResumeError, ResumeOutcome, ResumeServices};
use crate::storage::ClaimWaitingTurnResult;

/// Most frontend tools one run may declare.
pub(crate) const MAX_FRONTEND_TOOLS: usize = 64;
/// Longest frontend tool description, in characters.
const MAX_DESCRIPTION_CHARS: usize = 4096;
/// Largest serialized parameter schema of one frontend tool.
const MAX_SCHEMA_BYTES: usize = 16 * 1024;
/// Largest result one `tool` message may carry.
const MAX_RESULT_BYTES: usize = 256 * 1024;

/// A run's frontend tools as client-side tool definitions.
pub(crate) fn definitions(tools: &[AgUiTool]) -> Result<Vec<ToolDefinition>, String> {
    if tools.len() > MAX_FRONTEND_TOOLS {
        return Err(format!("at most {MAX_FRONTEND_TOOLS} tools per run"));
    }
    let mut seen = HashSet::with_capacity(tools.len());
    let definitions = tools
        .iter()
        .map(|tool| {
            if !is_valid_tool_name(&tool.name) || !seen.insert(tool.name.as_str()) {
                return Err("tool names must be unique and match ^[A-Za-z0-9_-]{1,64}$".to_string());
            }
            if tool.description.chars().count() > MAX_DESCRIPTION_CHARS {
                return Err(format!(
                    "tool descriptions are limited to {MAX_DESCRIPTION_CHARS} characters"
                ));
            }
            let parameters = tool
                .parameters
                .clone()
                .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} }));
            if !parameters.is_object() || parameters.to_string().len() > MAX_SCHEMA_BYTES {
                return Err(format!(
                    "tool parameters must be a JSON Schema object under {MAX_SCHEMA_BYTES} bytes"
                ));
            }
            Ok(ToolDefinition::ClientSide(ClientSideTool::new(
                tool.name.clone(),
                tool.description.clone(),
                parameters,
            )))
        })
        .collect::<Result<Vec<_>, _>>()?;
    crate::api::sessions::filter_or_reject_client_side_tools(definitions).map_err(str::to_string)
}

fn is_valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

/// The parked calls the consumer runs: calls to one of `names` that no
/// question or approval claims.
pub(crate) fn pending_calls(
    requested: &ToolCallRequestedData,
    parked: &ParkedCalls,
    names: &HashSet<String>,
) -> Vec<AgUiToolCall> {
    requested
        .tool_calls
        .iter()
        .filter(|call| names.contains(&call.name) && !parked.claims(&call.id))
        .map(|call| AgUiToolCall::function(&call.id, &call.name, call.arguments.to_string()))
        .collect()
}

/// The `tool` messages after the last message of any other role: the results
/// a run that continues after frontend tool calls carries. Empty unless the
/// input ends with a `tool` message.
pub(crate) fn trailing_results(messages: &[AgUiMessage]) -> Vec<ClientToolResult> {
    messages
        .iter()
        .rev()
        .map_while(|message| match message {
            AgUiMessage::Tool(tool) => Some(ClientToolResult {
                tool_call_id: tool.tool_call_id.clone(),
                result: tool.error.is_none().then(|| {
                    let text = tool.content.to_text();
                    serde_json::from_str(&text).unwrap_or(Value::String(text))
                }),
                error: tool.error.clone(),
            }),
            _ => None,
        })
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

/// Record a run's frontend tool results on the thread's parked turn and
/// resume it. Results for calls that are not parked are ignored with a
/// warning; while a parked frontend call has no result, nothing is recorded
/// and the run reports the calls again.
pub(crate) async fn submit_results(
    services: &ResumeServices<'_>,
    org_id: i64,
    session: &crate::records::Session,
    names: &HashSet<String>,
    results: Vec<ClientToolResult>,
) -> Result<ResumeOutcome, ResumeError> {
    if session.status != SessionStatus::WaitingForToolResults {
        warn_unrecognised(session, results.iter());
        return Ok(ResumeOutcome::NothingParked);
    }
    // THREAT[TM-TENANT-016]: `session` is the thread's own; a result can only
    // answer a call this thread's turn made.
    let rows = services
        .db
        .list_events(
            session.id,
            None,
            None,
            &["tool.call_requested".to_string()],
            &[],
            None,
            Some(QUESTION_LOOKBACK_EVENTS),
        )
        .await
        .map_err(ResumeError::Internal)?;
    let Some(row) = rows.last() else {
        warn_unrecognised(session, results.iter());
        return Ok(ResumeOutcome::NothingParked);
    };
    let requested: ToolCallRequestedData = serde_json::from_value(row.data.clone())
        .map_err(|error| ResumeError::Internal(error.into()))?;
    let parked = ParkedCalls::from_request(&requested);
    let pending = pending_calls(&requested, &parked, names);
    if pending.is_empty() {
        warn_unrecognised(session, results.iter());
        return Ok(ResumeOutcome::NothingParked);
    }
    let (results, unrecognised): (Vec<_>, Vec<_>) = results
        .into_iter()
        .partition(|result| pending.iter().any(|call| call.id == result.tool_call_id));
    warn_unrecognised(session, unrecognised.iter());
    if pending
        .iter()
        .any(|call| !results.iter().any(|result| result.tool_call_id == call.id))
    {
        return Ok(ResumeOutcome::StillOpen {
            tool_calls: pending,
            interrupts: Vec::new(),
        });
    }
    if results.iter().any(|result| {
        result
            .result
            .as_ref()
            .map_or(0, |value| value.to_string().len())
            + result.error.as_ref().map_or(0, String::len)
            > MAX_RESULT_BYTES
    }) {
        return Err(ResumeError::Invalid(format!(
            "a tool result is limited to {MAX_RESULT_BYTES} bytes"
        )));
    }

    let plan = tool_results_plan(session.id, &results);
    let claim = match services
        .db
        .claim_waiting_turn(org_id, session.id, plan)
        .await
        .map_err(ResumeError::Internal)?
    {
        ClaimWaitingTurnResult::Claimed(claim) => claim,
        ClaimWaitingTurnResult::Conflict { current_status } => {
            return Err(ResumeError::Conflict(format!(
                "the thread is no longer waiting for tool results ({current_status})"
            )));
        }
        ClaimWaitingTurnResult::SessionNotFound => {
            return Err(ResumeError::Conflict("the thread is gone".to_string()));
        }
    };
    crate::domains::tool_results::waiting_turn_resolution::execute_waiting_turn_resolution(
        services.db,
        services.event_service,
        &services.runner,
        org_id,
        session.id,
        &claim,
    )
    .await
    .map_err(ResumeError::Internal)?;
    Ok(ResumeOutcome::Resumed {
        input_message_id: row
            .context
            .get("input_message_id")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn warn_unrecognised<'a>(
    session: &crate::records::Session,
    results: impl Iterator<Item = &'a ClientToolResult>,
) {
    for result in results {
        tracing::warn!(
            session_id = %session.id,
            tool_call_id = %result.tool_call_id,
            "AG-UI tool message answers no parked frontend tool call; ignoring it"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str) -> AgUiTool {
        serde_json::from_value(json!({ "name": name, "description": "d" })).unwrap()
    }

    #[test]
    fn definitions_become_client_side_tools() {
        let definitions = definitions(&[tool("confirm"), tool("open-page")]).unwrap();
        assert_eq!(definitions.len(), 2);
        assert!(matches!(&definitions[0], ToolDefinition::ClientSide(t) if t.name == "confirm"));
    }

    #[test]
    fn definitions_refuse_reserved_duplicate_and_oversized_tools() {
        assert!(definitions(&[tool("mcp_guard__screen")]).is_err());
        assert!(definitions(&[tool("confirm"), tool("confirm")]).is_err());
        assert!(definitions(&[tool("has space")]).is_err());
        let many: Vec<_> = (0..=MAX_FRONTEND_TOOLS)
            .map(|i| tool(&format!("t{i}")))
            .collect();
        assert!(definitions(&many).is_err());
        let mut big = tool("big");
        big.parameters = Some(json!({ "description": "x".repeat(MAX_SCHEMA_BYTES) }));
        assert!(definitions(&[big]).is_err());
    }

    #[test]
    fn trailing_results_take_only_the_final_tool_messages() {
        let messages: Vec<AgUiMessage> = serde_json::from_value(json!([
            { "id": "t0", "role": "tool", "toolCallId": "old", "content": "stale" },
            { "id": "u1", "role": "user", "content": "hi" },
            { "id": "t1", "role": "tool", "toolCallId": "c1", "content": "{\"ok\":true}" },
            { "id": "t2", "role": "tool", "toolCallId": "c2", "content": "plain", "error": "boom" },
        ]))
        .unwrap();
        let results = trailing_results(&messages);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].tool_call_id, "c1");
        assert_eq!(results[0].result, Some(json!({ "ok": true })));
        assert_eq!(results[1].error.as_deref(), Some("boom"));
        assert!(results[1].result.is_none());
        assert!(trailing_results(&messages[..2]).is_empty());
    }
}
