use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::*;
use crate::channel::{
    ChannelAgentSurface, ChannelStreamDelivery, DeliveryContext, DeliveryResult, DeliveryTarget,
    OutboundChannelMessage,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Post(String),
    Start,
    Append(String),
    Replace(String),
    Stop,
    Status(String),
    Title(String),
}

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<Call>>,
    stream: bool,
    surface: bool,
    fail_start: bool,
}

impl Recorder {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
    fn push(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }
}

#[async_trait]
impl ChannelDeliveryAdapter for Recorder {
    fn platform(&self) -> &str {
        "test"
    }
    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        _: &DeliveryContext,
    ) -> DeliveryResult {
        self.push(Call::Post(message.text.clone()));
        DeliveryResult::Ok
    }
    async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }
    fn streaming(&self) -> Option<&dyn ChannelStreamDelivery> {
        self.stream.then_some(self as &dyn ChannelStreamDelivery)
    }
    fn agent_surface(&self) -> Option<&dyn ChannelAgentSurface> {
        self.surface.then_some(self as &dyn ChannelAgentSurface)
    }
}

#[async_trait]
impl ChannelStreamDelivery for Recorder {
    async fn start(&self, _: &DeliveryContext) -> Result<String, String> {
        if self.fail_start {
            return Err("no streams today".into());
        }
        self.push(Call::Start);
        Ok("h1".into())
    }
    async fn append(&self, _: &str, text: &str, _: &DeliveryContext) -> DeliveryResult {
        self.push(Call::Append(text.into()));
        DeliveryResult::Ok
    }
    async fn replace(&self, _: &str, text: &str, _: &DeliveryContext) -> DeliveryResult {
        self.push(Call::Replace(text.into()));
        DeliveryResult::Ok
    }
    async fn stop(&self, _: &str, _: &DeliveryContext) -> DeliveryResult {
        self.push(Call::Stop);
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelAgentSurface for Recorder {
    async fn set_status(&self, status: &str, _: &DeliveryContext) -> DeliveryResult {
        self.push(Call::Status(status.into()));
        DeliveryResult::Ok
    }
    async fn set_title(&self, title: &str, _: &DeliveryContext) -> DeliveryResult {
        self.push(Call::Title(title.into()));
        DeliveryResult::Ok
    }
}

const TURN: &str = "message_turn";

fn delivery(
    recorder: &Arc<Recorder>,
    mode: ChannelReplyMode,
    options: DeliveryOptions,
) -> TurnDelivery {
    let adapter: Arc<dyn ChannelDeliveryAdapter> = recorder.clone();
    TurnDelivery::new(
        adapter,
        DeliveryTarget::new("C1", "t1").context("", mode),
        SessionId::new(),
        TURN,
        DeliveryOptions {
            reply_mode: mode,
            ..options
        },
    )
}

fn event(event_type: &str, data: Value) -> DeliveryEvent {
    event_for(TURN, event_type, data)
}

fn event_for(turn: &str, event_type: &str, data: Value) -> DeliveryEvent {
    DeliveryEvent {
        sequence: Some(1),
        event_type: event_type.into(),
        data,
        input_message_id: Some(turn.into()),
    }
}

fn completed(id: &str, text: &str) -> DeliveryEvent {
    event(
        OUTPUT_MESSAGE_COMPLETED,
        json!({"message": {"id": id, "content": [{"type": "text", "text": text}]}}),
    )
}

fn delta(id: &str, text: &str) -> DeliveryEvent {
    event(
        OUTPUT_MESSAGE_DELTA,
        json!({"message_id": id, "delta": text}),
    )
}

#[tokio::test]
async fn automatic_mode_posts_each_completed_message_of_its_own_turn() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(
        &recorder,
        ChannelReplyMode::AllMessages,
        DeliveryOptions::default(),
    );

    delivery.observe(&completed("m1", "first")).await;
    delivery.observe(&completed("m2", "   ")).await;
    delivery
        .observe(&event_for(
            "message_other",
            OUTPUT_MESSAGE_COMPLETED,
            json!({"message": {"id": "x", "content": [{"type": "text", "text": "not ours"}]}}),
        ))
        .await;
    delivery.observe(&completed("m3", "second")).await;
    let step = delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    assert_eq!(step, DeliveryStep::Finished(TURN_COMPLETED.into()));
    assert_eq!(
        recorder.calls(),
        vec![Call::Post("first".into()), Call::Post("second".into())]
    );
    assert!(delivery.delivered());
}

#[tokio::test]
async fn a_turn_that_delivered_nothing_gets_one_notice() {
    let recorder = Arc::new(Recorder::default());
    let options = DeliveryOptions {
        session_link: Some("https://app/s/1".into()),
        ..DeliveryOptions::default()
    };
    let mut delivery = delivery(&recorder, ChannelReplyMode::AllMessages, options);
    delivery
        .observe(&event(TURN_FAILED, json!({"error": "secret detail"})))
        .await;
    delivery.observe(&event(TURN_FAILED, json!({}))).await;

    assert_eq!(
        recorder.calls(),
        vec![Call::Post(
            "The agent could not finish this request. https://app/s/1".into()
        )]
    );
}

#[tokio::test]
async fn tool_only_mode_posts_no_assistant_text_and_counts_receipts() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(
        &recorder,
        ChannelReplyMode::ToolOnly,
        DeliveryOptions::default(),
    );
    delivery
        .observe(&completed("m1", "internal thinking"))
        .await;
    let receipt = json!({
        "tool_name": "channel_post_message", "success": true,
        "result": [{"type": "text", "text": r#"{"delivered":true,"platform":"x","message_ref":"r1"}"#}]
    });
    delivery.observe(&event(TOOL_COMPLETED, receipt)).await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    assert!(recorder.calls().is_empty());
    assert!(delivery.delivered());
}

#[tokio::test]
async fn tool_only_mode_still_announces_a_failed_turn() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(
        &recorder,
        ChannelReplyMode::ToolOnly,
        DeliveryOptions::default(),
    );
    let receipt = json!({
        "tool_name": "channel_post_message", "success": true,
        "result": [{"type": "text", "text": r#"{"delivered":true,"message_ref":"r1"}"#}]
    });
    delivery.observe(&event(TOOL_COMPLETED, receipt)).await;
    delivery.observe(&event(TURN_FAILED, json!({}))).await;
    assert_eq!(
        recorder.calls(),
        vec![Call::Post(
            "The agent could not finish this request.".into()
        )]
    );
}

#[tokio::test]
async fn streaming_appends_deltas_and_closes_on_the_completed_text() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(
        &recorder,
        ChannelReplyMode::AllMessages,
        DeliveryOptions::default(),
    );
    delivery.observe(&delta("m1", "Hel")).await;
    delivery.observe(&delta("m1", "lo")).await;
    assert!(delivery.has_pending_text());
    delivery.flush().await;
    assert!(!delivery.has_pending_text());
    // The final chunk only exists in the completed message.
    delivery.observe(&completed("m1", "Hello world")).await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    assert_eq!(
        recorder.calls(),
        vec![
            Call::Start,
            Call::Append("Hello".into()),
            Call::Append(" world".into()),
            Call::Stop,
        ]
    );
}

#[tokio::test]
async fn a_large_burst_flushes_without_waiting_for_the_timer() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(
        &recorder,
        ChannelReplyMode::AllMessages,
        DeliveryOptions::default(),
    );
    let burst = "x".repeat(super::STREAM_FLUSH_CHARS);
    delivery.observe(&delta("m1", &burst)).await;
    assert_eq!(recorder.calls(), vec![Call::Start, Call::Append(burst)]);
}

#[tokio::test]
async fn a_guardrail_replacement_rewrites_the_stream_and_its_completion_is_done() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(
        &recorder,
        ChannelReplyMode::AllMessages,
        DeliveryOptions::default(),
    );
    delivery.observe(&delta("m1", "leaked canary")).await;
    delivery.flush().await;
    delivery
        .observe(&event(
            OUTPUT_MESSAGE_REPLACED,
            json!({"message_id": "m1", "replacement": "[removed]"}),
        ))
        .await;
    delivery.observe(&completed("m1", "[removed]")).await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    assert_eq!(
        recorder.calls(),
        vec![
            Call::Start,
            Call::Append("leaked canary".into()),
            Call::Replace("[removed]".into()),
            Call::Stop,
        ]
    );
}

