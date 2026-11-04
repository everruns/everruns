use crate::engine::durability::{DurableToolCallStatus, DurableToolResultStore};
use crate::engine::event_emitter::EventEmitter;
use crate::engine::events::{
    EventContext, EventRequest, TranscriptRepairAction, TranscriptRepairedData,
};
use crate::engine::message::{RuntimeMessage, RuntimeMessageRole};
use crate::engine::typed_id::SessionId;

/// Native tool results can commit while the assistant is still streaming.
/// Replay places those results after their owning call without changing events.
/// Tell the model who is speaking in a user-role message: an external actor's
/// display label, and the platform origin (schedule run, trigger, task wake-up,
/// notice) so a platform message is never mistaken for the person's own words.
pub(super) fn label_speaker(llm_msg: &mut crate::driver_registry::Message, msg: &RuntimeMessage) {
    if msg.role != RuntimeMessageRole::User {
        return;
    }
    if let Some(ref actor) = msg.external_actor {
        llm_msg.prepend_text_prefix(&format!("[{}] ", actor.display_label()));
    }
    let origin = msg.origin();
    if let Some(header) = origin.as_ref().map(|origin| origin.model_header())
        && !header.is_empty()
    {
        llm_msg.prepend_text_prefix(&header);
    }
    // Reactions that waited for this message come first, on their own lines,
    // so they never read as part of what the person typed. A reaction-origin
    // message already is the note.
    if origin.is_none_or(|origin| {
        origin.kind != everruns_contracts::message_origin::MessageOriginKind::Reaction
    }) && let Some(note) =
        everruns_contracts::message_reactions::ReactionNote::from_message_metadata(
            msg.metadata.as_ref(),
        )
    {
        llm_msg.prepend_text_prefix(&format!("{}\n\n", note.render()));
    }
}

