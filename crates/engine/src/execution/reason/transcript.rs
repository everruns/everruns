use crate::durability::{DurableToolCallStatus, DurableToolResultStore};
use crate::event_emitter::EventEmitter;
use crate::events::{EventContext, EventRequest, TranscriptRepairAction, TranscriptRepairedData};
use crate::message::{Message, MessageRole};
use crate::typed_id::SessionId;

/// Native tool results can commit while the assistant is still streaming.
/// Replay places those results after their owning call without changing events.
pub(super) fn order_native_results(messages: Vec<Message>) -> Vec<Message> {
    let owners: std::collections::HashMap<_, _> = messages
        .iter()
        .enumerate()
        .flat_map(|(index, message)| {
            message
                .tool_calls()
                .into_iter()
                .filter(|call| call.native.is_some())
                .map(move |call| (call.id.clone(), index))
        })
        .collect();
    let mut early: std::collections::BTreeMap<usize, Vec<Message>> =
        std::collections::BTreeMap::new();
    let mut kept = Vec::new();
    for (index, message) in messages.into_iter().enumerate() {
        if let Some(owner) = message
            .tool_call_id()
            .and_then(|id| owners.get(id))
            .filter(|owner| **owner > index)
        {
            early.entry(*owner).or_default().push(message);
        } else {
            kept.push((index, message));
        }
    }
    let mut ordered = Vec::new();
    for (index, message) in kept {
        ordered.push(message);
        if let Some(results) = early.remove(&index) {
            ordered.extend(results);
        }
    }
    ordered
}

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

#[cfg(test)]
mod native_tests {
    use super::*;
    #[test]
    fn early_native_result_replays_after_its_call_without_synthetic_failure() {
        let native = everruns_provider::native_async::NativeToolCall::Function {
            call_id: "original".into(),
            name: "lookup".into(),
            arguments: "{}".into(),
            asynchronous: true,
        };
        let mut call = Message::assistant("Independent reasoning");
        call.content.push(crate::message::ContentPart::ToolCall(
            crate::message::ToolCallContentPart::from_native(native).unwrap(),
        ));
        let output = Message::tool_result("original", Some(serde_json::json!({"answer":42})), None);
        let ordered = order_native_results(vec![
            Message::user("lookup"),
            output.clone(),
            call.clone(),
            Message::assistant("Done"),
        ]);
        assert_eq!(ordered[1].id, call.id);
        assert_eq!(ordered[1].content, call.content);
        assert_eq!(ordered[2].id, output.id);
        assert_eq!(ordered[2].content, output.content);
        assert_eq!(ordered.len(), 4);
    }
}
