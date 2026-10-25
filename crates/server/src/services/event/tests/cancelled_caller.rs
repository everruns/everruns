//! A stored event reaches live subscribers even when the emitting caller is
//! dropped after the insert, as a worker's gRPC handler is when the worker
//! dies mid-RPC (chaos harness: the event was in the log, but an SSE client
//! never saw it and resumed past it).

use super::*;
use everruns_core::Event;
use std::sync::Mutex;
use tokio::sync::{Notify, oneshot};

/// Holds the first event it hears until `gate` opens.
struct GatedListener {
    entered: Mutex<Option<oneshot::Sender<()>>>,
    gate: Arc<Notify>,
}

#[async_trait::async_trait]
impl everruns_core::EventListener for GatedListener {
    async fn on_event(&self, _event: &Event) {
        let entered = self.entered.lock().unwrap().take();
        if let Some(entered) = entered {
            let _ = entered.send(());
            self.gate.notified().await;
        }
    }
}

#[tokio::test]
async fn stored_events_are_published_after_the_caller_is_dropped() {
    let db = Arc::new(StorageBackend::test_database());
    let delivery = EventDelivery::in_memory();
    let (entered_tx, entered_rx) = oneshot::channel();
    let gate = Arc::new(Notify::new());
    let listener = Arc::new(GatedListener {
        entered: Mutex::new(Some(entered_tx)),
        gate: gate.clone(),
    });
    let event_service = EventService::with_listeners(db.clone(), delivery.clone(), vec![listener]);
    let agent_id = AgentId::from_uuid(
        db.create_test_agent(DEFAULT_ORG_ID, uuid::Uuid::now_v7())
            .await,
    );
    let session = db
        .create_session(test_session_input(agent_id))
        .await
        .unwrap()
        .id;
    let mut live = delivery.subscribe(session.uuid()).await.unwrap();
    let said = |text: &str| {
        EventRequest::new(
            session,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant(text)),
        )
    };

    // Both events are stored in one insert; delivery of the first is held
    // in its listener when the caller goes away.
    let emitting = {
        let event_service = event_service.clone();
        let batch = vec![said("one"), said("two")];
        tokio::spawn(async move { event_service.emit_batch_returning_last(batch).await })
    };
    entered_rx.await.unwrap();
    emitting.abort();
    assert!(emitting.await.unwrap_err().is_cancelled());
    gate.notify_one();

    let mut sequences = Vec::new();
    while sequences.len() < 2 {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), live.recv())
            .await
            .expect("every stored event must still be published")
            .expect("subscription open");
        sequences.push(event.sequence);
    }
    assert_eq!(sequences, vec![Some(1), Some(2)]);
}
