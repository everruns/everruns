use std::sync::Mutex;
use std::time::Duration;

use futures::channel::mpsc;
use serde_json::{Value, json};

use super::*;
use crate::channel::{DeliveryContext, DeliveryResult, OutboundChannelMessage};
use everruns_contracts::runtime::events::{OUTPUT_MESSAGE_COMPLETED, TURN_COMPLETED, TURN_STARTED};

/// A session host that records what the channel host asks of it. Tests emit
/// session events by hand.
#[derive(Default)]
struct FakePort {
    state: Mutex<PortState>,
}

#[derive(Default)]
struct PortState {
    created: Vec<NewChannelSession>,
    sent: Vec<(String, InputMessage)>,
    /// Durable events per session, for replay.
    history: HashMap<String, Vec<DeliveryEvent>>,
    subscribers: HashMap<String, Vec<mpsc::UnboundedSender<DeliveryEvent>>>,
    steer: bool,
    discarded: Vec<String>,
    seeded: Vec<(String, Vec<InputMessage>)>,
    controls: Vec<(String, ControlAction)>,
    decisions: Vec<(String, String, bool)>,
}

impl FakePort {
    fn state(&self) -> std::sync::MutexGuard<'_, PortState> {
        self.state.lock().unwrap()
    }

    fn emit(&self, session_id: &str, event: DeliveryEvent) {
        let mut state = self.state();
        state
            .history
            .entry(session_id.to_string())
            .or_default()
            .push(event.clone());
        if let Some(subscribers) = state.subscribers.get_mut(session_id) {
            subscribers.retain(|subscriber| subscriber.unbounded_send(event.clone()).is_ok());
        }
    }

    /// Drop every live subscription, as a host shutting down would.
    fn disconnect(&self) {
        self.state().subscribers.clear();
    }

    fn reply(&self, session_id: &str, turn: &str, text: &str) {
        let base = self.state().history.get(session_id).map_or(0, Vec::len) as i64;
        self.emit(session_id, event(base + 1, turn, TURN_STARTED, json!({})));
        self.emit(
            session_id,
            event(
                base + 2,
                turn,
                OUTPUT_MESSAGE_COMPLETED,
                json!({"message": {"id": format!("out_{turn}"), "content": [{"type": "text", "text": text}]}}),
            ),
        );
        self.emit(session_id, event(base + 3, turn, TURN_COMPLETED, json!({})));
    }
}

#[async_trait]
impl ChannelSessionPort for FakePort {
    async fn create_session(&self, request: NewChannelSession) -> Result<String, ChannelError> {
        let mut state = self.state();
        state.created.push(request);
        Ok(format!("session_{}", state.created.len()))
    }

    async fn send(
        &self,
        _: &ChannelConfig,
        session_id: &str,
        message: InputMessage,
    ) -> Result<SendOutcome, ChannelError> {
        let mut state = self.state();
        state.sent.push((session_id.to_string(), message));
        if state.steer {
            return Ok(SendOutcome::Steered);
        }
        Ok(SendOutcome::Started {
            input_message_id: format!("turn_{}", state.sent.len()),
        })
    }

    async fn discard_session(
        &self,
        _: &ChannelConfig,
        session_id: &str,
    ) -> Result<(), ChannelError> {
        self.state().discarded.push(session_id.to_string());
        Ok(())
    }

    async fn seed(
        &self,
        _: &ChannelConfig,
        session_id: &str,
        history: Vec<InputMessage>,
    ) -> Result<(), ChannelError> {
        self.state().seeded.push((session_id.to_string(), history));
        Ok(())
    }

    async fn control(
        &self,
        _: &ChannelConfig,
        session_id: &str,
        action: &ControlAction,
    ) -> Result<(), ChannelError> {
        self.state()
            .controls
            .push((session_id.to_string(), action.clone()));
        Ok(())
    }

    async fn decide(
        &self,
        _: &ChannelConfig,
        decision: &InboundDecision,
    ) -> Result<SendOutcome, ChannelError> {
        self.state().decisions.push((
            decision.session_id.clone(),
            decision.call_id.clone(),
            decision.approved,
        ));
        Ok(SendOutcome::Started {
            input_message_id: "turn_resumed".into(),
        })
    }

