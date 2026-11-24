use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{Value, json};

use super::*;
use crate::channel::{
    ApprovalPrompt, ChannelAgentSurface, ChannelApprovalPrompt, ChannelProgressSurface,
    ChannelReactionSurface, ChannelStreamDelivery, DeliveryContext, DeliveryResult, DeliveryTarget,
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
    Approval(String),
    ProgressPost(String),
    ProgressUpdate(String),
    React(String, String, bool),
    View(String, String),
}

#[derive(Default)]
struct Recorder {
    calls: Mutex<Vec<Call>>,
    stream: bool,
    surface: bool,
    approvals: bool,
    approval_failure: Option<&'static str>,
    reply_failure: Option<&'static str>,
    stop_failure: bool,
    slack: bool,
    progress: bool,
    fail_start: bool,
    reactions: bool,
    views: bool,
    reply_outcome: Mutex<Option<DeliveryResult>>,
    append_outcome: Mutex<Option<DeliveryResult>>,
    start_outcome: Mutex<Option<DeliveryResult>>,
    replace_outcome: Mutex<Option<DeliveryResult>>,
    after_post: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
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
        if self.slack { "slack" } else { "test" }
    }
    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        context: &DeliveryContext,
    ) -> DeliveryResult {
        self.push(Call::Post(message.text.clone()));
        let result = self
            .reply_outcome
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(DeliveryResult::Ok);
        if matches!(result, DeliveryResult::Ok) {
            if let Some(sink) = &context.receipts
                && sink.confirmed("C1:accepted").await.is_err()
            {
                return DeliveryResult::Unknown {
                    confirmed: vec!["C1:accepted".into()],
                };
            }
            if let Some(callback) = self.after_post.lock().unwrap().as_ref() {
                callback();
            }
        }
        result
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
    fn approvals(&self) -> Option<&dyn ChannelApprovalPrompt> {
        self.approvals.then_some(self as &dyn ChannelApprovalPrompt)
    }
    fn progress(&self) -> Option<&dyn ChannelProgressSurface> {
        self.progress.then_some(self as &dyn ChannelProgressSurface)
    }
    fn reactions(&self) -> Option<&dyn ChannelReactionSurface> {
        self.reactions
            .then_some(self as &dyn ChannelReactionSurface)
    }
    fn app_views(&self) -> Option<&dyn crate::channel::ChannelAppViewSurface> {
        self.views
            .then_some(self as &dyn crate::channel::ChannelAppViewSurface)
    }
}

