// PostgreSQL polling backend: live session events for servers that cannot
// share NATS.
//
// Why: in-memory delivery only reaches clients of the process that published
// the event. With more than one server and no NATS (NATS_URL unset on a
// multi-server install, or NATS unreachable when a server started), a chat
// client would miss every event written through another server, token deltas
// included, with no error anywhere.
//
// Decisions:
// - Polling persists every event, deltas included
//   (`EventDelivery::supports_ephemeral_skip` is false), so the log is the one
//   source every server can read. Publishing is then a no-op.
// - A subscription starts after the session's last stored event: callers
//   (session SSE) subscribe first and replay older events from the log
//   themselves, as they do with NATS. Per session, events commit in sequence
//   order (each append holds the session's `event_sequences` row lock until
//   commit), so a sequence cursor never skips one.
// - The poll backs off from `MIN_POLL` to `MAX_POLL` while a session is quiet
//   and returns to `MIN_POLL` on the next event. This is a degraded mode: it
//   costs a query per open stream per interval, and `/health` and the system
//   doctor report it.

use super::EventSubscription;
use crate::services::EventService;
use crate::storage::StorageBackend;
use everruns_contracts::typed_id::SessionId;
use everruns_core::Event;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;
use uuid::Uuid;

const MIN_POLL: Duration = Duration::from_millis(100);
const MAX_POLL: Duration = Duration::from_millis(1000);
/// Most events one poll reads; the rest come on the next poll.
const POLL_BATCH: usize = 500;

pub struct PollingEventDelivery {
    db: Arc<StorageBackend>,
}

impl PollingEventDelivery {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    pub async fn subscribe(&self, session_id: Uuid) -> anyhow::Result<EventSubscription> {
        let after = self
            .db
            .last_event_sequence(SessionId::from_uuid(session_id))
            .await?
            .unwrap_or(0);
        Ok(EventSubscription::Polling(Box::new(PollingSubscription {
            db: self.db.clone(),
            session_id,
            after,
            pending: VecDeque::new(),
            wait: MIN_POLL,
        })))
    }
}

pub struct PollingSubscription {
    db: Arc<StorageBackend>,
    session_id: Uuid,
    /// Last sequence handed out.
    after: i32,
    pending: VecDeque<Event>,
    wait: Duration,
}

impl PollingSubscription {
    pub(super) async fn recv(&mut self) -> Option<Event> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            match self
                .db
                .list_events(
                    SessionId::from_uuid(self.session_id),
                    Some(self.after),
                    None,
                    &[],
                    &[],
                    None,
                    None,
                )
                .await
            {
                Ok(rows) if !rows.is_empty() => {
                    for row in rows.into_iter().take(POLL_BATCH) {
                        self.after = row.sequence;
                        let event = EventService::row_to_event(row);
                        if !event.is_unsupported() {
                            self.pending.push_back(event);
                        }
                    }
                    self.wait = MIN_POLL;
                    continue;
                }
                Ok(_) => {}
                Err(error) => {
                    warn!(session_id = %self.session_id, %error, "event poll failed; retrying");
                }
            }
            tokio::time::sleep(self.wait).await;
            self.wait = (self.wait * 2).min(MAX_POLL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::CreateEventRow;
    use crate::storage::test_database::test_session_row;

    fn row(session_id: SessionId, event_type: &str) -> CreateEventRow {
        CreateEventRow {
            session_id,
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: serde_json::json!({}),
            data: serde_json::json!({
                "turn_id": everruns_contracts::typed_id::TurnId::new(),
                "input_message_id": everruns_contracts::typed_id::MessageId::new(),
            }),
            metadata: None,
            tags: None,
        }
    }

    #[tokio::test]
    async fn a_subscriber_gets_events_any_server_stores_after_it_subscribed() {
        let db = Arc::new(StorageBackend::test_database());
        let session = db
            .create_session(test_session_row(everruns_core::DEFAULT_ORG_ID))
            .await
            .expect("session")
            .id;
        db.create_event(row(session, "session.idled"))
            .await
            .expect("earlier event");

        let delivery =
            super::super::EventDelivery::Polling(Arc::new(PollingEventDelivery::new(db.clone())));
        let mut subscription = delivery.subscribe(session.uuid()).await.expect("subscribe");
        // Another server stores these: publishing here reaches no one.
        db.create_events(vec![
            row(session, "session.activated"),
            row(session, "session.idled"),
        ])
        .await
        .expect("later events");

        let mut seen = Vec::new();
        for _ in 0..2 {
            let event = tokio::time::timeout(Duration::from_secs(5), subscription.recv())
                .await
                .expect("an event within 5s")
                .expect("open subscription");
            seen.push(event.event_type);
        }
        assert_eq!(seen, ["session.activated", "session.idled"]);
        assert!(delivery.is_degraded());
        assert!(
            !delivery.supports_ephemeral_skip(),
            "polling needs every event stored"
        );
    }
}
