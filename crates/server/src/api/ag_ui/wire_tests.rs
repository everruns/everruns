//! AG-UI 1.0 wire-contract tests: serialized event shapes, reasoning-channel
//! sequence well-formedness, and 1.0 `RunAgentInput` acceptance (EVE-1135).

use super::tests::test_stream_state;
use super::*;
use crate::kernel_imports::{
    Event, EventContext, MessageId, RuntimeMessage, SessionId, ToolCall, ToolCompletedData,
    ToolStartedData, TurnId,
};
use everruns_ag_ui::{ContentPart, TextPart};
use everruns_core::events::ReasonItemData;
use serde_json::json;
use std::collections::HashSet;

/// Replays a stream the way the `@ag-ui/client` 1.0 verifier does for the
/// reasoning and text channels: content/end must name an open id, a start may
/// not reuse an open id, and `RUN_FINISHED` is rejected while anything is open.
fn assert_well_formed(events: &VecDeque<AgUiEvent>) {
    let mut spans = HashSet::new();
    let mut reasoning = HashSet::new();
    let mut text = HashSet::new();
    for event in events {
        match event {
            AgUiEvent::ReasoningStart(e) => assert!(spans.insert(e.message_id.clone())),
            AgUiEvent::ReasoningEnd(e) => assert!(spans.remove(&e.message_id), "{event:?}"),
            AgUiEvent::ReasoningMessageStart(e) => {
                assert!(reasoning.insert(e.message_id.clone()))
            }
            AgUiEvent::ReasoningMessageContent(e) => {
                assert!(reasoning.contains(&e.message_id), "{event:?}")
            }
            AgUiEvent::ReasoningMessageEnd(e) => {
                assert!(reasoning.remove(&e.message_id), "{event:?}")
            }
            AgUiEvent::TextMessageStart(e) => assert!(text.insert(e.message_id.clone())),
            AgUiEvent::TextMessageContent(e) => assert!(text.contains(&e.message_id)),
            AgUiEvent::TextMessageEnd(e) => assert!(text.remove(&e.message_id)),
            AgUiEvent::RunFinished(_) => {
                assert!(spans.is_empty(), "reasoning span open at RUN_FINISHED");
                assert!(
                    reasoning.is_empty(),
                    "reasoning message open at RUN_FINISHED"
                );
                assert!(text.is_empty(), "text message open at RUN_FINISHED");
            }
            _ => {}
        }
    }
}

struct Feed {
    session_id: SessionId,
    context: EventContext,
    turn_id: TurnId,
}

impl Feed {
    fn new(state: &AgUiStreamState) -> Self {
        let turn_id = TurnId::new();
        let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
        Self {
            session_id: SessionId::from_uuid(state.session_id),
            context: EventContext::turn(turn_id, input_message_id),
            turn_id,
        }
    }

    fn send(&self, state: &mut AgUiStreamState, data: impl Into<everruns_core::events::EventData>) {
        translate_event(
            state,
            &Event::new(self.session_id, self.context.clone(), data),
        );
    }

    fn thinking_started(&self, state: &mut AgUiStreamState) {
        self.send(
            state,
            ReasonThinkingStartedData {
                turn_id: self.turn_id,
                model: None,
            },
        );
    }

    fn thinking_delta(&self, state: &mut AgUiStreamState, delta: &str) {
        self.send(
            state,
            ReasonThinkingDeltaData {
                turn_id: self.turn_id,
                delta: delta.to_string(),
                accumulated: delta.to_string(),
            },
        );
    }

    fn thinking_completed(&self, state: &mut AgUiStreamState) {
        self.send(
            state,
            ReasonThinkingCompletedData {
                turn_id: self.turn_id,
                thinking: String::new(),
            },
        );
    }

    fn tool_started(&self, state: &mut AgUiStreamState, id: &str) {
        self.send(
            state,
            ToolStartedData {
                tool_call: ToolCall {
                    id: id.to_string(),
                    name: "web_search".to_string(),
                    arguments: json!({}),
                },
                tool_call_fingerprint: None,
                display_name: None,
                narration: None,
            },
        );
    }

    fn tool_completed(&self, state: &mut AgUiStreamState, id: &str) {
        self.send(
            state,
            ToolCompletedData {
                tool_call_id: id.to_string(),
                tool_name: "web_search".to_string(),
                tool_call_fingerprint: None,
                tool_result_fingerprint: None,
                display_name: None,
                success: true,
                status: "success".to_string(),
                result: None,
                error: None,
                duration_ms: None,
                capability_id: None,
                capability_name: None,
                narration: None,
            },
        );
    }

