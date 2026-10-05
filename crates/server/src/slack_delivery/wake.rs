//! What tells the Slack delivery dispatcher that a session has new events.
//!
//! The PostgreSQL `LISTEN/NOTIFY` broadcaster is the push source, but it is
//! deliberately not started when NATS carries event delivery (see
//! `knowledge/operations/production-deployment.md`). Before EVERRUNS-2B that
//! left production without a dispatcher at all, and every Slack reply fell back
//! to a 120-second polling loop that gave up on longer turns and stayed silent
//! on failed ones. Polling active sessions from the dispatcher keeps the same
//! delivery semantics (no deadline, terminal notices, streaming, approvals) on
//! every backend, and reads PostgreSQL, which every instance shares. The one
//! thing PostgreSQL lacks there is token deltas; `live_deltas.rs` feeds those
//! from the delivery bus (EVE-1211).

use std::sync::Arc;

use tokio::sync::broadcast;
use uuid::Uuid;

use super::SlackDeliveryDispatcher;
use super::session_scheduler::SessionDeliveryScheduler;
use crate::event_notifications::EventNotificationPayload;

/// How the dispatcher learns which sessions to re-read.
pub enum DeliveryWake {
    /// Per-session wakeups from `EventNotificationBroadcaster`.
    Notifications(broadcast::Receiver<EventNotificationPayload>),
    /// No notification source: re-read every session with an active delivery
    /// on each dispatcher tick.
    Poll,
}

impl From<broadcast::Receiver<EventNotificationPayload>> for DeliveryWake {
    fn from(rx: broadcast::Receiver<EventNotificationPayload>) -> Self {
        Self::Notifications(rx)
    }
}

impl DeliveryWake {
    /// Whether active sessions are re-read on every tick.
    pub fn polls(&self) -> bool {
        matches!(self, Self::Poll)
    }

    /// Next notification. Never resolves when polling, so the dispatcher's
    /// `select!` is driven by its tick alone.
    pub(super) async fn recv(&mut self) -> Result<Uuid, broadcast::error::RecvError> {
        match self {
            Self::Notifications(rx) => rx.recv().await.map(|payload| payload.session_id),
            Self::Poll => std::future::pending().await,
        }
    }
}

impl SlackDeliveryDispatcher {
    /// Schedule every session with a registered delivery. Snapshots the set
    /// first so its lock is not held while the work is scheduled.
    pub(super) async fn schedule_all_active(
        self: &Arc<Self>,
        scheduler: &mut SessionDeliveryScheduler,
    ) {
        let sessions: Vec<Uuid> = self.active_sessions.read().await.iter().copied().collect();
        for session_id in sessions {
            scheduler.schedule(self.clone(), session_id);
        }
    }
}