    async fn events(
        &self,
        _: &ChannelConfig,
        session_id: &str,
        after: Option<i64>,
    ) -> Result<ChannelEventStream, ChannelError> {
        let (sender, receiver) = mpsc::unbounded();
        let mut state = self.state();
        if let Some(after) = after {
            for event in state.history.get(session_id).into_iter().flatten() {
                if event.sequence.is_some_and(|sequence| sequence > after) {
                    let _ = sender.unbounded_send(event.clone());
                }
            }
        }
        state
            .subscribers
            .entry(session_id.to_string())
            .or_default()
            .push(sender);
        Ok(receiver.boxed())
    }
}

/// Takes `{"text", "thread", "id"}`, records every reply it is asked to post.
/// `{"control": "cancel" | "rename", "thread"}` is a control event and
/// `{"decide": "<call>", "session", "approved"}` a decision.
#[derive(Default)]
struct TestDriver {
    posts: Mutex<Vec<(String, String)>>,
    /// Texts the driver declines in `admit`.
    declined: Mutex<Vec<String>>,
    /// Earlier messages `backfill` returns.
    history: Mutex<Vec<String>>,
    decided: Mutex<Vec<String>>,
}

impl TestDriver {
    fn posts(&self) -> Vec<(String, String)> {
        self.posts.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelDeliveryAdapter for TestDriver {
    fn platform(&self) -> &str {
        "test"
    }
    async fn deliver(
        &self,
        message: &OutboundChannelMessage,
        _: &DeliveryContext,
    ) -> DeliveryResult {
        self.posts
            .lock()
            .unwrap()
            .push((message.thread_ref.clone(), message.text.clone()));
        DeliveryResult::Ok
    }
    async fn send_ack(&self, _: &str, _: &str, _: &DeliveryContext) -> DeliveryResult {
        DeliveryResult::Ok
    }
}

#[async_trait]
impl ChannelDriver for TestDriver {
    async fn receive(&self, request: &ChannelRequest) -> Result<Inbound, ChannelError> {
        let body = request.body_json()?;
        let field = |name: &str| {
            body.get(name)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        if field("text") == "ping" {
            return Ok(Inbound::Respond(ChannelResponse::ok(json!({"pong": true}))));
        }
        let thread = field("thread");
        let route = || InboundRoute {
            actor: actor("u1"),
            thread_ref: (!thread.is_empty()).then(|| thread.clone()),
            routing_metadata: HashMap::new(),
            reply_to: DeliveryTarget::new("C1", thread.clone()),
        };
        match field("control").as_str() {
            "" => {}
            "cancel" => {
                return Ok(Inbound::Control(Box::new(InboundControl {
                    route: route(),
                    action: ControlAction::Cancel,
                })));
            }
            _ => {
                return Ok(Inbound::Control(Box::new(InboundControl {
                    route: route(),
                    action: ControlAction::Rename {
                        title: field("text"),
                    },
                })));
            }
        }
        if !field("decide").is_empty() {
            return Ok(Inbound::Decision(Box::new(InboundDecision {
                session_id: field("session"),
                call_id: field("decide"),
                approved: body["approved"].as_bool().unwrap_or(false),
                reason: None,
                actor: actor("u2"),
                reply_to: DeliveryTarget::new("C1", thread.clone()),
            })));
        }
        let mut message_metadata = serde_json::Map::new();
        message_metadata.insert("test_ts".into(), json!(field("id")));
        Ok(Inbound::Message(Box::new(InboundMessage {
            event: InboundChannelEvent {
                actor: crate::channel::ExternalActor {
                    actor_id: "u1".into(),
                    actor_name: None,
                    source: "test".into(),
                    metadata: None,
                },
                text: field("text"),
                attachments: vec![InboundAttachment::FileDescription {
                    name: "notes.txt".into(),
                    mime_type: None,
                }],
                dedup_key: field("id"),
                thread_ref: (!thread.is_empty()).then(|| thread.clone()),
                message_metadata,
                routing_metadata: HashMap::new(),
            },
            reply_to: DeliveryTarget::new("C1", thread),
        })))
    }

    async fn admit(&self, message: &InboundMessage, _: Option<&str>) -> Result<bool, ChannelError> {
        Ok(!self.declined.lock().unwrap().contains(&message.event.text))
    }

    async fn backfill(&self, _: &InboundMessage) -> Result<Vec<InboundChannelEvent>, ChannelError> {
        Ok(self
            .history
            .lock()
            .unwrap()
            .iter()
            .map(|text| InboundChannelEvent {
                actor: actor("u0"),
                text: text.clone(),
                attachments: Vec::new(),
                dedup_key: String::new(),
                thread_ref: None,
                routing_metadata: HashMap::new(),
                message_metadata: Default::default(),
            })
            .collect())
    }

    async fn decided(&self, decision: &InboundDecision, _: &DeliveryContext) {
        self.decided.lock().unwrap().push(decision.call_id.clone());
    }
}

fn actor(id: &str) -> crate::channel::ExternalActor {
    crate::channel::ExternalActor {
        actor_id: id.into(),
        actor_name: None,
        source: "test".into(),
        metadata: None,
    }
}

fn event(sequence: i64, turn: &str, event_type: &str, data: Value) -> DeliveryEvent {
    DeliveryEvent {
        sequence: Some(sequence),
        event_type: event_type.into(),
        data,
        input_message_id: Some(turn.into()),
        turn_id: None,
    }
}

fn message(text: &str, thread: &str, id: &str) -> ChannelRequest {
    ChannelRequest::json(&json!({"text": text, "thread": thread, "id": id}))
}

struct Fixture {
    port: Arc<FakePort>,
    driver: Arc<TestDriver>,
    store: Arc<MemoryChannelStore>,
    host: ChannelHost,
}

fn fixture(binding: SessionBinding) -> Fixture {
    fixture_with(
        Arc::new(FakePort::default()),
        Arc::new(MemoryChannelStore::new()),
        binding,
    )
}

fn fixture_with(
    port: Arc<FakePort>,
    store: Arc<MemoryChannelStore>,
    binding: SessionBinding,
) -> Fixture {
    let driver = Arc::new(TestDriver::default());
    let mut config = ChannelConfig::new("support");
    config.binding = binding;
    config.delivery.stream = false;
    let host = ChannelHost::builder(port.clone())
        .store(store.clone())
        .channel(config, driver.clone())
        .flush_interval(Duration::from_millis(10))
        .build();
    Fixture {
        port,
        driver,
        store,
        host,
    }
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn a_message_starts_a_turn_and_its_reply_reaches_the_thread() {
    let f = fixture(SessionBinding::Thread);
    let response = f
        .host
        .handle("support", &message("hello", "t1", "m1"))
        .await;
    assert_eq!(response.status, 200);
    assert_eq!(
        response.body,
        json!({"ok": true, "session_id": "session_1", "created": true})
    );

    {
        let state = f.port.state();
        assert_eq!(state.created.len(), 1);
        assert_eq!(state.created[0].title, "hello");
        assert_eq!(state.created[0].metadata["channel"], "support");
        let (session, sent) = &state.sent[0];
        assert_eq!(session, "session_1");
        assert_eq!(sent.metadata.as_ref().unwrap()["channel"], "support");
        assert_eq!(sent.external_actor.as_ref().unwrap().actor_id, "u1");
        assert_eq!(
            sent.content.len(),
            2,
            "text plus the attached file description"
        );
    }

    f.port.reply("session_1", "turn_1", "hi there");
    eventually("the reply", || {
        f.driver.posts() == vec![("t1".into(), "hi there".into())]
    })
    .await;
    eventually("the pending delivery to clear", || {
        futures::executor::block_on(f.store.claim_pending("test", DEFAULT_DELIVERY_LEASE))
            .unwrap()
            .is_empty()
    })
    .await;
}

#[tokio::test]
async fn a_channel_without_replies_runs_the_turn_and_posts_nothing() {
    let port = Arc::new(FakePort::default());
    let store = Arc::new(MemoryChannelStore::new());
    let driver = Arc::new(TestDriver::default());
    let mut config = ChannelConfig::new("hook");
    config.delivery.replies = false;
    let host = ChannelHost::builder(port.clone())
        .store(store.clone())
        .channel(config, driver.clone())
        .build();
    let first = host.handle("hook", &message("one", "t1", "m1")).await;
    assert_eq!(first.body["created"], true);
    let second = host.handle("hook", &message("two", "t1", "m2")).await;
    assert_eq!(second.body["created"], false);
    assert_eq!(port.state().sent.len(), 2);

    port.reply("session_1", "turn_1", "answer");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(driver.posts().is_empty());
    assert!(
        store
            .claim_pending("test", DEFAULT_DELIVERY_LEASE)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_platform_retry_does_not_start_a_second_turn() {
    let f = fixture(SessionBinding::Thread);
    f.host
        .handle("support", &message("hello", "t1", "m1"))
        .await;
    let retry = f
        .host
        .handle("support", &message("hello", "t1", "m1"))
        .await;
    assert_eq!(retry.body["duplicate"], true);
    assert_eq!(f.port.state().sent.len(), 1);
}

#[tokio::test]
async fn a_thread_keeps_its_session_and_another_thread_gets_a_new_one() {
    let f = fixture(SessionBinding::Thread);
    f.host.handle("support", &message("one", "t1", "m1")).await;
    f.host.handle("support", &message("two", "t1", "m2")).await;
    f.host
        .handle("support", &message("three", "t2", "m3"))
        .await;
    let state = f.port.state();
    let sessions: Vec<&str> = state
        .sent
        .iter()
        .map(|(session, _)| session.as_str())
        .collect();
    assert_eq!(sessions, vec!["session_1", "session_1", "session_2"]);
    assert_eq!(state.created.len(), 2);
    assert!(state.created[0].binding_key.is_some());
}

#[tokio::test]
async fn an_ephemeral_channel_gets_a_session_per_message() {
    let f = fixture(SessionBinding::Ephemeral);
    f.host.handle("support", &message("one", "t1", "m1")).await;
    f.host.handle("support", &message("two", "t1", "m2")).await;
    let state = f.port.state();
    assert_eq!(state.created.len(), 2);
    assert!(
        state
            .created
            .iter()
            .all(|created| created.binding_key.is_none())
    );
}

#[tokio::test]
async fn a_message_that_steers_a_running_turn_gets_no_delivery_of_its_own() {
    let f = fixture(SessionBinding::Thread);
    f.port.state().steer = true;
    f.host
        .handle("support", &message("also this", "t1", "m1"))
        .await;
    f.port.reply("session_1", "turn_1", "answer");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(f.driver.posts().is_empty());
}

#[tokio::test]
async fn driver_answers_and_unknown_channels_come_back_as_responses() {
    let f = fixture(SessionBinding::Thread);
    let pong = f.host.handle("support", &message("ping", "", "")).await;
    assert_eq!(pong.body, json!({"pong": true}));
    assert!(f.port.state().created.is_empty());

    let missing = f.host.handle("nope", &message("hello", "t1", "m1")).await;
    assert_eq!(missing.status, 404);
}

#[tokio::test]
async fn a_delivery_cut_short_is_finished_by_recover() {
    let port = Arc::new(FakePort::default());
    let store = Arc::new(MemoryChannelStore::new());
    let first = fixture_with(port.clone(), store.clone(), SessionBinding::Thread);
    first
        .host
        .handle("support", &message("hello", "t1", "m1"))
        .await;

    // The turn starts, then the process goes away before it ends.
    port.emit("session_1", event(1, "turn_1", TURN_STARTED, json!({})));
    eventually("the pending delivery", || {
        futures::executor::block_on(store.claim_pending("test", DEFAULT_DELIVERY_LEASE))
            .unwrap()
            .len()
            == 1
    })
    .await;
    port.disconnect();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let pending = store
        .claim_pending("test", DEFAULT_DELIVERY_LEASE)
        .await
        .unwrap();
    assert_eq!(
        pending.len(),
        1,
        "an unfinished turn keeps its pending delivery"
    );
    assert_eq!(pending[0].after_sequence, 0);
    assert_eq!(pending[0].target.thread_ref, "t1");

    // The turn finished while nobody was listening.
    port.emit(
        "session_1",
        event(
            2,
            "turn_1",
            OUTPUT_MESSAGE_COMPLETED,
            json!({"message": {"id": "o1", "content": [{"type": "text", "text": "late answer"}]}}),
        ),
    );
    port.emit("session_1", event(3, "turn_1", TURN_COMPLETED, json!({})));

    let second = fixture_with(port.clone(), store.clone(), SessionBinding::Thread);
    assert_eq!(second.host.recover().await.unwrap(), 1);
    eventually("the recovered reply", || {
        second.driver.posts() == vec![("t1".into(), "late answer".into())]
    })
    .await;
    eventually("the pending delivery to clear", || {
        futures::executor::block_on(store.claim_pending("test", DEFAULT_DELIVERY_LEASE))
            .unwrap()
            .is_empty()
    })
    .await;
}

#[tokio::test]
async fn recover_drops_deliveries_of_channels_no_longer_configured() {
    let f = fixture(SessionBinding::Thread);
    f.store
        .save_pending(
            &PendingDelivery {
                channel: "retired".into(),
                session_id: "session_9".into(),
                input_message_id: "turn_9".into(),
                target: DeliveryTarget::new("C1", "t1"),
                after_sequence: 0,
            },
            "old",
            DEFAULT_DELIVERY_LEASE,
        )
        .await
        .unwrap();
    assert_eq!(f.host.recover().await.unwrap(), 0);
    assert!(
        f.store
            .claim_pending("test", DEFAULT_DELIVERY_LEASE)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn start_opens_a_conversation_from_this_side() {
    let f = fixture(SessionBinding::Thread);
    let session = f
        .host
        .start(
            "support",
            DeliveryTarget::new("C1", "daily"),
            "Post the morning summary",
        )
        .await
        .unwrap();
    assert_eq!(session, "session_1");
    assert!(f.port.state().created[0].binding_key.is_none());
    f.port.reply("session_1", "turn_1", "summary");
    eventually("the reply", || {
        f.driver.posts() == vec![("daily".into(), "summary".into())]
    })
    .await;
}

#[test]
fn titles_take_the_first_line() {
    assert_eq!(title_for("first\nsecond"), "first");
    assert_eq!(title_for("   "), "Channel conversation");
    assert_eq!(title_for(&"x".repeat(200)).len(), 80);
}

#[tokio::test]
async fn send_delivers_a_turn_on_a_session_the_host_picked() {
    let f = fixture(SessionBinding::Thread);
    let outcome = f
        .host
        .send(
            "support",
            "session_7",
            InputMessage::user("status?"),
            DeliveryTarget::new("C1", "ops"),
        )
        .await
        .unwrap();
    assert_eq!(
        outcome,
        SendOutcome::Started {
            input_message_id: "turn_1".into()
        }
    );
    assert_eq!(
        f.port.state().sent[0].1.metadata.as_ref().unwrap()["channel"],
        "support"
    );
    f.port.reply("session_7", "turn_1", "all good");
    eventually("the reply", || {
        f.driver.posts() == vec![("ops".into(), "all good".into())]
    })
    .await;

    let missing = f
        .host
        .send(
            "nope",
            "session_7",
            InputMessage::user("x"),
            DeliveryTarget::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(missing.status(), 404);
}

fn streaming_host(port: Arc<FakePort>) -> ChannelHost {
    let mut config = ChannelConfig::new("ag-ui:helper");
    config.agent = Some("helper".into());
    ChannelHost::builder(port)
        .stream_channel(config, "ag-ui")
        .build()
}

#[tokio::test]
async fn a_streaming_conversation_binds_one_session_per_key() {
    let port = Arc::new(FakePort::default());
    let host = streaming_host(port.clone());

    let first = host
        .conversation("ag-ui:helper", "thread-1", "Hello")
        .await
        .unwrap();
    let again = host
        .conversation("ag-ui:helper", "thread-1", "Hello again")
        .await
        .unwrap();
    let other = host
        .conversation("ag-ui:helper", "thread-2", "Hi")
        .await
        .unwrap();

    assert!(first.created);
    assert_eq!(
        again,
        Conversation {
            session_id: first.session_id.clone(),
            created: false
        }
    );
    assert_ne!(other.session_id, first.session_id);
    let created = &port.state().created;
    assert_eq!(created.len(), 2);
    assert_eq!(created[0].channel.agent.as_deref(), Some("helper"));
    assert_eq!(created[0].metadata["kind"], "ag-ui");
    assert_eq!(created[0].binding_key.as_deref(), Some("thread-1"));
}

#[tokio::test]
async fn a_stream_turn_yields_its_own_events_and_ends_at_the_turn_end() {
    let port = Arc::new(FakePort::default());
    let host = streaming_host(port.clone());
    let session = host
        .conversation("ag-ui:helper", "thread-1", "Hello")
        .await
        .unwrap()
        .session_id;

    let turn = host
        .stream_turn("ag-ui:helper", &session, InputMessage::user("hello"))
        .await
        .unwrap();
    assert_eq!(
        turn.outcome,
        SendOutcome::Started {
            input_message_id: "turn_1".into()
        }
    );
    // Another turn's event, an ephemeral delta, then this turn's reply.
    port.emit(&session, event(1, "turn_0", TURN_COMPLETED, json!({})));
    port.emit(
        &session,
        DeliveryEvent {
            sequence: None,
            event_type: "output.message.delta".into(),
            data: json!({"delta": "hi"}),
            input_message_id: None,
            turn_id: None,
        },
    );
    port.reply(&session, "turn_1", "hi there");
    port.emit(&session, event(99, "turn_1", TURN_STARTED, json!({})));

    let seen: Vec<String> = tokio::time::timeout(
        Duration::from_secs(2),
        turn.events.map(|event| event.event_type).collect(),
    )
    .await
    .unwrap();
    assert_eq!(
        seen,
        [
            "output.message.delta",
            TURN_STARTED,
            OUTPUT_MESSAGE_COMPLETED,
            TURN_COMPLETED
        ]
    );
    let sent = &port.state().sent;
    assert_eq!(
        sent[0].1.metadata.as_ref().unwrap()["channel"],
        "ag-ui:helper"
    );
}

#[tokio::test]
async fn messaging_and_streaming_channels_do_not_share_names() {
    let host = streaming_host(Arc::new(FakePort::default()));
    assert_eq!(host.stream_channel_names(), ["ag-ui:helper"]);
    assert!(host.channel_names().is_empty());
    let missing = host
        .conversation("support", "thread-1", "Hello")
        .await
        .unwrap_err();
    assert_eq!(missing.status(), 404);
    let missing = host
        .try_handle("ag-ui:helper", &message("hi", "t", "1"))
        .await
        .unwrap_err();
    assert_eq!(missing.status(), 404);
}

#[tokio::test]
async fn driver_message_metadata_lands_on_the_input_message() {
    let f = fixture(SessionBinding::Thread);
    f.host
        .handle("support", &message("hello", "t1", "m1"))
        .await;
    let state = f.port.state();
    assert_eq!(state.sent[0].1.metadata.as_ref().unwrap()["test_ts"], "m1");
}

#[tokio::test]
async fn a_message_the_driver_declines_starts_nothing() {
    let f = fixture(SessionBinding::Thread);
    f.driver.declined.lock().unwrap().push("chatter".into());
    let response = f
        .host
        .handle("support", &message("chatter", "t1", "m1"))
        .await;
    assert_eq!(response.body, json!({"ok": true}));
    let state = f.port.state();
    assert!(state.created.is_empty());
    assert!(state.sent.is_empty());
}

#[tokio::test]
async fn a_new_session_for_a_thread_under_way_is_seeded_with_its_history() {
    let f = fixture(SessionBinding::Thread);
    *f.driver.history.lock().unwrap() = vec!["earlier one".into(), "earlier two".into()];
    f.host.handle("support", &message("now", "t1", "m1")).await;
    f.host
        .handle("support", &message("later", "t1", "m2"))
        .await;
    let state = f.port.state();
    assert_eq!(state.seeded.len(), 1, "only a new session is seeded");
    let (session, history) = &state.seeded[0];
    assert_eq!(session, "session_1");
    let texts: Vec<&str> = history
        .iter()
        .filter_map(|message| message.content.first()?.as_text())
        .collect();
    assert_eq!(texts, ["earlier one", "earlier two"]);
    assert_eq!(history[0].metadata.as_ref().unwrap()["channel"], "support");
}

/// A store whose binding was won by another process between the lookup and
/// the bind.
struct RacedStore {
    inner: MemoryChannelStore,
}

#[async_trait]
impl ChannelStore for RacedStore {
    async fn session_for(&self, _: &str, _: &str) -> Result<Option<String>, ChannelError> {
        Ok(None)
    }
    async fn bind(&self, _: &str, _: &str, _: &str) -> Result<String, ChannelError> {
        Ok("session_winner".into())
    }
    async fn first_sighting(&self, channel: &str, key: &str) -> Result<bool, ChannelError> {
        self.inner.first_sighting(channel, key).await
    }
    async fn save_pending(
        &self,
        pending: &PendingDelivery,
        holder: &str,
        lease: Duration,
    ) -> Result<(), ChannelError> {
        self.inner.save_pending(pending, holder, lease).await
    }
    async fn clear_pending(&self, session: &str, input: &str) -> Result<(), ChannelError> {
        self.inner.clear_pending(session, input).await
    }
    async fn claim_pending(
        &self,
        holder: &str,
        lease: Duration,
    ) -> Result<Vec<PendingDelivery>, ChannelError> {
        self.inner.claim_pending(holder, lease).await
    }
}

#[tokio::test]
async fn a_session_that_lost_its_binding_race_is_discarded() {
    let port = Arc::new(FakePort::default());
    let driver = Arc::new(TestDriver::default());
    *driver.history.lock().unwrap() = vec!["earlier".into()];
    let host = ChannelHost::builder(port.clone())
        .store(Arc::new(RacedStore {
            inner: MemoryChannelStore::new(),
        }))
        .channel(ChannelConfig::new("support"), driver)
        .build();
    let response = host.handle("support", &message("hello", "t1", "m1")).await;
    assert_eq!(response.body["session_id"], "session_winner");
    let state = port.state();
    assert_eq!(state.discarded, ["session_1"]);
    assert_eq!(state.sent[0].0, "session_winner");
    assert!(
        state.seeded.is_empty(),
        "the winner was seeded by its creator"
    );
}

#[tokio::test]
async fn a_control_event_reaches_the_bound_session_only() {
    let f = fixture(SessionBinding::Thread);
    let ignored = f
        .host
        .handle(
            "support",
            &ChannelRequest::json(&json!({"control": "cancel", "thread": "t1"})),
        )
        .await;
    assert_eq!(ignored.body, json!({"ok": true}));
    assert!(f.port.state().controls.is_empty());

    f.host
        .handle("support", &message("hello", "t1", "m1"))
        .await;
    f.host
        .handle(
            "support",
            &ChannelRequest::json(&json!({"control": "rename", "thread": "t1", "text": "Plans"})),
        )
        .await;
    assert_eq!(
        f.port.state().controls,
        [(
            "session_1".to_string(),
            ControlAction::Rename {
                title: "Plans".into()
            }
        )]
    );
}

#[tokio::test]
async fn a_decision_resumes_the_turn_and_its_replies_are_delivered() {
    let f = fixture(SessionBinding::Thread);
    let response = f
        .host
        .handle(
            "support",
            &ChannelRequest::json(
                &json!({"decide": "call_1", "session": "session_4", "approved": true, "thread": "t9"}),
            ),
        )
        .await;
    assert_eq!(response.body["session_id"], "session_4");
    assert_eq!(
        f.port.state().decisions,
        [("session_4".to_string(), "call_1".to_string(), true)]
    );
    assert_eq!(*f.driver.decided.lock().unwrap(), ["call_1"]);
    f.port.reply("session_4", "turn_resumed", "done");
    eventually("the resumed turn's reply", || {
        f.driver.posts() == vec![("t9".into(), "done".into())]
    })
    .await;
}

/// Channels found per call, as a host with channel rows has them.
struct Rows {
    driver: Arc<TestDriver>,
    unavailable: bool,
}

#[async_trait]
impl ChannelResolver for Rows {
    async fn resolve(&self, name: &str) -> Result<Option<ResolvedChannel>, ChannelError> {
        if self.unavailable {
            return Err(ChannelError::Session("database is down".into()));
        }
        Ok(name.starts_with("row_").then(|| {
            let mut config = ChannelConfig::new(name);
            config.scope = ChannelScope::new(format!("tenant of {name}"));
            config.delivery.stream = false;
            ResolvedChannel::messaging(config, self.driver.clone())
        }))
    }
}

#[tokio::test]
async fn a_resolved_channel_runs_like_a_built_in_one_and_carries_its_scope() {
    let port = Arc::new(FakePort::default());
    let driver = Arc::new(TestDriver::default());
    let host = ChannelHost::builder(port.clone())
        .resolver(Arc::new(Rows {
            driver: driver.clone(),
            unavailable: false,
        }))
        .build();
    let response = host.handle("row_1", &message("hello", "t1", "m1")).await;
    assert_eq!(response.status, 200);
    assert_eq!(
        port.state().created[0]
            .channel
            .scope
            .get::<String>()
            .unwrap(),
        "tenant of row_1"
    );
    port.reply("session_1", "turn_1", "hi");
    eventually("the reply", || {
        driver.posts() == vec![("t1".into(), "hi".into())]
    })
    .await;

    let missing = host.handle("other", &message("hello", "t1", "m2")).await;
    assert_eq!(missing.status, 404);

    // A caller that resolved and authorized the channel itself passes it in.
    let resolved = host.resolve("row_2").await.unwrap();
    let response = host.handle(&resolved, &message("hey", "t2", "m3")).await;
    assert_eq!(response.status, 200);
}

#[tokio::test]
async fn recover_keeps_deliveries_whose_channel_cannot_be_read_now() {
    let store = Arc::new(MemoryChannelStore::new());
    store
        .save_pending(
            &PendingDelivery {
                channel: "row_1".into(),
                session_id: "session_1".into(),
                input_message_id: "turn_1".into(),
                target: DeliveryTarget::new("C1", "t1"),
                after_sequence: 0,
            },
            "old",
            DEFAULT_DELIVERY_LEASE,
        )
        .await
        .unwrap();
    let host = |unavailable| {
        ChannelHost::builder(Arc::new(FakePort::default()))
            .store(store.clone())
            .resolver(Arc::new(Rows {
                driver: Arc::new(TestDriver::default()),
                unavailable,
            }))
            .build()
    };
    assert_eq!(host(true).recover().await.unwrap(), 0);
    assert_eq!(
        store
            .claim_pending("test", DEFAULT_DELIVERY_LEASE)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(host(false).recover().await.unwrap(), 1);
}

#[tokio::test]
async fn a_stream_turn_ends_where_the_turn_parks_and_follow_picks_it_up() {
    let port = Arc::new(FakePort::default());
    let host = streaming_host(port.clone());
    let session = host
        .conversation("ag-ui:helper", "thread-1", "Hello")
        .await
        .unwrap()
        .session_id;
    let turn = host
        .stream_turn("ag-ui:helper", &session, InputMessage::user("approve?"))
        .await
        .unwrap();
    port.emit(&session, event(1, "turn_1", TURN_STARTED, json!({})));
    port.emit(&session, event(2, "turn_1", TOOL_CALL_REQUESTED, json!({})));
    let seen: Vec<String> = turn.events.map(|event| event.event_type).collect().await;
    assert_eq!(seen, [TURN_STARTED, TOOL_CALL_REQUESTED]);

    // The caller answered; the resumed turn's events come without a send.
    let follow = host.follow("ag-ui:helper", &session).await.unwrap();
    port.emit(&session, event(3, "turn_1", TURN_COMPLETED, json!({})));
    let seen: Vec<String> = follow.map(|event| event.event_type).collect().await;
    assert_eq!(seen, [TURN_COMPLETED]);
    assert_eq!(port.state().sent.len(), 1);
}