#[async_trait]
impl crate::channel::ChannelAppViewSurface for Recorder {
    async fn show(
        &self,
        _: SessionId,
        view: &crate::channel::ChannelAppView,
        _: &DeliveryContext,
    ) -> DeliveryResult {
        self.push(Call::View(view.app.resource_uri.clone(), view.text.clone()));
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelReactionSurface for Recorder {
    fn stream_message_key(&self, handle: &str, _: &DeliveryContext) -> String {
        format!("C1:{handle}")
    }
    async fn post_keyed(
        &self,
        message: &OutboundChannelMessage,
        _: &DeliveryContext,
    ) -> Result<Option<String>, DeliveryResult> {
        self.push(Call::Post(message.text.clone()));
        if let Some(outcome) = self.reply_outcome.lock().unwrap().clone() {
            return Err(outcome);
        }
        match self.reply_failure {
            Some(error) => Err(DeliveryResult::PermanentError(error.into())),
            None => Ok(Some("C1:posted".into())),
        }
    }
    async fn react(
        &self,
        key: &str,
        emoji: &str,
        add: bool,
        _: &DeliveryContext,
    ) -> DeliveryResult {
        self.push(Call::React(key.into(), emoji.into(), add));
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelApprovalPrompt for Recorder {
    async fn prompt(
        &self,
        _: SessionId,
        prompt: &ApprovalPrompt,
        _: &DeliveryContext,
    ) -> DeliveryResult {
        self.push(Call::Approval(prompt.action.clone()));
        match self.approval_failure {
            Some(error) => DeliveryResult::TransientError(error.into()),
            None => DeliveryResult::Ok,
        }
    }
}

#[async_trait]
impl ChannelProgressSurface for Recorder {
    async fn post(&self, text: &str, _: &DeliveryContext) -> Result<String, String> {
        self.push(Call::ProgressPost(text.into()));
        Ok("p1".into())
    }
    async fn update(&self, handle: &str, text: &str, _: &DeliveryContext) -> DeliveryResult {
        assert_eq!(handle, "p1");
        self.push(Call::ProgressUpdate(text.into()));
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelStreamDelivery for Recorder {
    async fn start_checked(&self, context: &DeliveryContext) -> Result<String, DeliveryResult> {
        if let Some(outcome) = self.start_outcome.lock().unwrap().clone() {
            self.push(Call::Start);
            return Err(outcome);
        }
        self.start(context)
            .await
            .map_err(DeliveryResult::TransientError)
    }

    async fn start(&self, _: &DeliveryContext) -> Result<String, String> {
        if self.fail_start {
            return Err("no streams today".into());
        }
        self.push(Call::Start);
        Ok("h1".into())
    }
    async fn append(&self, _: &str, text: &str, _: &DeliveryContext) -> DeliveryResult {
        self.push(Call::Append(text.into()));
        self.append_outcome
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(DeliveryResult::Ok)
    }
    async fn replace(&self, _: &str, text: &str, _: &DeliveryContext) -> DeliveryResult {
        self.push(Call::Replace(text.into()));
        self.replace_outcome
            .lock()
            .unwrap()
            .clone()
            .unwrap_or(DeliveryResult::Ok)
    }
    async fn stop(&self, _: &str, _: &DeliveryContext) -> DeliveryResult {
        self.push(Call::Stop);
        if self.stop_failure {
            DeliveryResult::PermanentError("PRIVATE_ERROR https://slack.example".into())
        } else {
            DeliveryResult::Ok
        }
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

fn delivery(recorder: &Arc<Recorder>, options: DeliveryOptions) -> TurnDelivery {
    let adapter: Arc<dyn ChannelDeliveryAdapter> = recorder.clone();
    TurnDelivery::new(
        adapter,
        DeliveryTarget::new("C1", "t1").context(""),
        SessionId::new(),
        TURN,
        options,
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
        turn_id: None,
        turn_traceparent: None,
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
    let mut delivery = delivery(&recorder, DeliveryOptions::default());

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
    let mut delivery = delivery(&recorder, options);
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

fn note(id: &str, text: &str) -> DeliveryEvent {
    event(
        OUTPUT_MESSAGE_COMPLETED,
        json!({"message": {
            "id": id,
            "phase": "commentary",
            "phase_source": "communication",
            "content": [{"type": "text", "text": text}],
        }}),
    )
}

fn sent(text: &str, delivery: Option<Value>) -> DeliveryEvent {
    let mut data = json!({"message_id": "message_sent", "text": text, "tool_call_id": "c1"});
    if let Some(delivery) = delivery {
        data["delivery"] = delivery;
    }
    event(CONVERSATION_MESSAGE, data)
}

#[tokio::test]
async fn explicit_agent_posts_sent_messages_and_never_its_notes() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery
        .observe(&event(
            OUTPUT_MESSAGE_STARTED,
            json!({"message_id": "m1", "phase": "commentary"}),
        ))
        .await;
    delivery.observe(&delta("m1", "internal thinking")).await;
    delivery.observe(&note("m1", "internal thinking")).await;
    delivery.observe(&sent("Shipped.", None)).await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    assert_eq!(recorder.calls(), vec![Call::Post("Shipped.".into())]);
    assert!(delivery.delivered());
}

#[tokio::test]
async fn a_message_the_sender_already_delivered_is_not_posted_again() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery
        .observe(&sent(
            "Shipped.",
            Some(json!({"platform": "x", "channel": "C1", "message_ref": "r1"})),
        ))
        .await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    assert!(recorder.calls().is_empty());
    assert!(delivery.delivered());
}

#[tokio::test]
async fn a_misconfigured_agent_says_so_without_detail() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery
        .observe(&event(
            TURN_FAILED,
            json!({"error": "Model meta/muse-spark is disabled", "error_code": "model_unavailable"}),
        ))
        .await;

    let calls = recorder.calls();
    assert_eq!(
        calls,
        vec![Call::Post(
            "This agent is not set up correctly, so it cannot answer right now. Someone who manages it needs to check its model and provider settings.".into()
        )]
    );
}

#[tokio::test]
async fn other_failure_codes_keep_the_plain_notice() {
    for code in ["processing_error", "provider_rate_limited"] {
        let recorder = Arc::new(Recorder::default());
        let mut delivery = delivery(&recorder, DeliveryOptions::default());
        delivery
            .observe(&event(TURN_FAILED, json!({"error_code": code})))
            .await;
        assert_eq!(
            recorder.calls(),
            vec![Call::Post(
                "The agent could not finish this request.".into()
            )],
            "{code}"
        );
    }
    assert!(is_setup_error_code("provider_misconfigured"));
    assert!(!is_setup_error_code("budget_exhausted"));
}

#[tokio::test]
async fn no_reply_keeps_the_conversation_silent() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&note("m1", "nothing for me here")).await;
    delivery
        .observe(&event(
            TOOL_COMPLETED,
            json!({"tool_name": "no_reply", "success": true}),
        ))
        .await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn explicit_agent_still_announces_a_failed_turn() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&sent("Working on it.", None)).await;
    delivery.observe(&event(TURN_FAILED, json!({}))).await;
    assert_eq!(
        recorder.calls(),
        vec![
            Call::Post("Working on it.".into()),
            Call::Post("The agent could not finish this request.".into()),
        ]
    );
}

#[tokio::test]
async fn streaming_appends_deltas_and_closes_on_the_completed_text() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&delta("m1", "Hel")).await;
    delivery.observe(&delta("m1", "lo")).await;
    assert!(delivery.has_pending_work());
    delivery.flush().await;
    assert!(!delivery.has_pending_work());
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
async fn a_stream_flush_keeps_an_unclosed_token_for_the_next_append() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&delta("m1", "Hi <!he")).await;
    delivery.flush().await;
    delivery.observe(&delta("m1", "re> and a < b")).await;
    delivery.flush().await;
    // Closing sends whatever is left, token or not.
    delivery
        .observe(&completed("m1", "Hi <!here> and a < b"))
        .await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    assert_eq!(
        recorder.calls(),
        vec![
            Call::Start,
            Call::Append("Hi ".into()),
            Call::Append("<!here> and a ".into()),
            Call::Append("< b".into()),
            Call::Stop,
        ]
    );
}

#[test]
fn a_long_unclosed_tail_is_not_held_back() {
    let long = format!("a <{}", "x".repeat(super::MAX_HELD_TOKEN_CHARS));
    assert_eq!(super::open_token_start(&long), long.len());
    assert_eq!(super::open_token_start("a <@U1"), 2);
    assert_eq!(super::open_token_start("a <@U1> b"), 9);
}

#[tokio::test]
async fn a_large_burst_flushes_without_waiting_for_the_timer() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
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
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
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
        ],
        "replace closes the stream itself"
    );
}

