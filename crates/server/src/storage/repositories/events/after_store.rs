// After an event write: session trace.
//
// Reporting no longer enqueues an outbox row per event: its projector is a
// later-tier consumer of the listener feed (`ReportingProjectionListener`),
// which delivers every committed event without a write on the event path.

use super::super::Database;
use super::rows::EventRow;
use uuid::Uuid;

impl Database {
    /// The session trace work a stored batch owes.
    pub(super) async fn after_events_stored(&self, ordered: &[EventRow]) {
        let mut traced: Vec<Uuid> = Vec::new();
        for row in ordered {
            if Self::ends_trace_turn(&row.event_type) && !traced.contains(&row.session_id.uuid()) {
                traced.push(row.session_id.uuid());
            }
        }
        for session_id in traced {
            self.schedule_session_trace_projection(session_id);
        }
    }
}