#[tokio::test]
async fn a_stream_that_cannot_open_falls_back_to_one_post() {
    let recorder = Arc::new(Recorder {
        stream: true,
        fail_start: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(
        &recorder,
        ChannelReplyMode::AllMessages,
        DeliveryOptions::default(),
    );
    delivery.observe(&delta("m1", "Hel")).await;
    delivery.observe(&delta("m1", "lo")).await;
    delivery.observe(&completed("m1", "Hello")).await;
    assert_eq!(recorder.calls(), vec![Call::Post("Hello".into())]);
}

#[tokio::test]
async fn streaming_off_posts_whole_messages() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let options = DeliveryOptions {
        stream: false,
        ..DeliveryOptions::default()
    };
    let mut delivery = delivery(&recorder, ChannelReplyMode::AllMessages, options);
    delivery.observe(&delta("m1", "Hel")).await;
    delivery.observe(&completed("m1", "Hello")).await;
    assert_eq!(recorder.calls(), vec![Call::Post("Hello".into())]);
}

#[tokio::test]
async fn an_open_stream_is_stopped_when_the_turn_ends_without_completion() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(
        &recorder,
        ChannelReplyMode::AllMessages,
        DeliveryOptions::default(),
    );
    delivery.observe(&delta("m1", "partial")).await;
    delivery.observe(&event(TURN_CANCELLED, json!({}))).await;
    assert_eq!(
        recorder.calls(),
        vec![Call::Start, Call::Append("partial".into()), Call::Stop]
    );
}

