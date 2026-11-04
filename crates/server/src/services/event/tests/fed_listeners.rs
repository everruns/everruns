//! Listeners fed from the database (`feed`) see every committed event in
//! session order, and no caller waits for them. In-process listeners
//! (`listener_dispatch`) keep the old rule: the holder of a session actor's
//! lease does not wait for them, other callers do.

use super::*;
use everruns_core::Event;
use sqlx::Connection;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Notify;

/// Records what it saw, holding each event until `gate` opens.
struct HeldListener {
    /// A listener's name decides where it runs (`feed::IN_PROCESS`).
    name: &'static str,
    gate: Notify,
    seen: Mutex<Vec<Option<i32>>>,
}

#[async_trait::async_trait]
impl everruns_core::EventListener for HeldListener {
    async fn on_event(&self, event: &Event) {
        self.gate.notified().await;
        self.seen.lock().unwrap().push(event.sequence);
    }

    fn name(&self) -> &'static str {
        self.name
    }
}

/// Fed from the database: an embedder's listener runs in the near tier.
const FED: &str = "TestListener";
/// Stays in-process.
const IN_PROCESS: &str = "PrometheusMetricsListener";

async fn fixture() -> (
    EventService,
    Arc<HeldListener>,
    SessionId,
    Arc<StorageBackend>,
) {
    fixture_named(FED).await
}

async fn fixture_named(
    name: &'static str,
) -> (
    EventService,
    Arc<HeldListener>,
    SessionId,
    Arc<StorageBackend>,
) {
    let db = Arc::new(StorageBackend::test_database());
    let listener = Arc::new(HeldListener {
        name,
        gate: Notify::new(),
        seen: Mutex::new(Vec::new()),
    });
    let service = EventService::with_listeners(
        db.clone(),
        EventDelivery::in_memory(),
        vec![listener.clone()],
    );
    let agent_id = AgentId::from_uuid(
        db.create_test_agent(DEFAULT_ORG_ID, uuid::Uuid::now_v7())
            .await,
    );
    let session = db
        .create_session(test_session_input(agent_id))
        .await
        .unwrap()
        .id;
    (service, listener, session, db)
}

fn said(session: SessionId, text: &str) -> EventRequest {
    EventRequest::new(
        session,
        EventContext::empty(),
        OutputMessageCompletedData::new(RuntimeMessage::assistant(text)),
    )
}

#[tokio::test]
async fn a_fenced_append_does_not_wait_for_listeners() {
    let (service, listener, session, db) = fixture().await;
    db.mark_session_actor_runnable(session, DEFAULT_ORG_ID, 1)
        .await
        .unwrap();
    let lease = db
        .lease_session_actors("worker-a", 1, Duration::from_secs(60), 32)
        .await
        .unwrap()
        .remove(0);
    let mut live = service
        .event_delivery()
        .subscribe(session.uuid())
        .await
        .unwrap();

    for text in ["one", "two"] {
        tokio::time::timeout(
            Duration::from_secs(5),
            service.emit_batch_fenced(
                vec![said(session, text)],
                ActorAppendFence {
                    holder: "worker-a",
                    fence: lease.fence,
                    detached_tool_call: None,
                },
            ),
        )
        .await
        .expect("a fenced append is answered while its listener is held")
        .unwrap();
    }
    // Live delivery is not a listener: it happened before the answer.
    for expected in [1, 2] {
        let event = tokio::time::timeout(Duration::from_secs(5), live.recv())
            .await
            .expect("published while the listener is held")
            .expect("subscription open");
        assert_eq!(event.sequence, Some(expected));
    }
    assert!(listener.seen.lock().unwrap().is_empty());

    release(&listener, 2).await;
    service.listeners_settled().await;
    assert_eq!(*listener.seen.lock().unwrap(), vec![Some(1), Some(2)]);
}

/// Release the held listener until it saw `count` events.
async fn release(listener: &HeldListener, count: usize) {
    tokio::time::timeout(Duration::from_secs(30), async {
        // `notify_one` stores a single permit, so release one at a time.
        while listener.seen.lock().unwrap().len() < count {
            listener.gate.notify_one();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the listener saw every event");
}

#[tokio::test]
async fn no_caller_waits_for_a_fed_listener() {
    let (service, listener, session, _db) = fixture().await;
    for text in ["one", "two", "three"] {
        tokio::time::timeout(Duration::from_secs(5), service.emit(said(session, text)))
            .await
            .expect("an emit is answered while its listener is held")
            .unwrap();
    }
    release(&listener, 3).await;
    service.listeners_settled().await;
    assert_eq!(
        *listener.seen.lock().unwrap(),
        vec![Some(1), Some(2), Some(3)],
        "every committed event, once, in session order"
    );
}

#[tokio::test]
async fn a_fed_listener_sees_an_event_only_once_its_transaction_commits() {
    let (service, listener, session, db) = fixture().await;
    // A dedicated connection, so the open transaction is ours alone.
    let mut tx = sqlx::PgConnection::connect_with(&db.pool().connect_options())
        .await
        .unwrap();
    sqlx::query("BEGIN").execute(&mut tx).await.unwrap();
    sqlx::query(
        "INSERT INTO events (session_id, sequence, event_type) \
         VALUES ($1, 1, 'output.message.completed')",
    )
    .bind(session.uuid())
    .execute(&mut tx)
    .await
    .unwrap();
    listener.gate.notify_one();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        listener.seen.lock().unwrap().is_empty(),
        "not before the commit"
    );
    sqlx::query("COMMIT").execute(&mut tx).await.unwrap();
    release(&listener, 1).await;
    service.listeners_settled().await;
    assert_eq!(*listener.seen.lock().unwrap(), vec![Some(1)]);
}

#[tokio::test]
async fn an_unfenced_emit_waits_for_its_in_process_listeners() {
    let (service, listener, session, _db) = fixture_named(IN_PROCESS).await;
    let emitting = {
        let service = service.clone();
        tokio::spawn(async move { service.emit(said(session, "hi")).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!emitting.is_finished(), "the caller waits for the listener");
    listener.gate.notify_one();
    tokio::time::timeout(Duration::from_secs(5), emitting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(*listener.seen.lock().unwrap(), vec![Some(1)]);
}
