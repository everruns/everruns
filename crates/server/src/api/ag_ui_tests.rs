//! Unit tests for [`super`], the AG-UI event translation surface.
//!
//! Split out of `ag_ui.rs` to keep it under the file-size ratchet.

use super::*;
use crate::kernel_imports::{
    Event, EventContext, MessageId, OutputMessageCompletedData, OutputMessageDeltaData,
    RuntimeMessage, SessionId, ToolCall, ToolCompletedData, ToolStartedData, TurnId,
};
use ag_ui_core::event::EventType as AgUiEventType;
use chrono::Duration as ChronoDuration;
use everruns_core::events::TurnFailedData;
use everruns_provider::execution_phase::ExecutionPhase;

#[test]
fn test_expired_age_seconds_within_window() {
    let now = chrono::Utc::now();
    let created = now - ChronoDuration::seconds(100);
    assert_eq!(expired_age_seconds(created, 200, now), None);
}

#[test]
fn test_expired_age_seconds_past_window() {
    let now = chrono::Utc::now();
    let created = now - ChronoDuration::seconds(300);
    assert_eq!(expired_age_seconds(created, 200, now), Some(300));
}

#[test]
fn test_expired_age_seconds_zero_disables_expiration() {
    let now = chrono::Utc::now();
    let created = now - ChronoDuration::days(30);
    assert_eq!(expired_age_seconds(created, 0, now), None);
}

#[test]
fn test_expired_age_seconds_clock_skew() {
    let now = chrono::Utc::now();
    let created = now + ChronoDuration::seconds(5);
    assert_eq!(expired_age_seconds(created, 60, now), None);
}

async fn test_stream_state() -> AgUiStreamState {
    let session_id = SessionId::new();
    let delivery = crate::event_delivery::EventDelivery::in_memory();
    let subscription = delivery.subscribe(session_id.uuid()).await.unwrap();
    AgUiStreamState {
        subscription: Box::new(subscription),
        session_id: session_id.uuid(),
        input_message_id: MessageId::new().to_string(),
        thread_id: AgUiThreadId::random(),
        run_id: AgUiRunId::random(),
        queue: VecDeque::new(),
        assistant_message_id: None,
        assistant_content_started: false,
        assistant_emitted_delta: false,
        thinking_started: false,
        thinking_text_started: false,
        tool_visibility: PublicToolVisibility::Generic,
        generic_tool_text: "Working...".to_string(),
        reasoning_summary_visible: false,
        active_tool_activity_count: 0,
        public_tool_activity_started: false,
        public_tool_activity_opened_thinking: false,
        finished: false,
    }
}

fn test_app() -> crate::api::app_ingress::IngressContext {
    crate::api::app_ingress::IngressContext::for_test("Test App", None)
}

#[test]
fn ag_ui_message_metadata_includes_system_app_id() {
    let app = test_app();
    let metadata = ag_ui_message_metadata(&app, "thread-1".to_string(), "run-1".to_string());

    assert_eq!(
        metadata.get("_app_id"),
        Some(&Value::String(app.public_id.to_string()))
    );
    assert_eq!(
        metadata.get("ag_ui_thread_id"),
        Some(&Value::String("thread-1".to_string()))
    );
}

#[test]
fn ag_ui_image_metadata_scopes_upload_to_app() {
    let app = test_app();
    let metadata = ag_ui_image_metadata(&app);
    let app_public_id = app.public_id.to_string();

    assert!(is_ag_ui_app_image(&metadata, &app_public_id));
    assert!(!is_ag_ui_app_image(
        &serde_json::json!({"source": "ag_ui"}),
        &app_public_id
    ));
}

#[test]
fn forwarded_ag_ui_image_ids_parse_camel_case_ids() {
    let image_id = ImageId::from_seed(42);
    let props = serde_json::json!({ "imageIds": [image_id.to_string()] });

    assert_eq!(forwarded_ag_ui_image_ids(&props).unwrap(), vec![image_id]);
}

#[test]
fn forwarded_ag_ui_image_ids_reject_bad_shape() {
    assert!(forwarded_ag_ui_image_ids(&serde_json::json!({ "imageIds": "img_bad" })).is_err());
    assert!(forwarded_ag_ui_image_ids(&serde_json::json!({ "imageIds": [42] })).is_err());
}