#[tokio::test]
async fn a_stream_that_cannot_open_falls_back_to_one_post() {
    let recorder = Arc::new(Recorder {
        stream: true,
        fail_start: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
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
    let mut delivery = delivery(&recorder, options);
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
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
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
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
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
    let mut delivery = delivery(&recorder, options);
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
        "data": {"delta": "x"},
        "metadata": {(crate::causal::TURN_TRACEPARENT_KEY):
            "00-11111111111111111111111111111111-2222222222222222-01"}
    });
    let event = DeliveryEvent::from_envelope(&envelope).unwrap();
    assert_eq!(event.sequence, Some(7));
    assert_eq!(event.input_message_id.as_deref(), Some("message_1"));
    assert_eq!(event.data["delta"], "x");
    assert!(
        event.turn_traceparent.is_none(),
        "only committed starts carry a parent"
    );
    let mut turn_start = envelope;
    turn_start["type"] = json!(TURN_STARTED);
    assert!(
        DeliveryEvent::from_envelope(&turn_start)
            .unwrap()
            .turn_traceparent
            .is_some()
    );
    turn_start["sequence"] = Value::Null;
    assert!(
        DeliveryEvent::from_envelope(&turn_start)
            .unwrap()
            .turn_traceparent
            .is_none()
    );
    assert!(!event.is_terminal());
    assert!(DeliveryEvent::from_envelope(&json!({"data": {}})).is_none());
}

#[tokio::test]
async fn a_late_delta_cannot_reopen_a_finished_message() {
    let recorder = Arc::new(Recorder {
        stream: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&delta("m1", "Hel")).await;
    delivery.observe(&completed("m1", "Hello")).await;
    // Live deltas travel apart from durable events and can arrive after.
    delivery.observe(&delta("m1", "lo")).await;
    delivery.observe(&completed("m1", "Hello")).await;
    delivery.flush().await;
    assert_eq!(
        recorder.calls(),
        vec![Call::Start, Call::Append("Hello".into()), Call::Stop]
    );
}

#[tokio::test]
async fn a_completed_message_posts_once_even_if_seen_twice() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&completed("m1", "Hello")).await;
    delivery.observe(&completed("m1", "Hello")).await;
    assert_eq!(recorder.calls(), vec![Call::Post("Hello".into())]);
}

fn approval_completed(action: &str) -> DeliveryEvent {
    let payload = json!({"awaiting_approval": true, "action": action});
    event(
        TOOL_COMPLETED,
        json!({
            "tool_name": "request_approval",
            "success": true,
            "result": [{"type": "text", "text": payload.to_string()}],
        }),
    )
}

