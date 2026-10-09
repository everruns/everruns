//! Streaming where deltas never reach PostgreSQL (EVE-1211).
//!
//! Under NATS, `output.message.delta` lives only on the delivery bus, so the
//! dispatcher's PostgreSQL read sees completions but no deltas. These tests
//! publish deltas the way NATS would — on the bus only — and persist every
//! durable event to PostgreSQL. The in-memory `EventDelivery` stands in for
//! NATS: both are driven through the same `EventDelivery::subscribe`, and no
//! NATS server runs in unit tests.

use super::super::*;
use super::streaming_tests::{Call, RecordingAdapter, dispatcher_with, recorded};
use super::terminal_state_tests;
use crate::live_updates::event_notifications::EventNotificationPayload;
use tokio::sync::broadcast;

use crate::live_updates::event_delivery::EventDelivery;
use crate::records::agent_channel::DEFAULT_AG_UI_GENERIC_TOOL_TEXT;
use everruns_contracts::typed_id::{MessageId, TurnId};
use everruns_core::events::{EventContext, EventData, OutputMessageDeltaData};
use std::sync::Mutex;
use std::time::Duration;

struct Turn {
    session: SessionId,
    input: MessageId,
}

impl Turn {
    fn input_id(&self) -> String {
        id_string(&self.input)
    }
}

fn id_string(id: &MessageId) -> String {
    serde_json::to_value(id)
        .expect("id serializes")
        .as_str()
        .expect("id is a string")
        .to_string()
}

async fn register_pane(dispatcher: &SlackDeliveryDispatcher, turn: &Turn) {
    dispatcher
        .register(DeliveryRegistration {
            session_id: turn.session.uuid(),
            input_message_id: turn.input_id(),
            bot_token: "xoxb-t".to_string(),
            channel: "D_PANE".to_string(),
            thread_ts: "1700000000.000100".to_string(),
            reply_mode: SlackReplyMode::AllMessages,
            surface: SlackSurface::Pane,
            recipient_user_id: Some("U_HUMAN".to_string()),
            recipient_team_id: Some("T_TEAM".to_string()),
            tool_visibility: PublicToolVisibility::default(),
            generic_tool_text: DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string(),
            approvals_enabled: true,
        })
        .await;
}

/// Publish a delta on the bus only, as `EventService::emit_ephemeral` does.
async fn publish_delta(bus: &EventDelivery, turn: &Turn, message: &MessageId, text: &str) {
    let event = everruns_core::Event {
        id: EventId::new(),
        event_type: events::OUTPUT_MESSAGE_DELTA.to_string(),
        ts: chrono::Utc::now(),
        session_id: turn.session,
        context: EventContext {
            input_message_id: Some(turn.input),
            ..Default::default()
        },
        data: EventData::OutputMessageDelta(OutputMessageDeltaData {
            turn_id: TurnId::new(),
            message_id: *message,
            delta: text.to_string(),
            accumulated: text.to_string(),
            phase: None,
        }),
        metadata: None,
        tags: None,
        sequence: None,
    };
    bus.publish(&event).await.expect("publish");
}

async fn persist(db: &StorageBackend, turn: &Turn, event_type: &str, data: serde_json::Value) {
    terminal_state_tests::emit(db, turn.session, event_type, &turn.input_id(), data).await;
}

fn completed_data(message: &MessageId, text: &str) -> serde_json::Value {
    serde_json::json!({
        "message": { "id": id_string(message), "content": [{ "type": "text", "text": text }] }
    })
}

fn count(calls: &Arc<Mutex<Vec<Call>>>, pred: impl Fn(&Call) -> bool) -> usize {
    recorded(calls).iter().filter(|c| pred(c)).count()
}