#[tokio::test]
async fn test_translate_output_completion_to_ag_ui_run_finished() {
    let mut state = test_stream_state().await;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let output_message_id = MessageId::new();
    let event = Event::new(
        SessionId::from_uuid(state.session_id),
        EventContext::turn(turn_id, input_message_id),
        OutputMessageCompletedData::new(
            RuntimeMessage::assistant("Hello from AG-UI").with_id(output_message_id),
        ),
    );

    translate_event(&mut state, &event);

    let event_types: Vec<AgUiEventType> = state.queue.iter().map(AgUiEvent::event_type).collect();
    assert_eq!(
        event_types,
        vec![
            AgUiEventType::TextMessageStart,
            AgUiEventType::TextMessageContent,
            AgUiEventType::TextMessageEnd,
            AgUiEventType::RunFinished,
        ]
    );
    match &state.queue[0] {
        AgUiEvent::TextMessageStart(event) => assert_eq!(
            event.message_id,
            AgUiMessageId::from(output_message_id.uuid())
        ),
        _ => panic!("expected text start event"),
    }
    assert!(state.finished);
}

#[tokio::test]
async fn test_translate_streaming_delta_does_not_duplicate_final_content() {
    let mut state = test_stream_state().await;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);
    let streamed_message_id = MessageId::new();

    let delta_event = Event::new(
        session_id,
        context.clone(),
        OutputMessageDeltaData {
            turn_id,
            message_id: streamed_message_id,
            delta: "Hello".to_string(),
            accumulated: "Hello".to_string(),
            phase: None,
        },
    );
    translate_event(&mut state, &delta_event);

    let completed_event = Event::new(
        session_id,
        context,
        OutputMessageCompletedData::new(
            RuntimeMessage::assistant("Hello from AG-UI").with_id(streamed_message_id),
        ),
    );
    translate_event(&mut state, &completed_event);

    let event_types: Vec<AgUiEventType> = state.queue.iter().map(AgUiEvent::event_type).collect();
    assert_eq!(
        event_types,
        vec![
            AgUiEventType::TextMessageStart,
            AgUiEventType::TextMessageContent,
            AgUiEventType::TextMessageEnd,
            AgUiEventType::RunFinished,
        ]
    );
    let content_events: Vec<&AgUiEvent> = state
        .queue
        .iter()
        .filter(|event| matches!(event, AgUiEvent::TextMessageContent(_)))
        .collect();
    assert_eq!(content_events.len(), 1);
    match content_events[0] {
        AgUiEvent::TextMessageContent(event) => assert_eq!(event.delta, "Hello"),
        _ => unreachable!(),
    }
    assert!(state.finished);
}

#[tokio::test]
async fn test_commentary_delta_tool_completion_does_not_hide_final_answer() {
    let mut state = test_stream_state().await;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);
    let commentary_message_id = MessageId::new();
    let final_message_id = MessageId::new();

    let delta_event = Event::new(
        session_id,
        context.clone(),
        OutputMessageDeltaData {
            turn_id,
            message_id: commentary_message_id,
            delta: "Let me check that.".to_string(),
            accumulated: "Let me check that.".to_string(),
            phase: None,
        },
    );
    translate_event(&mut state, &delta_event);

    let commentary_completed = Event::new(
        session_id,
        context.clone(),
        OutputMessageCompletedData::new(
            RuntimeMessage::assistant_with_tools(
                "",
                vec![ToolCall {
                    id: "call_lookup".to_string(),
                    name: "lookup".to_string(),
                    arguments: serde_json::json!({"query": "base harness"}),
                }],
            )
            .with_id(commentary_message_id)
            .with_phase(ExecutionPhase::Commentary),
        ),
    );
    translate_event(&mut state, &commentary_completed);

    let event_types: Vec<AgUiEventType> = state.queue.iter().map(AgUiEvent::event_type).collect();
    assert_eq!(
        event_types,
        vec![
            AgUiEventType::TextMessageStart,
            AgUiEventType::TextMessageContent,
            AgUiEventType::TextMessageEnd,
        ]
    );
    assert!(!state.finished);

    let final_completed = Event::new(
        session_id,
        context,
        OutputMessageCompletedData::new(
            RuntimeMessage::assistant("The Base harness is the default execution wrapper.")
                .with_id(final_message_id)
                .with_phase(ExecutionPhase::FinalAnswer),
        ),
    );
    translate_event(&mut state, &final_completed);

    let event_types: Vec<AgUiEventType> = state.queue.iter().map(AgUiEvent::event_type).collect();
    assert_eq!(
        event_types,
        vec![
            AgUiEventType::TextMessageStart,
            AgUiEventType::TextMessageContent,
            AgUiEventType::TextMessageEnd,
            AgUiEventType::TextMessageStart,
            AgUiEventType::TextMessageContent,
            AgUiEventType::TextMessageEnd,
            AgUiEventType::RunFinished,
        ]
    );
    let content_events: Vec<&AgUiTextMessageContentEvent> = state
        .queue
        .iter()
        .filter_map(|event| match event {
            AgUiEvent::TextMessageContent(event) => Some(event),
            _ => None,
        })
        .collect();
    assert_eq!(content_events.len(), 2);
    assert_eq!(content_events[0].delta, "Let me check that.");
    assert_eq!(
        content_events[1].delta,
        "The Base harness is the default execution wrapper."
    );
    assert!(state.finished);

    // EVE-773: AG-UI projects the canonical streamed message ids, preserving
    // the commentary/final boundary without exposing the turn uuid.
    let start_ids: Vec<&AgUiMessageId> = state
        .queue
        .iter()
        .filter_map(|event| match event {
            AgUiEvent::TextMessageStart(event) => Some(&event.message_id),
            _ => None,
        })
        .collect();
    assert_eq!(start_ids.len(), 2);
    assert_ne!(
        start_ids[0], start_ids[1],
        "commentary and final answer must have distinct messageIds"
    );
    assert_eq!(
        *start_ids[0],
        AgUiMessageId::from(commentary_message_id.uuid())
    );
    assert_eq!(*start_ids[1], AgUiMessageId::from(final_message_id.uuid()));
    let turn_message_id = AgUiMessageId::from(turn_id.uuid());
    assert_ne!(*start_ids[0], turn_message_id);
    assert_ne!(*start_ids[1], turn_message_id);
}

