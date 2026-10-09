//! Token deltas for the dispatcher where PostgreSQL never sees them (EVE-1211).
//!
//! Under NATS event delivery `output.message.delta` is ephemeral
//! (`EventDelivery::supports_ephemeral_skip`): it skips PostgreSQL and exists
//! only on the delivery bus. The polling dispatcher (`wake.rs`) reads
//! PostgreSQL, so it posts completed messages but would never stream. Each
//! session with a registered delivery therefore gets one `EventDelivery`
//! subscription that feeds its deltas into the same turn delivery the
//! PostgreSQL path feeds. Everything else — completion, terminal states,
//! approvals — still comes from the poll, which stays authoritative.
//!
//! Design decisions:
//! - One subscription per session, created at registration and aborted when
//!   the session's last delivery unregisters or the dispatcher shuts down.
//!   Created and dropped under the `deliveries` write lock, so a registration
//!   racing an unregistration cannot leak a subscription.
//! - A failed or ended subscription only logs: the poll still posts the
//!   completed message, which is exactly the pre-EVE-1211 behaviour. The next
//!   registration on that session retries.
//! - Feed and poll for one turn are serialized by the turn's own lock. Without
//!   it, a delta opening a stream while the poll handles `completed` could
//!   leave both a discrete reply and a stream.
//! - Deltas reach the feed out of band, so one can arrive after the poll already
//!   closed (or a guardrail replaced) its message. The turn delivery remembers
//!   finished messages and drops their late deltas: reopening would duplicate
//!   the reply, or resurface text a guardrail retracted.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, Weak};

use tokio::task::JoinHandle;

use super::*;
use crate::live_updates::event_delivery::EventDelivery;
use everruns_core::events;

/// Per-session delta subscriptions for one dispatcher.
pub(super) struct LiveDeltas {
    dispatcher: Weak<SlackDeliveryDispatcher>,
    /// Set once at startup, only on backends whose deltas skip PostgreSQL.
    source: OnceLock<EventDelivery>,
    feeds: Mutex<HashMap<Uuid, LiveFeed>>,
}

struct LiveFeed {
    task: JoinHandle<()>,
}

impl LiveDeltas {
    pub(super) fn new(dispatcher: Weak<SlackDeliveryDispatcher>) -> Self {
        Self {
            dispatcher,
            source: OnceLock::new(),
            feeds: Mutex::new(HashMap::new()),
        }
    }

    /// Unconditional; `feed_live_deltas` decides whether a backend needs it.
    pub(super) fn set_source(&self, source: EventDelivery) {
        if self.source.set(source).is_err() {
            warn!("Slack live delta source already set");
        }
    }

    fn feeds(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, LiveFeed>> {
        // Nothing under this lock can panic halfway through an update.
        self.feeds.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Start a session's feed unless one is already running. Call with the
    /// `deliveries` write lock held.
    pub(super) fn ensure(&self, session_id: Uuid) {
        let Some(source) = self.source.get() else {
            return;
        };
        let mut feeds = self.feeds();
        if feeds
            .get(&session_id)
            .is_some_and(|f| !f.task.is_finished())
        {
            return;
        }
        // A finished predecessor (failed or ended subscription) is replaced.
        let task = tokio::spawn(feed(source.clone(), self.dispatcher.clone(), session_id));
        feeds.insert(session_id, LiveFeed { task });
    }

    /// Stop a session's feed. Call with the `deliveries` write lock held.
    pub(super) fn drop_session(&self, session_id: Uuid) {
        if let Some(feed) = self.feeds().remove(&session_id) {
            feed.task.abort();
        }
    }

    pub(super) fn drop_all(&self) {
        for (_, feed) in self.feeds().drain() {
            feed.task.abort();
        }
    }

    #[cfg(test)]
    pub(super) fn running(&self) -> usize {
        self.feeds()
            .values()
            .filter(|f| !f.task.is_finished())
            .count()
    }
}

/// One session's subscription loop.
///
/// Holds the dispatcher weakly so a forgotten feed never keeps it alive.
async fn feed(source: EventDelivery, dispatcher: Weak<SlackDeliveryDispatcher>, session_id: Uuid) {
    let mut subscription = match source.subscribe(session_id).await {
        Ok(subscription) => subscription,
        Err(error) => {
            warn!(
                %session_id,
                error = %crate::live_updates::nats::error_chain(&error),
                "Slack live delta subscription failed; completed messages still post by polling"
            );
            return;
        }
    };
    while let Some(event) = subscription.recv().await {
        // In-memory delivery shares partitions between sessions.
        if event.session_id.uuid() != session_id || event.event_type != events::OUTPUT_MESSAGE_DELTA
        {
            continue;
        }
        let Some(dispatcher) = dispatcher.upgrade() else {
            return;
        };
        dispatcher.apply_live_delta(session_id, &event).await;
    }
    debug!(%session_id, "Slack live delta subscription ended; polling continues");
}

impl SlackDeliveryDispatcher {
    /// Feed `output.message.delta` from `source` into pane streams, if its
    /// deltas skip PostgreSQL. Elsewhere the poll or listener already reads
    /// them, and a second copy would only add contention. Applies to sessions
    /// registered after the call.
    pub fn feed_live_deltas(&self, source: &EventDelivery) {
        if source.supports_ephemeral_skip() {
            self.live.set_source(source.clone());
        }
    }

    async fn apply_live_delta(&self, session_id: Uuid, event: &everruns_core::Event) {
        // Same JSON shape the poll reads from PostgreSQL.
        let (Ok(context), Ok(data)) = (
            serde_json::to_value(&event.context),
            serde_json::to_value(&event.data),
        ) else {
            return;
        };
        let Some(input_message_id) = context.get("input_message_id").and_then(|v| v.as_str())
        else {
            return;
        };
        let key = DeliveryKey {
            session_id,
            input_message_id: input_message_id.to_string(),
        };
        let Some(slot) = self.deliveries.read().await.get(&key).cloned() else {
            return;
        };
        slot.lock()
            .await
            .delivery
            .observe(&DeliveryEvent {
                sequence: None,
                event_type: events::OUTPUT_MESSAGE_DELTA.to_string(),
                data,
                input_message_id: Some(input_message_id.to_string()),
            })
            .await;
    }
}