#[tokio::test]
async fn cancellation_ends_the_delivery_only_after_its_turn_began() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(
        &recorder,
        ChannelReplyMode::AllMessages,
        DeliveryOptions::default(),
    );
    // A cancellation of an earlier turn, before this turn's first event.
    let early = delivery
        .observe(&event_for("message_fresh", TURN_CANCELLED, json!({})))
        .await;
    assert_eq!(early, DeliveryStep::Continue);

    delivery.observe(&event(TURN_STARTED, json!({}))).await;
    let late = delivery
        .observe(&event_for("message_fresh", TURN_CANCELLED, json!({})))
        .await;
    assert_eq!(late, DeliveryStep::Finished(TURN_CANCELLED.into()));
    assert_eq!(
        recorder.calls(),
        vec![Call::Post("This request was cancelled.".into())]
    );
}

#[tokio::test]
async fn status_follows_tools_and_clears_at_the_end() {
    let recorder = Arc::new(Recorder {
        surface: true,
        ..Recorder::default()
    });
    let options = DeliveryOptions {
        tool_status: Some("is using tools...".into()),
        ..DeliveryOptions::default()
    };
    let mut delivery = delivery(&recorder, ChannelReplyMode::AllMessages, options);
    delivery.observe(&event(TURN_STARTED, json!({}))).await;
    delivery.observe(&event(TOOL_STARTED, json!({}))).await;
    delivery.observe(&event(TOOL_STARTED, json!({}))).await;
    delivery.observe(&event(TOOL_COMPLETED, json!({}))).await;
    delivery.observe(&event(TOOL_COMPLETED, json!({}))).await;
    delivery
        .observe(&event(SESSION_TITLE_UPDATED, json!({"title": "Revenue"})))
        .await;
    delivery.observe(&completed("m1", "done")).await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    assert_eq!(
        recorder.calls(),
        vec![
            Call::Status("is thinking...".into()),
            Call::Status("is using tools...".into()),
            Call::Status("is thinking...".into()),
            Call::Title("Revenue".into()),
            Call::Post("done".into()),
            Call::Status(String::new()),
        ]
    );
}

#[test]
fn envelopes_are_read_from_the_canonical_shape() {
    let envelope = json!({
        "type": "output.message.delta",
        "sequence": 7,
        "context": {"input_message_id": "message_1"},
        "data": {"delta": "x"}
    });
    let event = DeliveryEvent::from_envelope(&envelope).unwrap();
    assert_eq!(event.sequence, Some(7));
    assert_eq!(event.input_message_id.as_deref(), Some("message_1"));
    assert_eq!(event.data["delta"], "x");
    assert!(!event.is_terminal());
    assert!(DeliveryEvent::from_envelope(&json!({"data": {}})).is_none());
}