#[tokio::test]
async fn test_reason_item_summary_hidden_by_default() {
    let mut state = test_stream_state().await;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let event = Event::new(
        SessionId::from_uuid(state.session_id),
        EventContext::turn(turn_id, input_message_id),
        ReasonItemData {
            turn_id,
            provider: "openai".to_string(),
            model: Some("gpt-5.5".to_string()),
            item_id: "rs_private".to_string(),
            summary: vec!["Looked up private CRM details.".to_string()],
            token_count: Some(42),
        },
    );

    translate_event(&mut state, &event);

    assert!(state.queue.is_empty());
    assert!(!state.finished);
}

#[tokio::test]
async fn test_thinking_stream_hidden_by_default() {
    let mut state = test_stream_state().await;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);

    translate_event(
        &mut state,
        &Event::new(
            session_id,
            context.clone(),
            ReasonThinkingStartedData {
                turn_id,
                model: Some("private-model".to_string()),
            },
        ),
    );
    translate_event(
        &mut state,
        &Event::new(
            session_id,
            context.clone(),
            ReasonThinkingDeltaData {
                turn_id,
                delta: "Private reasoning".to_string(),
                accumulated: "Private reasoning".to_string(),
            },
        ),
    );
    translate_event(
        &mut state,
        &Event::new(
            session_id,
            context,
            ReasonThinkingCompletedData {
                turn_id,
                thinking: "Private reasoning".to_string(),
            },
        ),
    );

    assert!(state.queue.is_empty());
    assert!(!state.thinking_started);
    assert!(!state.thinking_text_started);
}

// EVE-775: a provider `reason.item` summary is a reasoning artifact and must
// project onto the AG-UI reasoning channel (THINKING_*), never the
// assistant-text channel, and never leak opaque reasoning content.
#[tokio::test]
async fn test_reason_item_summary_projects_to_reasoning_channel_when_enabled() {
    let mut state = test_stream_state().await;
    state.reasoning_summary_visible = true;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let event = Event::new(
        SessionId::from_uuid(state.session_id),
        EventContext::turn(turn_id, input_message_id),
        ReasonItemData {
            turn_id,
            provider: "openai".to_string(),
            model: Some("gpt-5.5".to_string()),
            item_id: "rs_1".to_string(),
            summary: vec!["Considered the file layout.".to_string()],
            token_count: Some(42),
        },
    );

    translate_event(&mut state, &event);

    let event_types: Vec<AgUiEventType> = state.queue.iter().map(AgUiEvent::event_type).collect();
    assert_eq!(
        event_types,
        vec![
            AgUiEventType::ThinkingStart,
            AgUiEventType::ThinkingTextMessageStart,
            AgUiEventType::ThinkingTextMessageContent,
            AgUiEventType::ThinkingTextMessageEnd,
            AgUiEventType::ThinkingEnd,
        ]
    );
    // The reasoning summary must never appear on the assistant-text channel.
    assert!(!state.queue.iter().any(|event| matches!(
        event,
        AgUiEvent::TextMessageStart(_)
            | AgUiEvent::TextMessageContent(_)
            | AgUiEvent::TextMessageEnd(_)
    )));
    match &state.queue[2] {
        AgUiEvent::ThinkingTextMessageContent(event) => {
            assert_eq!(event.delta, "Considered the file layout.");
        }
        other => panic!("expected thinking content, got {other:?}"),
    }
    // Hidden/opaque reasoning content is never exposed anywhere on the wire.
    let serialized: String = state
        .queue
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect();
    assert!(
        !serialized.contains("OPAQUE-DO-NOT-LEAK"),
        "opaque reasoning must never reach the AG-UI wire: {serialized}"
    );
    // A reasoning summary does not end the run — the final answer still follows.
    assert!(!state.finished);
}

