use crate::durability::{DurableToolCallStatus, DurableToolResultStore};
use crate::event_emitter::EventEmitter;
use crate::events::{EventContext, EventRequest, TranscriptRepairAction, TranscriptRepairedData};
use crate::message::{Message, MessageRole};
use crate::typed_id::SessionId;

/// Repair assistant tool calls without matching results before the next model
/// request. Durable status determines whether replay is safe or the result is
/// uncertain.
pub(super) async fn repair_dangling_tool_calls(
    messages: &[Message],
    durable_store: Option<&dyn DurableToolResultStore>,
    event_emitter: &dyn EventEmitter,
    session_id: SessionId,
    event_context: &EventContext,
    turn_id: &str,
) -> Vec<Message> {
    let mut result = Vec::new();

    for (index, message) in messages.iter().enumerate() {
        result.push(message.clone());

        if message.role != MessageRole::Agent || !message.has_tool_calls() {
            continue;
        }

        for tool_call in message.tool_calls() {
            let has_result = messages[(index + 1)..].iter().any(|candidate| {
                candidate.role == MessageRole::ToolResult
                    && candidate.tool_call_id() == Some(&tool_call.id)
            });
            if has_result {
                continue;
            }

            let (repair_message, action) = if let Some(store) = durable_store {
                match store.get_tool_call_status(turn_id, &tool_call.id).await {
                    Ok(Some(DurableToolCallStatus::Settled { result_json })) => {
                        let repair = match serde_json::from_value::<
                            crate::tool_types::ToolResult,
                        >(result_json.clone())
                        {
                            Ok(tool_result) => Message::tool_result(
                                &tool_call.id,
                                tool_result.result,
                                tool_result.error,
                            ),
                            Err(_) => {
                                Message::tool_result(&tool_call.id, Some(result_json), None)
                            }
                        };
                        (repair, TranscriptRepairAction::Replay)
                    }
                    Ok(Some(DurableToolCallStatus::Interrupted { result_json })) => {
                        let error = result_json
                            .as_ref()
                            .and_then(|value| {
                                serde_json::from_value::<crate::tool_types::ToolResult>(
                                    value.clone(),
                                )
                                .ok()
                            })
                            .and_then(|result| result.error)
                            .unwrap_or_else(|| {
                                "tool execution did not complete before recovery; result unknown"
                                    .to_string()
                            });
                        (
                            Message::tool_result(&tool_call.id, None, Some(error)),
                            TranscriptRepairAction::Replay,
                        )
                    }
                    Ok(Some(DurableToolCallStatus::Running)) => (
                        Message::tool_result(
                            &tool_call.id,
                            None,
                            Some(
                                "interrupted - tool execution was interrupted by worker failure and the result is uncertain; do not retry automatically"
                                    .to_string(),
                            ),
                        ),
                        TranscriptRepairAction::Synthesize,
                    ),
                    Ok(None) => (
                        Message::tool_result(
                            &tool_call.id,
                            None,
                            Some(
                                "interrupted - tool was not executed before recovery; safe to retry"
                                    .to_string(),
                            ),
                        ),
                        TranscriptRepairAction::Synthesize,
                    ),
                    Err(error) => {
                        tracing::warn!(
                            tool_call_id = %tool_call.id,
                            error = %error,
                            "transcript repair: durable store error; status unknown"
                        );
                        (
                            Message::tool_result(
                                &tool_call.id,
                                None,
                                Some(
                                    "interrupted - tool execution status unknown due to store error; do not retry automatically"
                                        .to_string(),
                                ),
                            ),
                            TranscriptRepairAction::Synthesize,
                        )
                    }
                }
            } else {
                (
                    Message::tool_result(
                        &tool_call.id,
                        None,
                        Some(
                            "cancelled - another message came in before it could be completed"
                                .to_string(),
                        ),
                    ),
                    TranscriptRepairAction::Synthesize,
                )
            };

            let repair_event = EventRequest::new(
                session_id,
                event_context.clone(),
                TranscriptRepairedData {
                    tool_call_id: tool_call.id.clone(),
                    tool_name: Some(tool_call.name.clone()),
                    action,
                },
            );
            if let Err(error) = event_emitter.emit(repair_event).await {
                tracing::warn!(
                    tool_call_id = %tool_call.id,
                    error = %error,
                    "transcript repair: failed to emit transcript.repaired event"
                );
            }

            result.push(repair_message);
        }
    }

    result
}
