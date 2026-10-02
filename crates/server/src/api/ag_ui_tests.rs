//! Unit tests for [`super`], the AG-UI event translation surface.
//!
//! Split out of `ag_ui.rs` to keep it under the file-size ratchet.

use super::*;
use crate::kernel_imports::{
    Event, EventContext, MessageId, OutputMessageCompletedData, OutputMessageDeltaData,
    RuntimeMessage, SessionId, ToolCall, ToolCompletedData, ToolStartedData, TurnId,
};
use chrono::Duration as ChronoDuration;
use everruns_core::events::{
    ReasonItemData, ReasonThinkingCompletedData, ReasonThinkingDeltaData,
    ReasonThinkingStartedData, TurnFailedData,
};
use everruns_platform::PublicToolVisibility;
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

/// Channel config with the defaults the old `test_stream_state()` used:
/// generic tool visibility, "Working...", reasoning summaries hidden.
fn test_config() -> AgUiChannelConfig {
    let mut config: AgUiChannelConfig = serde_json::from_value(serde_json::json!({})).unwrap();
    config.tool_visibility = PublicToolVisibility::Generic;
    config.generic_tool_text = "Working...".to_string();
    config.reasoning_summary_visible = false;
    config
}

/// One projected run: feeds runtime events through `translate_event` and
/// accumulates everything the projector emits, in order.
struct TestRun {
    projector: Projector,
    config: AgUiChannelConfig,
    frontend_tools: std::collections::HashSet<String>,
    session_id: SessionId,
    input_message_id: MessageId,
    turn_id: TurnId,
    events: Vec<AgUiEvent>,
}

impl TestRun {
    fn new() -> Self {
        Self::with_config(&test_config())
    }

    fn with_config(config: &AgUiChannelConfig) -> Self {
        Self {
            projector: Projector::new("thread", "run", public_projection_policy(config)),
            config: config.clone(),
            frontend_tools: ["set_theme".to_string()].into(),
            session_id: SessionId::new(),
            input_message_id: MessageId::new(),
            turn_id: TurnId::new(),
            events: Vec::new(),
        }
    }

    fn context(&self) -> EventContext {
        EventContext::turn(self.turn_id, self.input_message_id)
    }

    fn send(&mut self, data: impl Into<everruns_core::EventData>) {
        let event = Event::new(self.session_id, self.context(), data);
        translate_event(
            &mut self.projector,
            &self.config,
            &self.frontend_tools,
            &event,
        );
        self.events.extend(self.projector.drain());
    }

    fn types(&self) -> Vec<&'static str> {
        self.events.iter().map(AgUiEvent::event_type).collect()
    }

    fn text_deltas(&self) -> Vec<&str> {
        self.events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::TextMessageContent(event) => Some(event.delta.as_str()),
                _ => None,
            })
            .collect()
    }

    fn reasoning_deltas(&self) -> Vec<&str> {
        self.events
            .iter()
            .filter_map(|event| match event {
                AgUiEvent::ReasoningMessageContent(event) => Some(event.delta.as_str()),
                _ => None,
            })
            .collect()
    }

    fn tool_started(&mut self, id: &str, name: &str, narration: Option<&str>) {
        self.send(ToolStartedData {
            tool_call: ToolCall {
                id: id.to_string(),
                name: name.to_string(),
                arguments: serde_json::json!({"q": "private query"}),
            },
            tool_call_fingerprint: None,
            display_name: None,
            narration: narration.map(str::to_string),
        });
    }

    fn tool_completed(&mut self, id: &str, name: &str, result: &str) {
        self.send(ToolCompletedData {
            tool_call_id: id.to_string(),
            tool_name: name.to_string(),
            tool_call_fingerprint: None,
            tool_result_fingerprint: None,
            display_name: None,
            success: true,
            status: "success".to_string(),
            result: Some(vec![ContentPart::text(result)]),
            error: None,
            duration_ms: Some(10),
            capability_id: None,
            capability_name: None,
            narration: None,
        });
    }

    fn delta(&mut self, message_id: MessageId, text: &str) {
        self.send(OutputMessageDeltaData {
            turn_id: self.turn_id,
            message_id,
            delta: text.to_string(),
            accumulated: text.to_string(),
            phase: None,
        });
    }

    fn completed(&mut self, message: RuntimeMessage) {
        self.send(OutputMessageCompletedData::new(message));
    }

    fn thinking_started(&mut self) {
        self.send(ReasonThinkingStartedData {
            turn_id: self.turn_id,
            model: None,
        });
    }

    fn thinking_delta(&mut self, text: &str) {
        self.send(ReasonThinkingDeltaData {
            turn_id: self.turn_id,
            delta: text.to_string(),
            accumulated: text.to_string(),
        });
    }

    fn thinking_completed(&mut self, text: &str) {
        self.send(ReasonThinkingCompletedData {
            turn_id: self.turn_id,
            thinking: text.to_string(),
        });
    }

    fn reason_item(&mut self, item_id: &str, summary: Vec<String>) {
        self.send(ReasonItemData {
            turn_id: self.turn_id,
            provider: "openai".to_string(),
            model: Some("gpt-5.5".to_string()),
            item_id: item_id.to_string(),
            summary,
            token_count: Some(42),
        });
    }

    fn failed(&mut self, error: &str, error_code: Option<&str>) {
        self.send(TurnFailedData {
            turn_id: self.turn_id,
            error: error.to_string(),
            error_code: error_code.map(str::to_string),
            error_fields: None,
            error_disclosure: None,
        });
    }
}