#[tokio::test]
async fn test_reason_item_empty_summary_emits_nothing() {
    let mut state = test_stream_state().await;
    state.reasoning_summary_visible = true;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let event = Event::new(
        SessionId::from_uuid(state.session_id),
        EventContext::turn(turn_id, input_message_id),
        ReasonItemData {
            turn_id,
            provider: "openai".to_string(),
            model: None,
            item_id: "rs_empty".to_string(),
            summary: vec![String::new(), "   ".to_string()],
            token_count: None,
        },
    );
    translate_event(&mut state, &event);
    assert!(state.queue.is_empty());
}

// EVE-768 AC#7: reasoning summary vs commentary channel separation. Commentary
// is deliberate user-facing assistant text; the provider reasoning summary is a
// reasoning artifact. They must land on different channels and never mix.
#[tokio::test]
async fn test_reason_summary_and_commentary_use_separate_channels() {
    let mut state = test_stream_state().await;
    state.reasoning_summary_visible = true;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);

    let commentary_delta = Event::new(
        session_id,
        context.clone(),
        OutputMessageDeltaData {
            turn_id,
            message_id: MessageId::new(),
            delta: "Let me look into that.".to_string(),
            accumulated: "Let me look into that.".to_string(),
            phase: None,
        },
    );
    translate_event(&mut state, &commentary_delta);

    let reason_item = Event::new(
        session_id,
        context,
        ReasonItemData {
            turn_id,
            provider: "openai".to_string(),
            model: None,
            item_id: "rs_sep".to_string(),
            summary: vec!["Summarized the plan.".to_string()],
            token_count: None,
        },
    );
    translate_event(&mut state, &reason_item);

    // Assistant-text channel carries only the commentary.
    let text_deltas: Vec<&str> = state
        .queue
        .iter()
        .filter_map(|event| match event {
            AgUiEvent::TextMessageContent(event) => Some(event.delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_deltas, vec!["Let me look into that."]);

    // Reasoning channel carries only the summary.
    let thinking_deltas: Vec<&str> = state
        .queue
        .iter()
        .filter_map(|event| match event {
            AgUiEvent::ThinkingTextMessageContent(event) => Some(event.delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(thinking_deltas, vec!["Summarized the plan."]);
}

#[tokio::test]
async fn test_reason_item_summary_appends_to_active_thinking_block() {
    let mut state = test_stream_state().await;
    state.reasoning_summary_visible = true;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);

    let thinking_started = Event::new(
        session_id,
        context.clone(),
        ReasonThinkingStartedData {
            turn_id,
            model: None,
        },
    );
    translate_event(&mut state, &thinking_started);
    let thinking_delta = Event::new(
        session_id,
        context.clone(),
        ReasonThinkingDeltaData {
            turn_id,
            delta: "Reasoning...".to_string(),
            accumulated: "Reasoning...".to_string(),
        },
    );
    translate_event(&mut state, &thinking_delta);

    let reason_item = Event::new(
        session_id,
        context,
        ReasonItemData {
            turn_id,
            provider: "openai".to_string(),
            model: None,
            item_id: "rs_app".to_string(),
            summary: vec!["Summary tail.".to_string()],
            token_count: None,
        },
    );
    translate_event(&mut state, &reason_item);

    let event_types: Vec<AgUiEventType> = state.queue.iter().map(AgUiEvent::event_type).collect();
    // The summary appends into the open reasoning block — no duplicate start.
    assert_eq!(
        event_types
            .iter()
            .filter(|event_type| **event_type == AgUiEventType::ThinkingStart)
            .count(),
        1
    );
    assert!(
        !state
            .queue
            .iter()
            .any(|event| matches!(event, AgUiEvent::TextMessageContent(_)))
    );
    match state.queue.back().unwrap() {
        AgUiEvent::ThinkingTextMessageContent(event) => {
            assert_eq!(event.delta, "\nSummary tail.");
        }
        other => panic!("expected appended thinking content, got {other:?}"),
    }
}

#[tokio::test]
async fn test_tool_call_completion_does_not_finish_before_final_answer() {
    let mut state = test_stream_state().await;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);

    let commentary_completed = Event::new(
        session_id,
        context.clone(),
        OutputMessageCompletedData::new(
            RuntimeMessage::assistant_with_tools(
                "",
                vec![ToolCall {
                    id: "call_list_skills".to_string(),
                    name: "list_skills".to_string(),
                    arguments: serde_json::json!({"registry": "internal"}),
                }],
            )
            .with_phase(ExecutionPhase::Commentary),
        ),
    );
    translate_event(&mut state, &commentary_completed);
    assert!(state.queue.is_empty());
    assert!(!state.finished);

    let tool_started = Event::new(
        session_id,
        context.clone(),
        ToolStartedData {
            tool_call: ToolCall {
                id: "call_list_skills".to_string(),
                name: "list_skills".to_string(),
                arguments: serde_json::json!({"registry": "internal"}),
            },
            tool_call_fingerprint: None,
            display_name: None,
            narration: Some("Listing internal skills".to_string()),
        },
    );
    translate_event(&mut state, &tool_started);

    let tool_completed = Event::new(
        session_id,
        context.clone(),
        ToolCompletedData {
            tool_call_id: "call_list_skills".to_string(),
            tool_name: "list_skills".to_string(),
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: Some(vec![ContentPart::text("skill_123")]),
            error: None,
            duration_ms: Some(10),
            capability_id: None,
            capability_name: None,
            narration: None,
        },
    );
    translate_event(&mut state, &tool_completed);
    assert!(!state.finished);

    let final_completed = Event::new(
        session_id,
        context,
        OutputMessageCompletedData::new(
            RuntimeMessage::assistant("The Base harness is the default execution wrapper.")
                .with_phase(ExecutionPhase::FinalAnswer),
        ),
    );
    translate_event(&mut state, &final_completed);

    let event_types: Vec<AgUiEventType> = state.queue.iter().map(AgUiEvent::event_type).collect();
    assert_eq!(
        event_types,
        vec![
            AgUiEventType::ThinkingStart,
            AgUiEventType::ThinkingTextMessageStart,
            AgUiEventType::ThinkingTextMessageContent,
            AgUiEventType::ThinkingTextMessageEnd,
            AgUiEventType::ThinkingEnd,
            AgUiEventType::TextMessageStart,
            AgUiEventType::TextMessageContent,
            AgUiEventType::TextMessageEnd,
            AgUiEventType::RunFinished,
        ]
    );
    match &state.queue[2] {
        AgUiEvent::ThinkingTextMessageContent(event) => {
            assert_eq!(event.delta, "Working...");
            assert!(!event.delta.contains("list_skills"));
            assert!(!event.delta.contains("internal"));
            assert!(!event.delta.contains("skill_123"));
        }
        _ => panic!("expected generic thinking content"),
    }
    match &state.queue[6] {
        AgUiEvent::TextMessageContent(event) => {
            assert_eq!(
                event.delta,
                "The Base harness is the default execution wrapper."
            );
        }
        _ => panic!("expected final answer text content"),
    }
    assert!(state.finished);
}

#[tokio::test]
async fn test_generic_tool_activity_hides_public_tool_details() {
    let mut state = test_stream_state().await;
    state.generic_tool_text = "Checking now".to_string();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);

    let tool_started = Event::new(
        session_id,
        context.clone(),
        ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "web_search".to_string(),
                arguments: serde_json::json!({"q": "hello"}),
            },
            tool_call_fingerprint: None,
            display_name: None,
            narration: Some("Searching for hello".to_string()),
        },
    );
    translate_event(&mut state, &tool_started);

    let tool_completed = Event::new(
        session_id,
        context.clone(),
        ToolCompletedData {
            tool_call_id: "call_1".to_string(),
            tool_name: "web_search".to_string(),
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: Some(vec![ContentPart::text("result")]),
            error: None,
            duration_ms: Some(10),
            capability_id: None,
            capability_name: None,
            narration: None,
        },
    );
    translate_event(&mut state, &tool_completed);

    let event_types: Vec<AgUiEventType> = state.queue.iter().map(AgUiEvent::event_type).collect();
    assert_eq!(
        event_types,
        vec![
            AgUiEventType::ThinkingStart,
            AgUiEventType::ThinkingTextMessageStart,
            AgUiEventType::ThinkingTextMessageContent,
            AgUiEventType::ThinkingTextMessageEnd,
            AgUiEventType::ThinkingEnd,
        ]
    );
    match &state.queue[2] {
        AgUiEvent::ThinkingTextMessageContent(event) => {
            assert_eq!(event.delta, "Checking now");
            assert!(!event.delta.contains("web_search"));
            assert!(!event.delta.contains("hello"));
            assert!(!event.delta.contains("result"));
        }
        _ => panic!("expected generic thinking content"),
    }

    let output_completed = Event::new(
        session_id,
        context,
        OutputMessageCompletedData::new(RuntimeMessage::assistant("Hello after tool")),
    );
    translate_event(&mut state, &output_completed);

    // The public messageId is message-scoped and must never be the raw turn uuid.
    let turn_message_id = AgUiMessageId::from(turn_id.uuid());
    match &state.queue[5] {
        AgUiEvent::TextMessageStart(event) => {
            assert_ne!(event.message_id, turn_message_id);
        }
        _ => panic!("expected text start event"),
    }
}

