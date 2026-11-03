//! Reporting projection as a consumer of the listener feed.
//!
//! Decisions (`knowledge/evaluation/reporting.md`, "Event projection"):
//! - The projector reads events from the listener feed's later tier
//!   (`services::event::feed`), which delivers every committed event at least
//!   once and in session order. The event write no longer inserts a
//!   `reporting_outbox` row beside each projected event.
//! - Projections are keyed on the event (capability snapshots on the turn),
//!   so a second delivery after a crash writes nothing new.
//! - A projection that fails here is handed to the outbox, which retries it
//!   with backoff like any other source. The bounded repair job still finds
//!   events whose facts are missing, which covers events stored while
//!   reporting was off.
//! - Registered only when reporting is enabled (`REPORTING_BACKGROUND_ENABLED`).

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::{Event, EventListener};

use crate::storage::StorageBackend;
use crate::storage::reporting::outbox::{PROJECTED_EVENT_TYPES, PostgresReportingProjector};

/// Projects stored events into the reporting fact tables.
pub struct ReportingProjectionListener {
    db: Arc<StorageBackend>,
    projector: PostgresReportingProjector,
}

impl ReportingProjectionListener {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        let projector = PostgresReportingProjector::new(db.database().relaxed_pool().clone());
        Self { db, projector }
    }
}

#[async_trait]
impl EventListener for ReportingProjectionListener {
    fn name(&self) -> &'static str {
        "ReportingProjection"
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(PROJECTED_EVENT_TYPES.to_vec())
    }

    async fn on_event(&self, event: &Event) {
        let id = event.id.uuid();
        let org_id = match self.db.get_session_organization_id(event.session_id).await {
            Ok(Some(org_id)) => org_id,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    event_id = %id,
                    error = %error,
                    "reporting projection: session lookup failed; the repair job will find the event"
                );
                return;
            }
        };
        let Err(error) = self
            .projector
            .project_stored_event(org_id, id, &event.event_type)
            .await
        else {
            return;
        };
        tracing::warn!(
            event_id = %id,
            event_type = %event.event_type,
            error = %error,
            "reporting projection failed; retrying through the outbox"
        );
        let source = id.to_string();
        if let Err(error) = self
            .db
            .enqueue_reporting_outbox(org_id, "event", &source, Some(&source), "event_projection")
            .await
        {
            tracing::warn!(
                event_id = %id,
                error = %error,
                "reporting outbox enqueue failed; the repair job will find the event"
            );
        }
    }
}

#[cfg(test)]
#[path = "listener_tests.rs"]
mod tests;
