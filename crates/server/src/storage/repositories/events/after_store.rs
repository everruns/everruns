// After an event write: reporting outbox and session trace.

use super::super::Database;
use super::rows::EventRow;
use tracing::warn;
use uuid::Uuid;

impl Database {
    /// The reporting projection and session trace work a stored batch owes.
    pub(super) async fn after_events_stored(&self, ordered: &[EventRow]) {
        self.enqueue_event_projections(ordered).await;
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

    /// One outbox insert for every row of a stored batch that feeds reporting.
    pub(super) async fn enqueue_event_projections(&self, rows: &[EventRow]) {
        // Reporting outbox enqueue is best-effort per knowledge/evaluation/reporting.md:
        // "If an outbox write fails after canonical state commits, a periodic
        // reconciler must be able to discover and enqueue missing projection
        // work from canonical tables." Never fail the canonical event write
        // because the reporting projector queue is unavailable. No reconciler
        // job exists yet (tracked separately); until it lands, a failed
        // enqueue means the corresponding fact will remain stale until the
        // canonical row is touched again and the next enqueue succeeds.
        let rows: Vec<&EventRow> = rows
            .iter()
            .filter(|row| {
                crate::storage::reporting::outbox::event_needs_projection(&row.event_type)
            })
            .collect();
        if rows.is_empty() {
            return;
        }
        let event_ids: Vec<String> = rows.iter().map(|row| row.id.uuid().to_string()).collect();
        let session_ids: Vec<Uuid> = rows.iter().map(|row| row.session_id.uuid()).collect();
        if let Err(e) = sqlx::query(
            r#"
            INSERT INTO reporting_outbox (
                org_id, source_type, source_id, source_version, reason, status, next_attempt_at
            )
            SELECT s.org_id, 'event', e.event_id, e.event_id, 'event_projection', 'pending', NOW()
            FROM UNNEST($1::text[], $2::uuid[]) AS e(event_id, session_id)
            JOIN sessions s ON s.id = e.session_id
            ON CONFLICT (org_id, source_type, source_id, source_version, reason)
            DO NOTHING
            "#,
        )
        .bind(&event_ids)
        .bind(&session_ids)
        .execute(&self.pool)
        .await
        {
            for row in rows {
                warn!(
                    event_id = %row.id.uuid(),
                    session_id = %row.session_id.uuid(),
                    event_type = %row.event_type,
                    error = %e,
                    "reporting outbox enqueue failed; event persisted, projection may remain stale until reconciliation lands or the source row is updated again"
                );
            }
        }
    }
}