#[tokio::test]
async fn test_tool_activity_reuses_active_reason_thinking_block() {
    let mut state = test_stream_state().await;
    state.reasoning_summary_visible = true;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);

    let thinking_started = Event::new(
        session_id,
        context.clone(),
        ReasonThinkingStartedData {
            turn_id,
            model: Some("test-model".to_string()),
        },
    );
    translate_event(&mut state, &thinking_started);

    let thinking_delta = Event::new(
        session_id,
        context.clone(),
        ReasonThinkingDeltaData {
            turn_id,
            delta: "Thinking".to_string(),
            accumulated: "Thinking".to_string(),
        },
    );
    translate_event(&mut state, &thinking_delta);

    let tool_started = Event::new(
        session_id,
        context.clone(),
        ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "web_search".to_string(),
                arguments: serde_json::json!({"q": "private query"}),
            },
            tool_call_fingerprint: None,
            display_name: None,
            narration: None,
        },
    );
    translate_event(&mut state, &tool_started);

    let tool_completed = Event::new(
        session_id,
        context.clone(),
        ToolCompletedData {
            tool_call_id: "call_1".to_string(),
            tool_name: "web_search".to_string(),
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: Some(vec![ContentPart::text("private result")]),
            error: None,
            duration_ms: Some(10),
            capability_id: None,
            capability_name: None,
            narration: None,
        },
    );
    translate_event(&mut state, &tool_completed);

    let thinking_completed = Event::new(
        session_id,
        context,
        ReasonThinkingCompletedData {
            turn_id,
            thinking: "Thinking".to_string(),
        },
    );
    translate_event(&mut state, &thinking_completed);

    let event_types: Vec<AgUiEventType> = state.queue.iter().map(AgUiEvent::event_type).collect();
    assert_eq!(
        event_types,
        vec![
            AgUiEventType::ThinkingStart,
            AgUiEventType::ThinkingTextMessageStart,
            AgUiEventType::ThinkingTextMessageContent,
            AgUiEventType::ThinkingTextMessageContent,
            AgUiEventType::ThinkingTextMessageEnd,
            AgUiEventType::ThinkingEnd,
        ]
    );
    assert_eq!(
        event_types
            .iter()
            .filter(|event_type| **event_type == AgUiEventType::ThinkingStart)
            .count(),
        1
    );
    assert_eq!(
        event_types
            .iter()
            .filter(|event_type| **event_type == AgUiEventType::ThinkingEnd)
            .count(),
        1
    );
    match &state.queue[3] {
        AgUiEvent::ThinkingTextMessageContent(event) => {
            assert_eq!(event.delta, "\nWorking...");
            assert!(!event.delta.contains("web_search"));
            assert!(!event.delta.contains("private query"));
            assert!(!event.delta.contains("private result"));
        }
        _ => panic!("expected generic tool activity content"),
    }
}