    fn summary(&self, state: &mut AgUiStreamState, text: &str) {
        self.send(
            state,
            ReasonItemData {
                turn_id: self.turn_id,
                provider: "openai".to_string(),
                model: None,
                item_id: "rs_1".to_string(),
                summary: vec![text.to_string()],
                token_count: None,
            },
        );
    }

    fn answer(&self, state: &mut AgUiStreamState, text: &str) {
        self.send(
            state,
            OutputMessageCompletedData::new(RuntimeMessage::assistant(text)),
        );
    }

    fn delta(&self, state: &mut AgUiStreamState, message_id: MessageId, delta: &str) {
        self.send(
            state,
            OutputMessageDeltaData {
                turn_id: self.turn_id,
                message_id,
                delta: delta.to_string(),
                accumulated: delta.to_string(),
                phase: None,
            },
        );
    }

    /// Any terminal turn event takes the same `finish_run` path.
    fn turn_cancelled(&self, state: &mut AgUiStreamState) {
        self.send(
            state,
            everruns_core::TurnCancelledData {
                turn_id: self.turn_id,
                reason: None,
                usage: None,
            },
        );
    }
}

#[tokio::test]
async fn reasoning_events_serialize_with_1_0_shape() {
    let mut state = test_stream_state().await;
    state.reasoning_summary_visible = true;
    let feed = Feed::new(&state);
    feed.thinking_started(&mut state);
    feed.thinking_delta(&mut state, "planning");
    feed.thinking_completed(&mut state);

    let wire: Vec<Value> = state
        .queue
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .collect();
    let types: Vec<&str> = wire.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(
        types,
        [
            "REASONING_START",
            "REASONING_MESSAGE_START",
            "REASONING_MESSAGE_CONTENT",
            "REASONING_MESSAGE_END",
            "REASONING_END",
        ]
    );
    let span_id = wire[0]["messageId"].as_str().unwrap();
    let message_id = wire[1]["messageId"].as_str().unwrap();
    assert_ne!(span_id, message_id, "span and message ids are distinct");
    assert_eq!(wire[1]["role"], "reasoning");
    assert_eq!(
        wire[2],
        json!({"type": "REASONING_MESSAGE_CONTENT", "messageId": message_id, "delta": "planning"})
    );
    assert_eq!(
        wire[3],
        json!({"type": "REASONING_MESSAGE_END", "messageId": message_id})
    );
    assert_eq!(
        wire[4],
        json!({"type": "REASONING_END", "messageId": span_id})
    );
    let serialized = serde_json::to_string(&wire).unwrap();
    assert!(!serialized.contains("THINKING"), "{serialized}");
    assert!(
        !serialized.contains("null"),
        "1.0 rejects null optionals: {serialized}"
    );
}

#[tokio::test]
async fn run_finished_serializes_with_1_0_shape() {
    let mut state = test_stream_state().await;
    let feed = Feed::new(&state);
    feed.turn_cancelled(&mut state);
    assert_eq!(
        serde_json::to_value(state.queue.back().unwrap()).unwrap(),
        json!({
            "type": "RUN_FINISHED",
            "threadId": state.thread_id.to_string(),
            "runId": state.run_id.to_string(),
        })
    );
}

#[tokio::test]
async fn interleaved_reasoning_sources_stay_well_formed() {
    let mut state = test_stream_state().await;
    state.reasoning_summary_visible = true;
    let feed = Feed::new(&state);
    // Tool activity opens a span, thinking shares it, a summary joins the
    // open thinking message, a second tool batch runs after thinking closed
    // the shared span, then the answer finishes the run.
    feed.tool_started(&mut state, "call_1");
    feed.thinking_started(&mut state);
    feed.thinking_delta(&mut state, "a");
    feed.summary(&mut state, "summary");
    feed.tool_completed(&mut state, "call_1");
    feed.thinking_completed(&mut state);
    feed.tool_started(&mut state, "call_2");
    feed.tool_completed(&mut state, "call_2");
    feed.summary(&mut state, "standalone summary");
    feed.answer(&mut state, "done");

    assert_well_formed(&state.queue);
    assert!(state.finished);
    assert!(state.reasoning.is_idle());
    assert_eq!(
        state.queue.back().map(AgUiEvent::event_type),
        Some("RUN_FINISHED")
    );
}

