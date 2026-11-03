//! The holder of a session actor's lease is answered once its events are
//! stored and published live, without waiting for listeners; other callers
//! still wait for theirs (`listener_dispatch`).

use super::*;
use everruns_core::Event;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Notify;

/// Records what it saw, holding each event until `gate` opens.
#[derive(Default)]
struct HeldListener {
    gate: Notify,
    seen: Mutex<Vec<Option<i32>>>,
}

#[async_trait::async_trait]
impl everruns_core::EventListener for HeldListener {
    async fn on_event(&self, event: &Event) {
        self.gate.notified().await;
        self.seen.lock().unwrap().push(event.sequence);
    }
}

async fn fixture() -> (
    EventService,
    Arc<HeldListener>,
    SessionId,
    Arc<StorageBackend>,
) {
    let db = Arc::new(StorageBackend::test_database());
    let listener = Arc::new(HeldListener::default());
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

    // `notify_one` stores a single permit, so release one event at a time.
    while listener.seen.lock().unwrap().len() < 2 {
        listener.gate.notify_one();
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    service.listeners_settled().await;
    assert_eq!(*listener.seen.lock().unwrap(), vec![Some(1), Some(2)]);
}

#[tokio::test]
async fn an_unfenced_emit_waits_for_its_listeners() {
    let (service, listener, session, _db) = fixture().await;
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