#[tokio::test]
async fn test_none_tool_visibility_hides_public_tool_activity() {
    let mut state = test_stream_state().await;
    state.tool_visibility = PublicToolVisibility::None;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);

    let tool_started = Event::new(
        session_id,
        context.clone(),
        ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "web_search".to_string(),
                arguments: serde_json::json!({"q": "hello"}),
            },
            tool_call_fingerprint: None,
            display_name: None,
            narration: Some("Searching for hello".to_string()),
        },
    );
    translate_event(&mut state, &tool_started);

    let tool_completed = Event::new(
        session_id,
        context,
        ToolCompletedData {
            tool_call_id: "call_1".to_string(),
            tool_name: "web_search".to_string(),
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: Some(vec![ContentPart::text("result")]),
            error: None,
            duration_ms: Some(10),
            capability_id: None,
            capability_name: None,
            narration: None,
        },
    );
    translate_event(&mut state, &tool_completed);

    assert!(state.queue.is_empty());
}

#[tokio::test]
async fn test_narrated_tool_visibility_falls_back_to_generic_text() {
    let mut state = test_stream_state().await;
    state.tool_visibility = PublicToolVisibility::Narrated;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let context = EventContext::turn(turn_id, input_message_id);
    let session_id = SessionId::from_uuid(state.session_id);

    let tool_started = Event::new(
        session_id,
        context,
        ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "web_search".to_string(),
                arguments: serde_json::json!({"q": "private query"}),
            },
            tool_call_fingerprint: None,
            display_name: Some("Web Search".to_string()),
            narration: Some("Checking public sources".to_string()),
        },
    );
    translate_event(&mut state, &tool_started);

    match &state.queue[2] {
        AgUiEvent::ThinkingTextMessageContent(event) => {
            assert_eq!(event.delta, "Working...");
            assert!(!event.delta.contains("web_search"));
            assert!(!event.delta.contains("private query"));
        }
        _ => panic!("expected narrated thinking content"),
    }
}