#[tokio::test]
async fn run_finished_closes_open_reasoning_and_text() {
    let mut state = test_stream_state().await;
    state.reasoning_summary_visible = true;
    let feed = Feed::new(&state);
    // A thinking delta without `started`, a tool that never completes, and
    // streamed text that never completes, then the turn ends.
    feed.thinking_delta(&mut state, "orphan");
    feed.tool_started(&mut state, "call_1");
    feed.delta(&mut state, MessageId::new(), "partial");
    feed.turn_cancelled(&mut state);

    assert_well_formed(&state.queue);
    let types: Vec<&str> = state.queue.iter().map(AgUiEvent::event_type).collect();
    assert_eq!(types.first(), Some(&"REASONING_START"));
    assert_eq!(
        &types[types.len() - 4..],
        [
            "TEXT_MESSAGE_END",
            "REASONING_MESSAGE_END",
            "REASONING_END",
            "RUN_FINISHED",
        ]
    );
}

#[test]
fn accepts_minimal_1_0_run_agent_input() {
    let thread_id = Uuid::new_v4();
    let input: AgUiRunAgentInput = serde_json::from_value(json!({
        "threadId": thread_id.to_string(),
        "runId": Uuid::new_v4().to_string(),
        "protocolVersion": "1.0",
        "messages": [
            { "id": "u1", "role": "user", "content": "Hi" },
            { "id": "r1", "role": "reasoning", "content": "earlier reasoning", "encryptedValue": "x" },
            { "id": "a1", "role": "assistant", "content": "Hello",
              "toolCalls": [{ "id": "c1", "type": "function", "function": { "name": "f", "arguments": "{}" } }] },
            { "id": "act1", "role": "activity", "activityType": "progress", "content": {} },
            { "id": "u2", "role": "user", "content": [
                { "type": "text", "text": "Next " },
                { "type": "text", "text": "question" }
            ], "metadata": { "k": 1 } }
        ]
    }))
    .unwrap();
    assert_eq!(input.thread_id, thread_id.to_string());
    assert_eq!(input.protocol_version.as_deref(), Some("1.0"));
    assert!(input.forwarded_props.is_none());
    validate_input_messages(&input.messages).unwrap();

    match input.messages.last().unwrap() {
        AgUiMessage::User(message) => {
            assert_eq!(user_text(&message.content).unwrap(), "Next question")
        }
        other => panic!("expected user message, got {other:?}"),
    }
    // Client-materialised reasoning/activity never reach the model.
    let seeded: Vec<_> = input
        .messages
        .iter()
        .filter_map(to_stored_history_message)
        .collect();
    assert_eq!(seeded.len(), 3);
}

#[test]
fn rejects_media_content_parts() {
    let content: AgUiMessageContent = serde_json::from_value(json!([
        { "type": "text", "text": "look" },
        { "type": "image", "source": { "type": "data", "value": "AAAA", "mimeType": "image/png" } }
    ]))
    .unwrap();
    assert!(user_text(&content).is_err());
    let text_only = AgUiMessageContent::Parts(vec![ContentPart::Text(TextPart {
        text: "look".to_string(),
        ..TextPart::default()
    })]);
    assert_eq!(user_text(&text_only).unwrap(), "look");

    let messages: Vec<AgUiMessage> = serde_json::from_value(json!([{
        "id": "u1",
        "role": "user",
        "content": [{ "type": "image", "source": { "type": "url", "value": "https://example.com/a.png" } }]
    }]))
    .unwrap();
    assert!(validate_input_messages(&messages).is_err());
}

#[test]
fn rejects_duplicate_ids_across_ignored_roles() {
    let messages: Vec<AgUiMessage> = serde_json::from_value(json!([
        { "id": "same", "role": "reasoning", "content": "r" },
        { "id": "same", "role": "user", "content": "u" }
    ]))
    .unwrap();
    assert!(validate_input_messages(&messages).is_err());
}

#[test]
fn rejects_non_uuid_thread_and_run_ids() {
    assert!(parse_ag_ui_uuid("thread-1", "threadId").is_err());
    let id = Uuid::new_v4();
    assert_eq!(parse_ag_ui_uuid(&id.to_string(), "runId").unwrap(), id);
}