#[tokio::test]
async fn an_approval_pause_is_drawn_and_counts_as_delivered() {
    let recorder = Arc::new(Recorder {
        approvals: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&approval_completed("Merge the PR")).await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;
    assert!(delivery.delivered());
    assert_eq!(
        recorder.calls(),
        vec![Call::Approval("Merge the PR".into())],
        "no end-of-turn notice after a prompt"
    );
}

#[cfg(feature = "otel")]
#[tokio::test]
async fn slack_delivery_traces_distinguish_a_finished_turn_from_delivery_failure() {
    use crate::host::observability::delivery::test_support::DeliveryTraceFixture;

    let trace = DeliveryTraceFixture::new();
    for (failed, stream) in [(false, false), (true, false), (false, true), (true, true)] {
        let carrier = trace.completed_turn_carrier();
        let recorder = Arc::new(Recorder {
            approvals: true,
            reactions: true,
            slack: true,
            stream,
            stop_failure: failed && stream,
            approval_failure: failed.then_some("PRIVATE_ERROR https://slack.example"),
            reply_failure: failed.then_some("PRIVATE_ERROR https://slack.example"),
            ..Recorder::default()
        });
        let mut delivery = delivery(&recorder, DeliveryOptions::default());
        delivery.set_observer(Some(trace.observer()));
        delivery.context.auth_token = "PRIVATE_TOKEN".into();
        // The detached delivery task learns the trusted anchor from the
        // committed turn.start, not from the current thread or raw input.
        let start = DeliveryEvent::from_envelope(&json!({
            "type": TURN_STARTED, "sequence": 1,
            "context": {"input_message_id": TURN},
            "metadata": {(crate::causal::TURN_TRACEPARENT_KEY): carrier}
        }))
        .unwrap();
        delivery.observe(&start).await;
        delivery
            .observe(&approval_completed("PRIVATE_PROMPT"))
            .await;
        if stream {
            delivery.observe(&delta("m1", "PRIVATE_")).await;
        }
        delivery.observe(&completed("m1", "PRIVATE_REPLY")).await;
        delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

        assert!(delivery.is_finished());
        assert!(
            delivery.delivered(),
            "a failed send still allows the existing notice"
        );
        let mut expected = vec![Call::Approval("PRIVATE_PROMPT".into())];
        if stream {
            expected.extend([
                Call::Start,
                Call::Append("PRIVATE_REPLY".into()),
                Call::Stop,
            ]);
        } else {
            expected.push(Call::Post("PRIVATE_REPLY".into()));
            if failed {
                expected.push(Call::Post("The agent finished without a reply.".into()));
            }
        }
        assert_eq!(recorder.calls(), expected);
        trace.assert_deliveries(&carrier, failed);
    }
}

#[tokio::test]
async fn without_an_approval_surface_the_pause_draws_nothing() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&approval_completed("Merge the PR")).await;
    assert!(recorder.calls().is_empty());
}

fn task(id: &str, name: &str, state: &str) -> DeliveryEvent {
    event(
        TASK_UPDATED,
        json!({"task": {"id": id, "display_name": name, "state": state}}),
    )
}

#[tokio::test]
async fn task_progress_posts_once_then_edits_and_ends_final() {
    let recorder = Arc::new(Recorder {
        progress: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&task("t1", "worker-a", "running")).await;
    delivery.observe(&task("t2", "worker-b", "running")).await;
    assert!(delivery.has_pending_work());
    delivery.flush().await;
    assert!(!delivery.has_pending_work());
    delivery.flush().await;
    delivery.observe(&task("t1", "worker-a", "succeeded")).await;
    delivery.flush().await;
    delivery.observe(&event(TURN_COMPLETED, json!({}))).await;

    let calls = recorder.calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    assert!(matches!(&calls[0], Call::ProgressPost(text) if text.contains("0 of 2 finished")));
    assert!(matches!(&calls[1], Call::ProgressUpdate(text) if text.contains("1 of 2 finished")));
    assert!(
        matches!(&calls[2], Call::ProgressUpdate(text) if text.contains("Turn ended")),
        "the final push drops the in-flight framing: {calls:?}"
    );
    assert!(delivery.delivered(), "progress counts as delivered");
}

#[tokio::test]
async fn progress_without_a_surface_is_no_pending_work() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&task("t1", "worker-a", "running")).await;
    assert!(!delivery.has_pending_work());
}

