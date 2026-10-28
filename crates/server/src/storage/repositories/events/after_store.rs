// After an event write: listener wake-up, reporting outbox and session trace.

use super::super::Database;
use super::rows::EventRow;
use tracing::warn;
use uuid::Uuid;

impl Database {
    /// The listener wake-up, reporting projection and session trace work a
    /// stored batch owes.
    pub(super) async fn after_events_stored(&self, ordered: &[EventRow]) {
        let mut sessions: Vec<Uuid> = ordered.iter().map(|row| row.session_id.uuid()).collect();
        sessions.sort();
        sessions.dedup();
        self.notify_event_available(&sessions).await;
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

    /// Wake PostgreSQL LISTEN subscribers (`event_available`, payload: the
    /// session id) for sessions that just got events.
    ///
    /// Decision: this used to be an `AFTER INSERT` trigger on `events`. Every
    /// transaction that queues a NOTIFY takes one database-wide lock at commit
    /// to keep notification order, whether or not anyone listens, so every
    /// event write in the cluster committed one at a time. Production at 500
    /// concurrent agents spent more than half its database load in COMMIT
    /// waiting for that lock (Performance Insights `Lock:object`). Under NATS
    /// event delivery nothing listens on `event_available`, and the server
    /// turns this off (`set_pg_event_notify(false)`); without NATS the SSE
    /// broadcaster still needs it.
    ///
    /// Decision: sent after the insert, on the same pool, as a best-effort
    /// hint: subscribers re-read the log, so a lost wake-up only delays SSE
    /// until the next one. Inside a command transaction the NOTIFY joins it
    /// and is delivered at its commit, as the trigger's was.
    pub(super) async fn notify_event_available(&self, sessions: &[Uuid]) {
        if sessions.is_empty() || !self.pg_event_notify() {
            return;
        }
        if let Err(error) = sqlx::query(
            "SELECT pg_notify('event_available', id::text) FROM UNNEST($1::uuid[]) AS id",
        )
        .bind(sessions)
        .execute(&self.pool)
        .await
        {
            warn!(%error, "event_available notify failed; SSE listeners wake on the next event");
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