#[tokio::test]
async fn test_public_tool_text_falls_back_when_configured_value_is_empty() {
    // Existing channels may have stored an empty/whitespace generic_tool_text from
    // before validation tightened. The public stream must never emit an empty delta.
    for visibility in [
        PublicToolVisibility::Generic,
        PublicToolVisibility::Narrated,
    ] {
        for configured in ["", "   ", "\n\t"] {
            let mut state = test_stream_state().await;
            state.tool_visibility = visibility;
            state.generic_tool_text = configured.to_string();
            let turn_id = TurnId::new();
            let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
            let context = EventContext::turn(turn_id, input_message_id);
            let session_id = SessionId::from_uuid(state.session_id);

            let tool_started = Event::new(
                session_id,
                context,
                ToolStartedData {
                    tool_call: ToolCall {
                        id: "call_1".to_string(),
                        name: "web_search".to_string(),
                        arguments: serde_json::json!({"q": "x"}),
                    },
                    tool_call_fingerprint: None,
                    display_name: None,
                    narration: Some("backend narration".to_string()),
                },
            );
            translate_event(&mut state, &tool_started);

            match &state.queue[2] {
                AgUiEvent::ThinkingTextMessageContent(event) => {
                    assert_eq!(
                        event.delta, "Working...",
                        "visibility={visibility:?} configured={configured:?}"
                    );
                }
                _ => panic!(
                    "expected non-empty thinking content for visibility {visibility:?} configured {configured:?}"
                ),
            }
        }
    }
}

fn turn_failed_event(state: &AgUiStreamState, data: TurnFailedData) -> Event {
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    Event::new(
        SessionId::from_uuid(state.session_id),
        EventContext::turn(data.turn_id, input_message_id),
        data,
    )
}

fn assert_run_error(state: &AgUiStreamState, expected_message: &str, expected_code: &str) {
    assert_eq!(state.queue.len(), 1);
    match state.queue.front() {
        Some(AgUiEvent::RunError(event)) => {
            assert_eq!(event.message, expected_message);
            assert_eq!(event.code.as_deref(), Some(expected_code));
        }
        other => panic!("expected RunError, got {other:?}"),
    }
    assert!(state.finished);
}

#[tokio::test]
async fn turn_failed_rate_limit_returns_generic_message() {
    let mut state = test_stream_state().await;
    let event = turn_failed_event(
        &state,
        TurnFailedData {
            turn_id: TurnId::new(),
            error: "OpenAI API error (429): {\"error\":{\"message\":\"Rate limit reached for gpt-4o in organization org-redacted on requests per min (RPM): Limit 500\",\"code\":\"rate_limit_exceeded\"}}".to_string(),
            error_code: Some(user_facing_error_codes::PROVIDER_RATE_LIMITED.to_string()),
            error_fields: None,
            error_disclosure: None,
        },
    );

    translate_event(&mut state, &event);

    assert_run_error(
        &state,
        "The service is busy right now. Please try again in a moment.",
        "rate_limited",
    );
}

#[tokio::test]
async fn turn_failed_provider_unavailable_returns_generic_message() {
    let mut state = test_stream_state().await;
    let event = turn_failed_event(
        &state,
        TurnFailedData {
            turn_id: TurnId::new(),
            error: "OpenAI API error (503): server overloaded".to_string(),
            error_code: Some(user_facing_error_codes::PROVIDER_UNAVAILABLE.to_string()),
            error_fields: None,
            error_disclosure: None,
        },
    );

    translate_event(&mut state, &event);

    assert_run_error(
        &state,
        "The service is temporarily unavailable. Please try again shortly.",
        "service_unavailable",
    );
}