/// Wait for `pred`, re-running `nudge` meanwhile. The feed subscribes in a
/// spawned task, so a delta published before it is listening is lost — which
/// is fine in production, where the next delta carries the full text again.
async fn wait_for<F, Fut>(what: &str, mut nudge: F, pred: impl Fn() -> bool)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    tokio::time::timeout(Duration::from_secs(5), async {
        while !pred() {
            nudge().await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

struct Fixture {
    db: Arc<StorageBackend>,
    bus: EventDelivery,
    calls: Arc<Mutex<Vec<Call>>>,
    dispatcher: Arc<SlackDeliveryDispatcher>,
    turn: Turn,
    /// Keeps the event loop running: it stops when its wake source closes, and
    /// takes the feeds with it. Nothing is sent, so sessions are processed only
    /// when a test says so.
    _wake: broadcast::Sender<EventNotificationPayload>,
}

async fn setup() -> Fixture {
    let db = Arc::new(StorageBackend::test_database());
    let session = terminal_state_tests::seed_session(&db).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (wake, rx) = broadcast::channel(16);
    let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
        db.clone(),
        rx,
        "https://app.example.com".to_string(),
        Arc::new(RecordingAdapter::new(calls.clone())),
    );
    let bus = EventDelivery::in_memory();
    dispatcher.live.set_source(bus.clone());
    let turn = Turn {
        session,
        input: MessageId::new(),
    };
    Fixture {
        db,
        bus,
        calls,
        dispatcher,
        turn,
        _wake: wake,
    }
}

#[tokio::test]
async fn bus_only_deltas_stream_and_the_feed_ends_with_the_turn() {
    let Fixture {
        db,
        bus,
        calls,
        dispatcher,
        turn,
        _wake,
    } = setup().await;
    register_pane(&dispatcher, &turn).await;
    assert_eq!(dispatcher.live.running(), 1, "registration subscribes");

    let m1 = MessageId::new();
    wait_for(
        "the first bus delta to open a stream",
        || publish_delta(&bus, &turn, &m1, "Hel"),
        || count(&calls, |c| *c == Call::Start) == 1,
    )
    .await;
    publish_delta(&bus, &turn, &m1, "Hello wor").await;
    wait_for(
        "the dispatcher tick to flush the streamed text",
        || async {},
        || {
            recorded(&calls)
                .iter()
                .any(|c| matches!(c, Call::Append(_, t) if t.ends_with("wor")))
        },
    )
    .await;

    // Completion and turn end come from PostgreSQL, as they do under NATS.
    persist(
        &db,
        &turn,
        "output.message.completed",
        completed_data(&m1, "Hello world"),
    )
    .await;
    persist(&db, &turn, "turn.completed", serde_json::json!({})).await;
    dispatcher.process_session_events(turn.session.uuid()).await;

    let calls = recorded(&calls);
    assert_eq!(calls.last(), Some(&Call::Stop("stream-1".to_string())));
    let appended: String = calls
        .iter()
        .filter_map(|c| match c {
            Call::Append(_, text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(appended, "Hello world", "{calls:?}");
    assert!(
        !calls.iter().any(|c| matches!(c, Call::Discrete(_))),
        "a streamed reply must not also be posted discretely: {calls:?}"
    );
    assert_eq!(dispatcher.live.running(), 0, "unregistering drops the feed");
    dispatcher.shutdown();
}

#[tokio::test]
async fn a_late_delta_does_not_reopen_a_finished_message() {
    let Fixture {
        db,
        bus,
        calls,
        dispatcher,
        turn,
        _wake,
    } = setup().await;
    register_pane(&dispatcher, &turn).await;

    let m1 = MessageId::new();
    wait_for(
        "the first bus delta to open a stream",
        || publish_delta(&bus, &turn, &m1, "first"),
        || count(&calls, |c| *c == Call::Start) == 1,
    )
    .await;
    persist(
        &db,
        &turn,
        "output.message.completed",
        completed_data(&m1, "first answer"),
    )
    .await;
    dispatcher.process_session_events(turn.session.uuid()).await;
    assert_eq!(
        recorded(&calls).last(),
        Some(&Call::Stop("stream-1".to_string()))
    );

    // The bus delivers out of band: a delta for m1 can trail its completion.
    publish_delta(&bus, &turn, &m1, "first answer").await;
    // The turn goes on. The bus is ordered per session, so once m2 has opened
    // its stream the late m1 delta has been handled.
    let m2 = MessageId::new();
    publish_delta(&bus, &turn, &m2, "second").await;
    wait_for(
        "the next message to open its stream",
        || async {},
        || count(&calls, |c| *c == Call::Start) >= 2,
    )
    .await;

    assert_eq!(
        count(&calls, |c| *c == Call::Start),
        2,
        "a finished message must not open a second stream: {:?}",
        recorded(&calls)
    );
    assert_eq!(dispatcher.live.running(), 1, "the turn is still live");
    dispatcher.shutdown();
}

#[tokio::test]
async fn one_feed_per_session_and_shutdown_drops_it() {
    let db = Arc::new(StorageBackend::test_database());
    let session = terminal_state_tests::seed_session(&db).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    // A running event loop, so shutdown is observed.
    let dispatcher = SlackDeliveryDispatcher::start_with_adapter(
        db,
        DeliveryWake::Poll,
        "https://app.example.com".to_string(),
        Arc::new(RecordingAdapter::new(calls)),
    );
    dispatcher.live.set_source(EventDelivery::in_memory());
    let turn = Turn {
        session,
        input: MessageId::new(),
    };
    register_pane(&dispatcher, &turn).await;
    let follow_up = Turn {
        session: turn.session,
        input: MessageId::new(),
    };
    register_pane(&dispatcher, &follow_up).await;
    assert_eq!(
        dispatcher.live.running(),
        1,
        "turns share their session's feed"
    );

    dispatcher.shutdown();
    tokio::time::timeout(Duration::from_secs(5), async {
        while dispatcher.live.running() != 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("shutdown aborts every feed");
}

#[tokio::test]
async fn a_backend_that_persists_deltas_gets_no_feed() {
    let db = Arc::new(StorageBackend::test_database());
    let session = terminal_state_tests::seed_session(&db).await;
    let calls = Arc::new(Mutex::new(Vec::new()));
    let dispatcher = dispatcher_with(db, RecordingAdapter::new(calls)).await;
    // In-memory delivery persists deltas to PostgreSQL, where the dispatcher
    // already reads them; only a backend that skips PostgreSQL is fed.
    dispatcher.feed_live_deltas(&EventDelivery::in_memory());
    let turn = Turn {
        session,
        input: MessageId::new(),
    };
    register_pane(&dispatcher, &turn).await;
    assert_eq!(dispatcher.live.running(), 0);
    dispatcher.shutdown();
}