#[tokio::test]
async fn agent_surface_off_shows_no_status_or_title() {
    let recorder = Arc::new(Recorder {
        surface: true,
        ..Recorder::default()
    });
    let options = DeliveryOptions {
        agent_surface: false,
        ..DeliveryOptions::default()
    };
    let mut delivery = delivery(&recorder, options);
    delivery.observe(&event(TURN_STARTED, json!({}))).await;
    delivery
        .observe(&event(SESSION_TITLE_UPDATED, json!({"title": "Revenue"})))
        .await;
    delivery.observe(&completed("m1", "Hi")).await;
    assert_eq!(recorder.calls(), vec![Call::Post("Hi".into())]);
}

fn reaction(message_id: &str, emoji: &str, actor_kind: &str, added: bool) -> DeliveryEvent {
    event(
        if added {
            MESSAGE_REACTION_ADDED
        } else {
            MESSAGE_REACTION_REMOVED
        },
        json!({
            "message_id": message_id,
            "emoji": emoji,
            "actor": {"kind": actor_kind, "id": "x"},
        }),
    )
}

#[tokio::test]
async fn posted_and_streamed_replies_are_linked_to_their_platform_keys() {
    let recorder = Arc::new(Recorder {
        reactions: true,
        stream: true,
        ..Recorder::default()
    });
    let mut streaming = delivery(&recorder, DeliveryOptions::default());
    streaming.observe(&delta("m1", "streamed")).await;
    streaming.observe(&completed("m1", "streamed")).await;
    let streamed = streaming.take_links();
    assert_eq!(streamed, vec![("C1:h1".to_string(), "m1".to_string())]);

    let recorder = Arc::new(Recorder {
        reactions: true,
        ..Recorder::default()
    });
    let mut posting = delivery(&recorder, DeliveryOptions::default());
    posting.observe(&completed("m2", "posted")).await;
    assert_eq!(
        posting.take_links(),
        vec![("C1:posted".to_string(), "m2".to_string())]
    );
    assert!(posting.take_links().is_empty(), "links are taken once");
    assert!(posting.delivered());
}

#[tokio::test]
async fn only_the_agents_reactions_on_known_messages_reach_the_platform() {
    let recorder = Arc::new(Recorder {
        reactions: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.know_message_key(TURN, "C1:input");

    delivery.observe(&reaction(TURN, "👀", "agent", true)).await;
    delivery.observe(&reaction(TURN, "👍", "user", true)).await;
    delivery
        .observe(&reaction("message_unknown", "✅", "agent", true))
        .await;
    delivery
        .observe(&reaction(TURN, "👀", "agent", false))
        .await;

    assert_eq!(
        recorder.calls(),
        vec![
            Call::React("C1:input".into(), "👀".into(), true),
            Call::React("C1:input".into(), "👀".into(), false),
        ]
    );
}

#[tokio::test]
async fn a_platform_without_reactions_keeps_them_inside() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.know_message_key(TURN, "C1:input");
    delivery.observe(&reaction(TURN, "👀", "agent", true)).await;
    delivery.observe(&completed("m1", "reply")).await;
    assert_eq!(recorder.calls(), vec![Call::Post("reply".into())]);
    assert!(delivery.take_links().is_empty());
}

fn view_completed(error: Option<&str>) -> DeliveryEvent {
    event(
        TOOL_COMPLETED,
        json!({
            "tool_call_id": "call_1",
            "tool_name": "mcp_sentry__render_chart",
            "success": error.is_none(),
            "error": error,
            "result": [{"type": "text", "text": "1,284 events"}],
            "details": {"mcp_app": {
                "server": "sentry",
                "tool": "render_chart",
                "resource_uri": "ui://sentry/chart.html",
                "structured_content": {"events": 1284}
            }},
        }),
    )
}

#[tokio::test]
async fn a_tool_view_reaches_the_view_surface_with_its_text() {
    let recorder = Arc::new(Recorder {
        views: true,
        ..Recorder::default()
    });
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&view_completed(None)).await;
    delivery.observe(&view_completed(Some("boom"))).await;
    assert_eq!(
        recorder.calls(),
        vec![Call::View(
            "ui://sentry/chart.html".into(),
            "1,284 events".into()
        )],
        "a failed call shows no view"
    );
    assert!(!delivery.delivered(), "a view is not the agent's reply");
}

#[tokio::test]
async fn without_a_view_surface_a_tool_view_draws_nothing() {
    let recorder = Arc::new(Recorder::default());
    let mut delivery = delivery(&recorder, DeliveryOptions::default());
    delivery.observe(&view_completed(None)).await;
    assert!(recorder.calls().is_empty());
}

#[path = "checkpoint_delivery_tests.rs"]
mod checkpoint_tests;