#[tokio::test]
async fn turn_failed_zero_credits_does_not_leak_quota_details() {
    // OpenAI surfaces "no credits" as 429 + insufficient_quota; classifier
    // maps it to PROVIDER_MISCONFIGURED (operator must top up or rotate
    // the key). Either way, the public channel must not echo the raw
    // provider body.
    let mut state = test_stream_state().await;
    let raw_error = "OpenAI API error (429): {\"error\":{\"message\":\"You exceeded your current quota, please check your plan and billing details.\",\"type\":\"insufficient_quota\",\"code\":\"insufficient_quota\"}}";
    let event = turn_failed_event(
        &state,
        TurnFailedData {
            turn_id: TurnId::new(),
            error: raw_error.to_string(),
            error_code: Some(user_facing_error_codes::PROVIDER_MISCONFIGURED.to_string()),
            error_fields: None,
            error_disclosure: None,
        },
    );

    translate_event(&mut state, &event);

    match state.queue.front() {
        Some(AgUiEvent::RunError(event)) => {
            assert!(
                !event.message.contains("OpenAI")
                    && !event.message.contains("quota")
                    && !event.message.contains("429"),
                "public message must not leak provider details: {}",
                event.message
            );
            assert!(!event.message.to_lowercase().contains("contact"));
            assert!(!event.message.to_lowercase().contains("admin"));
            assert!(!event.message.to_lowercase().contains("support"));
        }
        other => panic!("expected RunError, got {other:?}"),
    }
}

#[tokio::test]
async fn turn_failed_misconfigured_maps_to_service_unavailable() {
    let mut state = test_stream_state().await;
    let event = turn_failed_event(
        &state,
        TurnFailedData {
            turn_id: TurnId::new(),
            error: "OpenAI API error (401): invalid api key".to_string(),
            error_code: Some(user_facing_error_codes::PROVIDER_MISCONFIGURED.to_string()),
            error_fields: None,
            error_disclosure: None,
        },
    );

    translate_event(&mut state, &event);

    assert_run_error(
        &state,
        "The service is temporarily unavailable. Please try again shortly.",
        "service_unavailable",
    );
}

#[tokio::test]
async fn turn_failed_unknown_code_falls_back_to_internal_error() {
    let mut state = test_stream_state().await;
    let event = turn_failed_event(
        &state,
        TurnFailedData {
            turn_id: TurnId::new(),
            error: "stack trace: thread 'tokio-runtime-worker' panicked at 'oops'".to_string(),
            error_code: None,
            error_fields: None,
            error_disclosure: None,
        },
    );

    translate_event(&mut state, &event);

    assert_run_error(
        &state,
        "Something went wrong. Please try again.",
        "internal_error",
    );
}

// Note: the exhaustive "never leaks internal concepts" property is
// verified in crates/server/src/api/public.rs::tests, since the
// sanitization logic now lives there.

#[tokio::test]
async fn turn_cancelled_emits_run_finished_not_run_error() {
    // Cancellations are a deliberate terminal state. Public AG-UI clients
    // must see a clean RUN_FINISHED, not an internal_error.
    use everruns_core::TurnCancelledData;

    let mut state = test_stream_state().await;
    let turn_id = TurnId::new();
    let input_message_id = MessageId::parse(&state.input_message_id).unwrap();
    let event = Event::new(
        SessionId::from_uuid(state.session_id),
        EventContext::turn(turn_id, input_message_id),
        TurnCancelledData {
            turn_id,
            reason: Some("client cancelled".to_string()),
            usage: None,
        },
    );

    translate_event(&mut state, &event);

    assert_eq!(state.queue.len(), 1);
    match state.queue.front() {
        Some(AgUiEvent::RunFinished(_)) => {}
        other => panic!("expected RunFinished for cancellation, got {other:?}"),
    }
    assert!(state.finished);
}

// Regression test for the stream_closed branch in run_agent. We exercise the
// helper directly (the run_agent stream_closed path constructs the same
// event) so the public RUN_ERROR contract for unexpected stream termination
// stays pinned to PublicError::fallback() — internal_error / generic message.
#[test]
fn stream_closed_falls_back_to_internal_error() {
    let event = public_run_error_event(PublicError::fallback());
    match event {
        AgUiEvent::RunError(event) => {
            assert_eq!(event.code.as_deref(), Some("internal_error"));
            assert_eq!(event.message, "Something went wrong. Please try again.");
            let lower = event.message.to_lowercase();
            assert!(
                !lower.contains("stream"),
                "leaked stream detail: {}",
                event.message
            );
            assert!(
                !lower.contains("subscription"),
                "leaked subscription detail: {}",
                event.message
            );
        }
        other => panic!("expected RunError, got {other:?}"),
    }
}
