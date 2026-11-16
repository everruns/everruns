//! Recover a startup polling fallback without restarting the server.
//!
//! Existing polling subscriptions close after recovery. Their SSE clients
//! reconnect using the normal durable cursor replay/deduplication path, then
//! subscribe to NATS before replay. This avoids switching a live stream's
//! source mid-replay. Ephemeral deltas remain best-effort during reconnect.

use super::{EventSubscription, NatsEventDelivery, PollingEventDelivery};
use crate::storage::StorageBackend;
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tracing::{info, warn};
use uuid::Uuid;

const RETRY_INTERVAL: Duration = Duration::from_secs(30);
const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);

pub struct RecoveringEventDelivery {
    polling: PollingEventDelivery,
    recovered: watch::Sender<Option<Arc<NatsEventDelivery>>>,
}

impl RecoveringEventDelivery {
    pub(super) fn start(db: Arc<StorageBackend>, url: String) -> Arc<Self> {
        Self::start_with_interval(db, url, RETRY_INTERVAL)
    }

    fn start_with_interval(db: Arc<StorageBackend>, url: String, interval: Duration) -> Arc<Self> {
        let inner = Arc::new(Self {
            polling: PollingEventDelivery::new(db),
            recovered: watch::channel(None).0,
        });
        let weak = Arc::downgrade(&inner);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                if weak.strong_count() == 0 {
                    return;
                }
                let attempt =
                    tokio::time::timeout(ATTEMPT_TIMEOUT, NatsEventDelivery::connect(&url)).await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                match attempt {
                    Ok(Ok(nats)) => {
                        inner.recovered.send_replace(Some(Arc::new(nats)));
                        info!("Event delivery recovered to NATS; polling streams will reconnect");
                        return;
                    }
                    Ok(Err(error)) => {
                        warn!(error = %crate::live_updates::nats::error_chain(&error), "NATS recovery failed; continuing PostgreSQL polling")
                    }
                    Err(_) => warn!("NATS recovery timed out; continuing PostgreSQL polling"),
                }
            }
        });
        inner
    }

    pub(super) fn nats(&self) -> Option<Arc<NatsEventDelivery>> {
        self.recovered.borrow().clone()
    }

    pub(super) async fn subscribe(&self, session_id: Uuid) -> anyhow::Result<EventSubscription> {
        if let Some(nats) = self.nats() {
            return nats.subscribe(session_id).await;
        }
        // Subscribe to the state before awaiting the DB cursor: recovery during
        // that read must close this polling stream too, not strand it forever.
        let recovered = self.recovered.subscribe();
        let EventSubscription::Polling(polling) = self.polling.subscribe(session_id).await? else {
            unreachable!("polling backend returns a polling subscription")
        };
        Ok(EventSubscription::RecoveringPolling { polling, recovered })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{EventDelivery, tests::TestNatsServer};
    use super::*;
    use crate::api::events::{SessionEventStream, session_event_sse, sse_frame};
    use crate::api::sse::{SseConnectionLimits, SseConnectionTracker};
    use crate::services::EventService;
    use axum::response::IntoResponse;
    use everruns_core::{EventContext, EventRequest, OutputMessageCompletedData, RuntimeMessage};
    use futures::StreamExt;

    fn output(session: everruns_contracts::typed_id::SessionId, text: &str) -> EventRequest {
        EventRequest::new(
            session,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant(text)),
        )
    }

    #[tokio::test]
    async fn startup_fallback_recovers_without_restarting_and_cycles_existing_subscriptions() {
        let db = Arc::new(StorageBackend::test_database());
        let session = db.create_test_session().await;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("nats://127.0.0.1:{port}");
        // Hold the port with no NATS handshake while startup fallback serves events.
        let inner = RecoveringEventDelivery::start_with_interval(
            db.clone(),
            url,
            Duration::from_millis(50),
        );
        let delivery = EventDelivery::Recovering(inner.clone());
        let service = Arc::new(EventService::new(db, delivery.clone()));
        assert_eq!(delivery.backend_name(), "polling");
        assert!(delivery.is_degraded());
        assert!(!delivery.supports_ephemeral_skip());
        let mut existing = delivery.subscribe(session.uuid()).await.unwrap();
        let before = service
            .emit(output(session, "before recovery"))
            .await
            .unwrap();
        assert_eq!(existing.recv().await.unwrap().id, before.id);
        let tracker = Arc::new(SseConnectionTracker::new(SseConnectionLimits {
            global_max: 10,
            per_org_max: 10,
            per_session_max: 10,
        }));
        let old_response = session_event_sse(SessionEventStream {
            event_service: service.clone(),
            session_id: session.uuid(),
            since_id: Some(before.id.uuid()),
            after_sequence: None,
            filter_types: Vec::new(),
            exclude_types: Vec::new(),
            guard: tracker.try_acquire(1, session.uuid()).unwrap(),
            render: Arc::new(|event, retry| {
                Some(sse_frame(
                    event,
                    serde_json::to_string(event).unwrap(),
                    retry,
                ))
            }),
        })
        .await;
        let mut old_body = old_response.into_response().into_body().into_data_stream();
        assert!(
            std::str::from_utf8(&old_body.next().await.unwrap().unwrap())
                .unwrap()
                .contains("event: connected")
        );
        drop(listener);
        let _broker = TestNatsServer::start_on(port).await;
        let mut recovered = inner.recovered.subscribe();
        tokio::time::timeout(Duration::from_secs(30), recovered.wait_for(|n| n.is_some()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivery.backend_name(), "nats");
        assert!(!delivery.is_degraded());
        assert!(delivery.supports_ephemeral_skip());
        assert!(
            tokio::time::timeout(Duration::from_secs(2), existing.recv())
                .await
                .unwrap()
                .is_none(),
            "old stream must reconnect, not remain on polling"
        );
        let closing = tokio::time::timeout(Duration::from_secs(2), old_body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            std::str::from_utf8(&closing)
                .unwrap()
                .contains("event: disconnecting")
        );
        assert!(old_body.next().await.is_none());
        drop(old_body);
        // Durable events emitted while the old connection is retired must be
        // replayed once, even if they are also buffered on the new NATS subscription.
        let gap = service
            .emit(output(session, "during reconnect"))
            .await
            .unwrap();
        let tracker = Arc::new(SseConnectionTracker::new(SseConnectionLimits {
            global_max: 10,
            per_org_max: 10,
            per_session_max: 10,
        }));
        let response = session_event_sse(SessionEventStream {
            event_service: service.clone(),
            session_id: session.uuid(),
            since_id: Some(before.id.uuid()),
            after_sequence: None,
            filter_types: Vec::new(),
            exclude_types: Vec::new(),
            guard: tracker.try_acquire(1, session.uuid()).unwrap(),
            render: Arc::new(|event, retry| {
                Some(sse_frame(
                    event,
                    serde_json::to_string(event).unwrap(),
                    retry,
                ))
            }),
        })
        .await;
        let mut body = response.into_response().into_body().into_data_stream();
        let connected = body.next().await.unwrap().unwrap();
        assert!(
            std::str::from_utf8(&connected)
                .unwrap()
                .contains("event: connected")
        );
        // This event is both in replay and the already-established live subscription.
        let overlap = service
            .emit(output(session, "replay/live overlap"))
            .await
            .unwrap();
        for event in [&gap, &overlap] {
            let frame = tokio::time::timeout(Duration::from_secs(5), body.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            let text = std::str::from_utf8(&frame).unwrap();
            assert!(text.contains(&event.id.to_string()), "{text}");
            assert!(!text.contains(&before.id.to_string()), "{text}");
        }
        let live = service
            .emit(output(session, "live after replay"))
            .await
            .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(5), body.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            std::str::from_utf8(&frame)
                .unwrap()
                .contains(&live.id.to_string()),
            "buffered overlap must not be repeated"
        );
        drop(body);
        let mut fresh = delivery.subscribe(session.uuid()).await.unwrap();
        let after = service
            .emit(output(session, "after recovery"))
            .await
            .unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), fresh.recv())
                .await
                .unwrap()
                .unwrap()
                .id,
            after.id
        );
        inner.nats().unwrap().shutdown().await;
    }

    #[tokio::test]
    async fn dropping_fallback_does_not_keep_the_backend_alive() {
        let inner = RecoveringEventDelivery::start_with_interval(
            Arc::new(StorageBackend::test_database()),
            "nats://127.0.0.1:1".into(),
            Duration::from_secs(60),
        );
        let weak = Arc::downgrade(&inner);
        drop(inner);
        assert!(weak.upgrade().is_none());
    }
}