pub(super) fn order_native_results(messages: Vec<RuntimeMessage>) -> Vec<RuntimeMessage> {
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
    let mut early: std::collections::BTreeMap<usize, Vec<RuntimeMessage>> =
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

/// Whether the latest agent step already settled the conversation: every call
/// is `send_message` or `no_reply`, every call succeeded, and only those
/// results came after. A steering message or task wake after them means
/// someone spoke again, so the model is asked as usual.
///
/// The tool promises to end the turn, but a tool result always feeds another
/// generation. Some models answer that generation with yet another
/// `no_reply`, or with filler sent through `send_message`, instead of
/// stopping, which kept one Slack turn spinning for over a minute. A step
/// that only delivered a message has the same shape: the person already has
/// the text, and the follow-up generation only repeats it.
#[cfg(test)]
pub(super) fn ended_by_no_reply(messages: &[RuntimeMessage]) -> bool {
    ended_by_tools(messages, |name| {
        name == crate::conversation::NO_REPLY_TOOL_NAME
    })
}

pub(super) fn ended_by_communication(messages: &[RuntimeMessage]) -> bool {
    ended_by_tools(messages, |name| {
        matches!(
            name,
            crate::conversation::NO_REPLY_TOOL_NAME
                | crate::conversation::SEND_MESSAGE_TOOL_NAME
                | "channel_post_message"
        )
    })
}

fn ended_by_tools(messages: &[RuntimeMessage], tool: impl Fn(&str) -> bool) -> bool {
    let Some(index) = messages
        .iter()
        .rposition(|message| message.role == RuntimeMessageRole::Agent)
    else {
        return false;
    };
    let calls = messages[index].tool_calls();
    let after = &messages[index + 1..];
    !calls.is_empty()
        && calls.iter().all(|call| tool(&call.name))
        && after.iter().all(|message| {
            message.role == RuntimeMessageRole::ToolResult
                && message
                    .tool_result_content()
                    .is_some_and(|result| result.error.is_none())
        })
        && calls.iter().all(|call| {
            after
                .iter()
                .any(|message| message.tool_call_id() == Some(call.id.as_str()))
        })
}

/// The finished reason step for an explicit agent whose latest step already
/// delivered or declined (see [`ended_by_communication`]); `None` asks the
/// model as usual.
pub(super) fn ended_by_no_reply_result(
    assembled: &crate::engine::runtime_context::AssembledTurnContext,
) -> Option<super::ReasonResult> {
    let agent = &assembled.runtime_agent;
    if !agent.communication.is_explicit() || !ended_by_communication(&assembled.messages) {
        return None;
    }
    Some(super::ReasonResult {
        success: true,
        commentary: true,
        tool_definitions: agent.tools.clone(),
        max_iterations: agent.max_iterations,
        locale: assembled.resolved_locale.clone(),
        network_access: agent.network_access.clone(),
        parallel_tool_calls: agent.parallel_tool_calls,
        ..Default::default()
    })
}

/// Repair assistant tool calls without matching results before the next model
/// request. Durable status determines whether replay is safe or the result is
/// uncertain.
pub(super) async fn repair_dangling_tool_calls(
    messages: &[RuntimeMessage],
    durable_store: Option<&dyn DurableToolResultStore>,
    event_emitter: &dyn EventEmitter,
    session_id: SessionId,
    event_context: &EventContext,
    turn_id: &str,
) -> Vec<RuntimeMessage> {
    let mut result = Vec::new();

    for (index, message) in messages.iter().enumerate() {
        result.push(message.clone());

        if message.role != RuntimeMessageRole::Agent || !message.has_tool_calls() {
            continue;
        }

        for tool_call in message.tool_calls() {
            let has_result = messages[(index + 1)..].iter().any(|candidate| {
                candidate.role == RuntimeMessageRole::ToolResult
                    && candidate.tool_call_id() == Some(&tool_call.id)
            });
            if has_result {
                continue;
            }

            let (repair_message, action) = if let Some(store) = durable_store {
                match store.get_tool_call_status(turn_id, &tool_call.id).await {
                    Ok(Some(DurableToolCallStatus::Settled { result_json })) => {
                        let repair = match serde_json::from_value::<
                            crate::engine::tool_types::ToolResult,
                        >(result_json.clone())
                        {
                            Ok(tool_result) => RuntimeMessage::tool_result(
                                &tool_call.id,
                                tool_result.result,
                                tool_result.error,
                            ),
                            Err(_) => {
                                RuntimeMessage::tool_result(&tool_call.id, Some(result_json), None)
                            }
                        };
                        (repair, TranscriptRepairAction::Replay)
                    }
                    Ok(Some(DurableToolCallStatus::Interrupted { result_json })) => {
                        let error = result_json
                            .as_ref()
                            .and_then(|value| {
                                serde_json::from_value::<crate::engine::tool_types::ToolResult>(
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
                            RuntimeMessage::tool_result(&tool_call.id, None, Some(error)),
                            TranscriptRepairAction::Replay,
                        )
                    }
                    Ok(Some(DurableToolCallStatus::Running)) => (
                        RuntimeMessage::tool_result(
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
                        RuntimeMessage::tool_result(
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
                            RuntimeMessage::tool_result(
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
                    RuntimeMessage::tool_result(
                        &tool_call.id,
                        None,
                        Some(everruns_contracts::runtime::UNKNOWN_TOOL_OUTCOME.to_string()),
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

    fn labeled(msg: &RuntimeMessage) -> String {
        let mut llm = crate::driver_registry::Message::text(
            crate::driver_registry::MessageRole::User,
            msg.text().unwrap_or_default().to_string(),
        );
        label_speaker(&mut llm, msg);
        llm.content_as_text()
    }

    fn reaction_note() -> everruns_contracts::message_reactions::ReactionNote {
        everruns_contracts::message_reactions::ReactionNote {
            through_sequence: 9,
            items: vec![everruns_contracts::message_reactions::ReactionNoteItem {
                message_id: "message_1".into(),
                emoji: "👍".into(),
                actor_name: "Mykhailo".into(),
                removed: false,
                on_agent_message: true,
                excerpt: Some("Deployed.".into()),
            }],
        }
    }

    #[test]
    fn waiting_reactions_come_before_the_message_text() {
        let mut msg = RuntimeMessage::user("Post the release card too.");
        let mut metadata = std::collections::HashMap::new();
        reaction_note().write(&mut metadata);
        msg.metadata = Some(metadata);
        assert_eq!(
            labeled(&msg),
            "[Reactions, not typed messages]\nMykhailo reacted 👍 on your reply \"Deployed.\" (message_1)\n\nPost the release card too."
        );
    }

    #[test]
    fn a_reaction_message_is_the_note_alone() {
        let note = reaction_note();
        let mut msg = RuntimeMessage::user(note.render());
        let mut metadata = everruns_contracts::message_origin::MessageOrigin::new(
            everruns_contracts::message_origin::MessageOriginKind::Reaction,
        )
        .into_metadata();
        note.write(&mut metadata);
        msg.metadata = Some(metadata);
        assert_eq!(labeled(&msg), note.render());
    }
    #[test]
    fn early_native_result_replays_after_its_call_without_synthetic_failure() {
        let native = everruns_contracts::native_async::NativeToolCall::Function {
            call_id: "original".into(),
            name: "lookup".into(),
            arguments: "{}".into(),
            asynchronous: true,
        };
        let mut call = RuntimeMessage::assistant("Independent reasoning");
        call.content
            .push(crate::engine::message::ContentPart::ToolCall(
                crate::engine::message::ToolCallContentPart::from_native(native).unwrap(),
            ));
        let output =
            RuntimeMessage::tool_result("original", Some(serde_json::json!({"answer":42})), None);
        let ordered = order_native_results(vec![
            RuntimeMessage::user("lookup"),
            output.clone(),
            call.clone(),
            RuntimeMessage::assistant("Done"),
        ]);
        assert_eq!(ordered[1].id, call.id);
        assert_eq!(ordered[1].content, call.content);
        assert_eq!(ordered[2].id, output.id);
        assert_eq!(ordered[2].content, output.content);
        assert_eq!(ordered.len(), 4);
    }

    fn agent_calling(names: &[&str]) -> RuntimeMessage {
        let mut message = RuntimeMessage::assistant("notes");
        for (index, name) in names.iter().enumerate() {
            message
                .content
                .push(crate::engine::message::ContentPart::ToolCall(
                    crate::engine::message::ToolCallContentPart {
                        native: None,
                        id: format!("call_{index}"),
                        name: (*name).to_string(),
                        arguments: serde_json::json!({}),
                    },
                ));
        }
        message
    }

    fn ok(id: &str) -> RuntimeMessage {
        RuntimeMessage::tool_result(id, Some(serde_json::json!({"replied": false})), None)
    }

    #[test]
    fn no_reply_ends_the_turn() {
        let messages = vec![
            RuntimeMessage::user("thanks"),
            agent_calling(&["no_reply"]),
            ok("call_0"),
        ];
        assert!(ended_by_no_reply(&messages));
    }

    #[test]
    fn a_message_after_no_reply_is_answered() {
        let messages = vec![
            RuntimeMessage::user("hi"),
            agent_calling(&["no_reply"]),
            ok("call_0"),
            RuntimeMessage::user("who does dishes on Nov 23?"),
        ];
        assert!(!ended_by_no_reply(&messages));
    }

    #[test]
    fn other_tools_or_failures_keep_the_turn_going() {
        let mixed = vec![
            RuntimeMessage::user("hi"),
            agent_calling(&["send_message", "read_file"]),
            ok("call_0"),
            ok("call_1"),
        ];
        assert!(!ended_by_no_reply(&mixed));
        assert!(!ended_by_communication(&mixed));

        let failed = vec![
            RuntimeMessage::user("hi"),
            agent_calling(&["no_reply"]),
            RuntimeMessage::tool_result("call_0", None, Some("bad arguments".into())),
        ];
        assert!(!ended_by_no_reply(&failed));

        let pending = vec![RuntimeMessage::user("hi"), agent_calling(&["no_reply"])];
        assert!(!ended_by_no_reply(&pending));

        assert!(!ended_by_no_reply(&[RuntimeMessage::user("hi")]));
    }

    #[test]
    fn send_message_ends_the_turn_and_other_tools_do_not() {
        let sent = vec![
            RuntimeMessage::user("what is this?"),
            agent_calling(&["send_message"]),
            ok("call_0"),
        ];
        assert!(!ended_by_no_reply(&sent));
        assert!(ended_by_communication(&sent));

        let both = vec![
            RuntimeMessage::user("hi"),
            agent_calling(&["send_message", "no_reply"]),
            ok("call_0"),
            ok("call_1"),
        ];
        assert!(ended_by_communication(&both));

        let continued = vec![
            RuntimeMessage::user("look this up"),
            agent_calling(&["send_message", "read_file"]),
            ok("call_0"),
            ok("call_1"),
        ];
        assert!(!ended_by_communication(&continued));
    }
}