fn reasoning_config() -> AgUiChannelConfig {
    let mut config = test_config();
    config.reasoning_summary_visible = true;
    config
}

fn run_error(run: &TestRun) -> &AgUiRunErrorEvent {
    assert_eq!(run.events.len(), 1);
    match &run.events[0] {
        AgUiEvent::RunError(event) => event,
        other => panic!("expected RunError, got {other:?}"),
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
    let mut run = TestRun::new();
    let output_message_id = MessageId::new();
    run.completed(RuntimeMessage::assistant("Hello from AG-UI").with_id(output_message_id));

    assert_eq!(
        run.types(),
        vec![
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "RUN_FINISHED",
        ]
    );
    match &run.events[0] {
        AgUiEvent::TextMessageStart(event) => {
            assert_eq!(event.message_id, output_message_id.uuid().to_string())
        }
        _ => panic!("expected text start event"),
    }
    assert!(run.projector.is_finished());
}

#[tokio::test]
async fn test_translate_streaming_delta_does_not_duplicate_final_content() {
    let mut run = TestRun::new();
    let streamed_message_id = MessageId::new();
    run.delta(streamed_message_id, "Hello");
    run.completed(RuntimeMessage::assistant("Hello from AG-UI").with_id(streamed_message_id));

    assert_eq!(
        run.types(),
        vec![
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "RUN_FINISHED",
        ]
    );
    assert_eq!(run.text_deltas(), vec!["Hello"]);
    assert!(run.projector.is_finished());
}

#[tokio::test]
async fn test_commentary_delta_tool_completion_does_not_hide_final_answer() {
    let mut run = TestRun::new();
    let commentary_message_id = MessageId::new();
    let final_message_id = MessageId::new();

    run.delta(commentary_message_id, "Let me check that.");
    run.completed(
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
    );

    assert_eq!(
        run.types(),
        vec![
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
        ]
    );
    assert!(!run.projector.is_finished());

    run.completed(
        RuntimeMessage::assistant("The Base harness is the default execution wrapper.")
            .with_id(final_message_id)
            .with_phase(ExecutionPhase::FinalAnswer),
    );

    assert_eq!(
        run.types(),
        vec![
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "RUN_FINISHED",
        ]
    );
    assert_eq!(
        run.text_deltas(),
        vec![
            "Let me check that.",
            "The Base harness is the default execution wrapper."
        ]
    );
    assert!(run.projector.is_finished());

    // EVE-773: AG-UI projects the canonical streamed message ids, preserving
    // the commentary/final boundary without exposing the turn uuid.
    let start_ids: Vec<&str> = run
        .events
        .iter()
        .filter_map(|event| match event {
            AgUiEvent::TextMessageStart(event) => Some(event.message_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(start_ids.len(), 2);
    assert_ne!(
        start_ids[0], start_ids[1],
        "commentary and final answer must have distinct messageIds"
    );
    assert_eq!(start_ids[0], commentary_message_id.uuid().to_string());
    assert_eq!(start_ids[1], final_message_id.uuid().to_string());
    let turn_message_id = run.turn_id.uuid().to_string();
    assert_ne!(start_ids[0], turn_message_id);
    assert_ne!(start_ids[1], turn_message_id);
}

#[tokio::test]
async fn test_reason_item_summary_hidden_by_default() {
    let mut run = TestRun::new();
    run.reason_item(
        "rs_private",
        vec!["Looked up private CRM details.".to_string()],
    );

    assert!(run.events.is_empty());
    assert!(!run.projector.is_finished());
}

#[tokio::test]
async fn test_thinking_stream_hidden_by_default() {
    let mut run = TestRun::new();
    run.thinking_started();
    run.thinking_delta("Private reasoning");
    run.thinking_completed("Private reasoning");

    assert!(run.events.is_empty());
    assert!(!run.projector.is_finished());
}

// EVE-775: a provider `reason.item` summary is a reasoning artifact and must
// project onto the AG-UI reasoning channel (REASONING_*), never the
// assistant-text channel, and never leak opaque reasoning content.
#[tokio::test]
async fn test_reason_item_summary_projects_to_reasoning_channel_when_enabled() {
    let mut run = TestRun::with_config(&reasoning_config());
    run.reason_item("rs_1", vec!["Considered the file layout.".to_string()]);

    assert_eq!(
        run.types(),
        vec![
            "REASONING_START",
            "REASONING_MESSAGE_START",
            "REASONING_MESSAGE_CONTENT",
            "REASONING_MESSAGE_END",
            "REASONING_END",
        ]
    );
    // The reasoning summary must never appear on the assistant-text channel.
    assert!(run.text_deltas().is_empty());
    assert!(!run.types().iter().any(|t| t.starts_with("TEXT_MESSAGE")));
    assert_eq!(run.reasoning_deltas(), vec!["Considered the file layout."]);
    // Hidden/opaque reasoning content is never exposed anywhere on the wire.
    let serialized: String = run
        .events
        .iter()
        .map(|event| serde_json::to_string(event).unwrap())
        .collect();
    assert!(
        !serialized.contains("OPAQUE-DO-NOT-LEAK"),
        "opaque reasoning must never reach the AG-UI wire: {serialized}"
    );
    // A reasoning summary does not end the run, the final answer still follows.
    assert!(!run.projector.is_finished());
}

#[tokio::test]
async fn test_reason_item_empty_summary_emits_nothing() {
    let mut run = TestRun::with_config(&reasoning_config());
    run.reason_item("rs_empty", vec![String::new(), "   ".to_string()]);
    assert!(run.events.is_empty());
}

// EVE-768 AC#7: reasoning summary vs commentary channel separation. Commentary
// is deliberate user-facing assistant text; the provider reasoning summary is a
// reasoning artifact. They must land on different channels and never mix.
#[tokio::test]
async fn test_reason_summary_and_commentary_use_separate_channels() {
    let mut run = TestRun::with_config(&reasoning_config());
    run.delta(MessageId::new(), "Let me look into that.");
    run.reason_item("rs_sep", vec!["Summarized the plan.".to_string()]);

    // Assistant-text channel carries only the commentary.
    assert_eq!(run.text_deltas(), vec!["Let me look into that."]);
    // Reasoning channel carries only the summary.
    assert_eq!(run.reasoning_deltas(), vec!["Summarized the plan."]);
}

#[tokio::test]
async fn test_reason_item_summary_appends_to_active_thinking_block() {
    let mut run = TestRun::with_config(&reasoning_config());
    run.thinking_started();
    run.thinking_delta("Reasoning...");
    run.reason_item("rs_app", vec!["Summary tail.".to_string()]);

    // The summary appends into the open reasoning block, no duplicate start.
    assert_eq!(
        run.types()
            .iter()
            .filter(|t| **t == "REASONING_START")
            .count(),
        1
    );
    assert!(run.text_deltas().is_empty());
    match run.events.last().unwrap() {
        AgUiEvent::ReasoningMessageContent(event) => {
            assert_eq!(event.delta, "\nSummary tail.");
        }
        other => panic!("expected appended reasoning content, got {other:?}"),
    }
}

#[tokio::test]
async fn test_tool_call_completion_does_not_finish_before_final_answer() {
    let mut run = TestRun::new();

    run.completed(
        RuntimeMessage::assistant_with_tools(
            "",
            vec![ToolCall {
                id: "call_list_skills".to_string(),
                name: "list_skills".to_string(),
                arguments: serde_json::json!({"registry": "internal"}),
            }],
        )
        .with_phase(ExecutionPhase::Commentary),
    );
    assert!(run.events.is_empty());
    assert!(!run.projector.is_finished());

    run.tool_started(
        "call_list_skills",
        "list_skills",
        Some("Listing internal skills"),
    );
    run.tool_completed("call_list_skills", "list_skills", "skill_123");
    assert!(!run.projector.is_finished());

    run.completed(
        RuntimeMessage::assistant("The Base harness is the default execution wrapper.")
            .with_phase(ExecutionPhase::FinalAnswer),
    );

    assert_eq!(
        run.types(),
        vec![
            "REASONING_START",
            "REASONING_MESSAGE_START",
            "REASONING_MESSAGE_CONTENT",
            "REASONING_MESSAGE_END",
            "REASONING_END",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "RUN_FINISHED",
        ]
    );
    match &run.events[2] {
        AgUiEvent::ReasoningMessageContent(event) => {
            assert_eq!(event.delta, "Working...");
            assert!(!event.delta.contains("list_skills"));
            assert!(!event.delta.contains("internal"));
            assert!(!event.delta.contains("skill_123"));
        }
        _ => panic!("expected generic reasoning content"),
    }
    match &run.events[6] {
        AgUiEvent::TextMessageContent(event) => {
            assert_eq!(
                event.delta,
                "The Base harness is the default execution wrapper."
            );
        }
        _ => panic!("expected final answer text content"),
    }
    assert!(run.projector.is_finished());
}

#[tokio::test]
async fn test_generic_tool_activity_hides_public_tool_details() {
    let mut config = test_config();
    config.generic_tool_text = "Checking now".to_string();
    let mut run = TestRun::with_config(&config);

    run.tool_started("call_1", "web_search", Some("Searching for hello"));
    run.tool_completed("call_1", "web_search", "result");

    assert_eq!(
        run.types(),
        vec![
            "REASONING_START",
            "REASONING_MESSAGE_START",
            "REASONING_MESSAGE_CONTENT",
            "REASONING_MESSAGE_END",
            "REASONING_END",
        ]
    );
    match &run.events[2] {
        AgUiEvent::ReasoningMessageContent(event) => {
            assert_eq!(event.delta, "Checking now");
            assert!(!event.delta.contains("web_search"));
            assert!(!event.delta.contains("hello"));
            assert!(!event.delta.contains("result"));
        }
        _ => panic!("expected generic reasoning content"),
    }

    run.completed(RuntimeMessage::assistant("Hello after tool"));

    // The public messageId is message-scoped and must never be the raw turn uuid.
    let turn_message_id = run.turn_id.uuid().to_string();
    match &run.events[5] {
        AgUiEvent::TextMessageStart(event) => {
            assert_ne!(event.message_id, turn_message_id);
        }
        _ => panic!("expected text start event"),
    }
}

#[tokio::test]
async fn test_tool_activity_reuses_active_reason_thinking_block() {
    let mut run = TestRun::with_config(&reasoning_config());
    run.thinking_started();
    run.thinking_delta("Thinking");
    run.tool_started("call_1", "web_search", None);
    run.tool_completed("call_1", "web_search", "private result");
    run.thinking_completed("Thinking");

    let types = run.types();
    assert_eq!(
        types,
        vec![
            "REASONING_START",
            "REASONING_MESSAGE_START",
            "REASONING_MESSAGE_CONTENT",
            "REASONING_MESSAGE_CONTENT",
            "REASONING_MESSAGE_END",
            "REASONING_END",
        ]
    );
    assert_eq!(types.iter().filter(|t| **t == "REASONING_START").count(), 1);
    assert_eq!(types.iter().filter(|t| **t == "REASONING_END").count(), 1);
    match &run.events[3] {
        AgUiEvent::ReasoningMessageContent(event) => {
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
    let mut config = test_config();
    config.tool_visibility = PublicToolVisibility::None;
    let mut run = TestRun::with_config(&config);

    run.tool_started("call_1", "web_search", Some("Searching for hello"));
    run.tool_completed("call_1", "web_search", "result");

    assert!(run.events.is_empty());
}

#[tokio::test]
async fn test_narrated_tool_visibility_falls_back_to_generic_text() {
    let mut config = test_config();
    config.tool_visibility = PublicToolVisibility::Narrated;
    let mut run = TestRun::with_config(&config);

    run.tool_started("call_1", "web_search", Some("Checking public sources"));

    match &run.events[2] {
        AgUiEvent::ReasoningMessageContent(event) => {
            assert_eq!(event.delta, "Working...");
            assert!(!event.delta.contains("web_search"));
            assert!(!event.delta.contains("private query"));
        }
        _ => panic!("expected narrated reasoning content"),
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
            let mut config = test_config();
            config.tool_visibility = visibility;
            config.generic_tool_text = configured.to_string();
            let mut run = TestRun::with_config(&config);

            run.tool_started("call_1", "web_search", Some("backend narration"));

            match &run.events[2] {
                AgUiEvent::ReasoningMessageContent(event) => {
                    assert_eq!(
                        event.delta, "Working...",
                        "visibility={visibility:?} configured={configured:?}"
                    );
                }
                _ => panic!(
                    "expected non-empty reasoning content for visibility {visibility:?} configured {configured:?}"
                ),
            }
        }
    }
}

fn assert_run_error(run: &TestRun, expected_message: &str, expected_code: &str) {
    let event = run_error(run);
    assert_eq!(event.message, expected_message);
    assert_eq!(event.code.as_deref(), Some(expected_code));
    assert!(run.projector.is_finished());
}

#[tokio::test]
async fn turn_failed_rate_limit_returns_generic_message() {
    let mut run = TestRun::new();
    run.failed(
        "OpenAI API error (429): {\"error\":{\"message\":\"Rate limit reached for gpt-4o in organization org-redacted on requests per min (RPM): Limit 500\",\"code\":\"rate_limit_exceeded\"}}",
        Some(user_facing_error_codes::PROVIDER_RATE_LIMITED),
    );

    assert_run_error(
        &run,
        "The service is busy right now. Please try again in a moment.",
        "rate_limited",
    );
}

#[tokio::test]
async fn turn_failed_provider_unavailable_returns_generic_message() {
    let mut run = TestRun::new();
    run.failed(
        "OpenAI API error (503): server overloaded",
        Some(user_facing_error_codes::PROVIDER_UNAVAILABLE),
    );

    assert_run_error(
        &run,
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
    let mut run = TestRun::new();
    run.failed(
        "OpenAI API error (429): {\"error\":{\"message\":\"You exceeded your current quota, please check your plan and billing details.\",\"type\":\"insufficient_quota\",\"code\":\"insufficient_quota\"}}",
        Some(user_facing_error_codes::PROVIDER_MISCONFIGURED),
    );

    let event = run_error(&run);
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

#[tokio::test]
async fn turn_failed_misconfigured_maps_to_service_unavailable() {
    let mut run = TestRun::new();
    run.failed(
        "OpenAI API error (401): invalid api key",
        Some(user_facing_error_codes::PROVIDER_MISCONFIGURED),
    );

    assert_run_error(
        &run,
        "The service is temporarily unavailable. Please try again shortly.",
        "service_unavailable",
    );
}

#[tokio::test]
async fn turn_failed_unknown_code_falls_back_to_internal_error() {
    let mut run = TestRun::new();
    run.failed(
        "stack trace: thread 'tokio-runtime-worker' panicked at 'oops'",
        None,
    );

    assert_run_error(
        &run,
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
    // must see a clean RUN_FINISHED (cancelled outcome), not an internal_error.
    use everruns_core::TurnCancelledData;

    let mut run = TestRun::new();
    run.send(TurnCancelledData {
        turn_id: run.turn_id,
        reason: Some("client cancelled".to_string()),
        usage: None,
    });

    assert_eq!(run.events.len(), 1);
    match &run.events[0] {
        AgUiEvent::RunFinished(event) => assert!(matches!(
            event.outcome,
            Some(everruns_ag_ui::RunFinishedOutcome::Cancelled)
        )),
        other => panic!("expected RunFinished for cancellation, got {other:?}"),
    }
    assert!(run.projector.is_finished());
}

// 1.0 fails a run that finishes with a message or span still open, so the
// projection must balance every start with an end before RUN_FINISHED, and
// every event must be wire-clean (typed, no null fields).
#[tokio::test]
async fn reasoning_tool_and_answer_scenario_is_balanced_and_wire_clean() {
    use std::collections::HashSet;

    let mut run = TestRun::with_config(&reasoning_config());
    let commentary_id = MessageId::new();
    run.thinking_started();
    run.thinking_delta("Planning");
    run.thinking_completed("Planning");
    run.reason_item("rs_1", vec!["Summary".to_string()]);
    run.delta(commentary_id, "Checking.");
    run.tool_started("call_1", "web_search", None);
    run.completed(
        RuntimeMessage::assistant_with_tools(
            "",
            vec![ToolCall {
                id: "call_1".to_string(),
                name: "web_search".to_string(),
                arguments: serde_json::json!({}),
            }],
        )
        .with_id(commentary_id)
        .with_phase(ExecutionPhase::Commentary),
    );
    run.thinking_started();
    run.thinking_delta("More");
    run.tool_completed("call_1", "web_search", "ok");
    // Terminal event arrives while reasoning is still open.
    run.completed(RuntimeMessage::assistant("Done.").with_phase(ExecutionPhase::FinalAnswer));

    fn assert_no_nulls(value: &Value) {
        match value {
            Value::Null => panic!("null on the wire"),
            Value::Array(items) => items.iter().for_each(assert_no_nulls),
            Value::Object(map) => map.values().for_each(assert_no_nulls),
            _ => {}
        }
    }

    let mut open_spans: HashSet<String> = HashSet::new();
    let mut open_reasoning: HashSet<String> = HashSet::new();
    let mut open_text: HashSet<String> = HashSet::new();
    for (index, event) in run.events.iter().enumerate() {
        let wire = serde_json::to_value(event).unwrap();
        assert_eq!(wire["type"], event.event_type());
        assert_no_nulls(&wire);
        match event {
            AgUiEvent::ReasoningStart(e) => assert!(open_spans.insert(e.message_id.clone())),
            AgUiEvent::ReasoningEnd(e) => assert!(open_spans.remove(&e.message_id)),
            AgUiEvent::ReasoningMessageStart(e) => {
                assert!(open_reasoning.insert(e.message_id.clone()))
            }
            AgUiEvent::ReasoningMessageContent(e) => {
                assert!(open_reasoning.contains(&e.message_id))
            }
            AgUiEvent::ReasoningMessageEnd(e) => {
                assert!(open_reasoning.remove(&e.message_id))
            }
            AgUiEvent::TextMessageStart(e) => assert!(open_text.insert(e.message_id.clone())),
            AgUiEvent::TextMessageContent(e) => assert!(open_text.contains(&e.message_id)),
            AgUiEvent::TextMessageEnd(e) => assert!(open_text.remove(&e.message_id)),
            AgUiEvent::RunFinished(_) => {
                assert_eq!(index, run.events.len() - 1, "RUN_FINISHED must be last");
                assert!(open_spans.is_empty(), "span open at RUN_FINISHED");
                assert!(open_reasoning.is_empty(), "reasoning open at RUN_FINISHED");
                assert!(open_text.is_empty(), "text open at RUN_FINISHED");
            }
            other => panic!("unexpected event {other:?}"),
        }
    }
    assert!(matches!(run.events.last(), Some(AgUiEvent::RunFinished(_))));
}

#[test]
fn is_valid_ag_ui_id_accepts_and_rejects() {
    assert!(is_valid_ag_ui_id(&Uuid::new_v4().to_string()));
    assert!(is_valid_ag_ui_id("run-1"));
    assert!(is_valid_ag_ui_id("abc_DEF.9"));
    assert!(is_valid_ag_ui_id(&"a".repeat(128)));
    assert!(!is_valid_ag_ui_id(""));
    assert!(!is_valid_ag_ui_id("a:b"));
    assert!(!is_valid_ag_ui_id("a b"));
    assert!(!is_valid_ag_ui_id(&"a".repeat(129)));
}

// Regression test for the stream_closed branch in run_agent. We exercise the
// helper directly (the run_agent stream_closed path constructs the same
// event) so the public RUN_ERROR contract for unexpected stream termination
// stays pinned to PublicError::fallback() — internal_error / generic message.
#[test]
fn stream_closed_falls_back_to_internal_error() {
    let event = public_run_error_event(PublicError::fallback());
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

#[test]
fn parked_ask_user_ends_the_run_with_an_interrupt() {
    let mut run = TestRun::new();
    run.send(OutputMessageDeltaData {
        turn_id: run.turn_id,
        message_id: MessageId::new(),
        delta: "One question first.".into(),
        accumulated: "One question first.".into(),
        phase: None,
    });
    run.send(everruns_core::events::ToolCallRequestedData {
        tool_calls: vec![ToolCall {
            id: "call_ask_1".into(),
            name: everruns_builtins::ask_user::ASK_USER_TOOL_NAME.into(),
            arguments: serde_json::json!({
                "questions": [{
                    "kind": "text", "id": "name", "header": "Name",
                    "question": "What should I call it?",
                    "multi_select": false, "allow_other": false, "options": [],
                }],
            }),
        }],
        tool_summaries: Vec::new(),
        headline: None,
        completed_headline: None,
    });
    assert_eq!(
        run.types(),
        [
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "RUN_FINISHED"
        ]
    );
    let Some(AgUiEvent::RunFinished(finished)) = run.events.last() else {
        panic!("expected RUN_FINISHED");
    };
    let Some(everruns_ag_ui::RunFinishedOutcome::Interrupt { interrupts }) = &finished.outcome
    else {
        panic!("expected an interrupt, got {:?}", finished.outcome);
    };
    assert_eq!(interrupts[0].id, "call_ask_1");
    assert_eq!(interrupts[0].reason, "everruns.ask_user");
    assert!(interrupts[0].response_schema.is_some());
    // Usage stays off unless the endpoint turns it on.
    assert_eq!(finished.usage, None);
}

#[test]
fn a_parked_secret_question_is_never_offered_as_a_form() {
    let mut run = TestRun::new();
    run.send(everruns_core::events::ToolCallRequestedData {
        tool_calls: vec![ToolCall {
            id: "call_ask_2".into(),
            name: everruns_builtins::ask_user::ASK_USER_TOOL_NAME.into(),
            arguments: serde_json::json!({
                "questions": [{
                    "kind": "secret", "id": "key", "header": "Key",
                    "question": "Which API key?", "multi_select": false,
                    "allow_other": false, "options": [], "secret_name": "API_KEY",
                }],
            }),
        }],
        tool_summaries: Vec::new(),
        headline: None,
        completed_headline: None,
    });
    let Some(AgUiEvent::RunFinished(finished)) = run.events.last() else {
        panic!("expected RUN_FINISHED");
    };
    let Some(everruns_ag_ui::RunFinishedOutcome::Interrupt { interrupts }) = &finished.outcome
    else {
        panic!("expected an interrupt");
    };
    assert_eq!(interrupts[0].reason, "everruns.secret_required");
    assert!(interrupts[0].response_schema.is_none());
    assert!(interrupts[0].metadata.is_none());
}

#[test]
fn a_call_to_no_frontend_tool_raises_nothing() {
    let mut run = TestRun::new();
    run.send(everruns_core::events::ToolCallRequestedData {
        tool_calls: vec![ToolCall {
            id: "call_x".into(),
            name: "not_declared_here".into(),
            arguments: serde_json::json!({}),
        }],
        tool_summaries: Vec::new(),
        headline: None,
        completed_headline: None,
    });
    assert!(run.events.is_empty());
    assert!(!run.projector.is_finished());
}

#[test]
fn a_frontend_tool_call_streams_by_name_and_ends_the_run_in_success() {
    // Generic tool visibility hides server tools; a frontend tool is the
    // consumer's own, so its name and arguments are what it needs to run it.
    let mut run = TestRun::new();
    run.send(everruns_core::events::ToolCallRequestedData {
        tool_calls: vec![ToolCall {
            id: "call_x".into(),
            name: "set_theme".into(),
            arguments: serde_json::json!({ "theme": "dark" }),
        }],
        tool_summaries: Vec::new(),
        headline: None,
        completed_headline: None,
    });
    assert_eq!(
        run.types(),
        [
            "TOOL_CALL_START",
            "TOOL_CALL_ARGS",
            "TOOL_CALL_END",
            "RUN_FINISHED"
        ]
    );
    let wire: Vec<Value> = run
        .events
        .iter()
        .map(|event| serde_json::to_value(event).unwrap())
        .collect();
    assert_eq!(wire[0]["toolCallName"], "set_theme");
    assert_eq!(wire[1]["delta"], r#"{"theme":"dark"}"#);
    assert_eq!(
        wire[3]["outcome"],
        serde_json::json!({ "type": "success", "pendingToolCallIds": ["call_x"] })
    );
}

fn subagent_task(state: &str, extra: Value) -> everruns_core::EventData {
    let mut task = serde_json::json!({
        "id": "task_child",
        "session_id": SessionId::new().to_string(),
        "kind": "subagent",
        "display_name": "Researcher",
        "spec": { "instructions": "private brief", "mode": "foreground" },
        "state": state,
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:01Z",
    });
    if let (Some(task), Value::Object(extra)) = (task.as_object_mut(), extra) {
        task.extend(extra);
    }
    let data = everruns_core::events::SessionTaskEventData {
        task: serde_json::from_value(task).unwrap(),
    };
    if state == "running" {
        everruns_core::EventData::TaskCreated(data)
    } else {
        everruns_core::EventData::TaskUpdated(data)
    }
}

#[test]
fn subagents_stay_hidden_unless_the_endpoint_shows_them() {
    let mut hidden = TestRun::new();
    hidden.send(subagent_task("running", Value::Null));
    hidden.send(subagent_task(
        "succeeded",
        serde_json::json!({ "summary": "done" }),
    ));
    assert!(
        hidden.types().is_empty(),
        "off by default: {:?}",
        hidden.types()
    );

    let mut config = test_config();
    config.subagents_visible = true;
    let mut run = TestRun::with_config(&config);
    run.send(subagent_task("running", Value::Null));
    run.send(subagent_task(
        "failed",
        serde_json::json!({ "error": { "kind": "provider_error", "message": "key sk-1 rejected" } }),
    ));
    run.completed(RuntimeMessage::assistant("Done."));
    assert_eq!(
        run.types(),
        [
            "SUBAGENT_STARTED",
            "SUBAGENT_ERROR",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "RUN_FINISHED",
        ]
    );
    let wire = serde_json::to_string(&run.events).unwrap();
    assert!(
        !wire.contains("private brief"),
        "the task spec stays private"
    );
    assert!(
        !wire.contains("sk-1"),
        "a child's error goes through PublicError"
    );
}

#[test]
fn run_metadata_carries_the_turn_and_withholds_model_and_session() {
    let mut run = TestRun::new();
    let generation: everruns_core::events::LlmGenerationData =
        serde_json::from_value(serde_json::json!({
            "messages": [],
            "output": { "text": "", "tool_calls": [] },
            "metadata": { "model": "gpt-5", "success": true },
        }))
        .unwrap();
    run.send(generation);
    run.completed(RuntimeMessage::assistant("Done."));
    let Some(AgUiEvent::RunFinished(finished)) = run.events.last() else {
        panic!("expected RUN_FINISHED");
    };
    assert_eq!(
        serde_json::to_value(&finished.base.metadata).unwrap(),
        serde_json::json!({ "everruns": { "turnId": run.turn_id.to_string() } }),
        "no model without usage, no session id on a policy that names none"
    );

    let mut config = test_config();
    config.usage_visible = true;
    let policy = public_projection_policy(&config);
    assert!(policy.model_visible && policy.session_id.is_none());
}

#[test]
fn capabilities_follow_the_endpoint_config() {
    let mut schema: Value = serde_json::from_str(everruns_ag_ui::SCHEMA_JSON).unwrap();
    schema["$ref"] = serde_json::json!("#/$defs/AgentCapabilities");
    let validator = jsonschema::validator_for(&schema).unwrap();

    let defaults = crate::api::ag_ui_capabilities::capabilities("Bot", None, &test_config());
    let wire = serde_json::to_value(&defaults).unwrap();
    assert!(validator.is_valid(&wire), "schema rejected {wire}");
    assert_eq!(
        wire["identity"],
        serde_json::json!({ "name": "Bot", "type": "everruns" })
    );
    assert_eq!(wire["reasoning"]["supported"], false);
    assert!(
        wire.get("multiAgent").is_none(),
        "subagents undeclared when hidden"
    );
    assert_eq!(wire["humanInTheLoop"]["approvals"], false);
    assert_eq!(wire["tools"]["clientProvided"], true);
    assert!(
        wire["tools"].get("items").is_none(),
        "agent tools stay private"
    );
    assert_eq!(wire["custom"]["everruns"]["usage"], false);

    let mut config = reasoning_config();
    config.subagents_visible = true;
    config.tool_approval_interrupts = true;
    config.usage_visible = true;
    let wire = serde_json::to_value(crate::api::ag_ui_capabilities::capabilities(
        "Bot",
        Some("Helps"),
        &config,
    ))
    .unwrap();
    assert!(validator.is_valid(&wire), "schema rejected {wire}");
    assert_eq!(wire["identity"]["description"], "Helps");
    assert_eq!(wire["reasoning"]["supported"], true);
    assert_eq!(wire["multiAgent"]["delegation"], true);
    assert_eq!(wire["humanInTheLoop"]["approvals"], true);
    assert_eq!(wire["custom"]["everruns"]["usage"], true);
}
