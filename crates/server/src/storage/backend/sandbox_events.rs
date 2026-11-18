//! Sandbox events: the cause behind each Sandbox lifecycle change, and each
//! decision not to change (`sandbox_events`).
//!
//! Decision: written by the actor that decided (idle timer, idle sweep,
//! retention sweep, a person through the API), not by a trigger, because only
//! the actor knows why. `sandbox_state_transitions` keeps recording the states
//! themselves; the two read together as one history.
//!
//! Decision: a `skipped` event is written only when it differs from the
//! Sandbox's latest event, so a sweep that looks at the same Sandbox every
//! minute for a day writes one row, not 1,440.
//!
//! Decision: free text (`message`, string values in `detail`) is scrubbed
//! (`everruns_contracts::secret_scrub`) before insert, so a provider error
//! that echoes a key never reaches the table.

use super::*;

/// Most events one read returns.
pub const MAX_SANDBOX_EVENTS: i64 = 500;

/// One cause to record.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSandboxEvent {
    /// `idle_timer`, `idle_sweep`, `retention_sweep`, `person` or `provider`.
    pub actor: &'static str,
    /// `pause`, `resume`, `delete` or `rebuild`.
    pub action: &'static str,
    /// `done`, `skipped` or `failed`.
    pub outcome: &'static str,
    /// Stable reason code.
    pub reason: &'static str,
    /// One plain sentence.
    pub message: String,
    pub detail: serde_json::Value,
}

impl NewSandboxEvent {
    pub fn new(
        actor: &'static str,
        action: &'static str,
        outcome: &'static str,
        reason: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            actor,
            action,
            outcome,
            reason,
            message: message.into(),
            detail: serde_json::json!({}),
        }
    }

    pub fn with_detail(mut self, detail: serde_json::Value) -> Self {
        self.detail = detail;
        self
    }
}

/// A recorded event.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SandboxEventRow {
    pub id: i64,
    pub sandbox_id: Uuid,
    pub generation: i64,
    pub at: DateTime<Utc>,
    pub actor: String,
    pub action: String,
    pub outcome: String,
    pub reason: String,
    pub message: String,
    pub detail: serde_json::Value,
}

impl StorageBackend {
    /// Record `event` on the Session's primary Sandbox (its newest, whatever
    /// its state). Returns whether a row was written: `false` when the Session
    /// has no Sandbox, or the event is a `skipped` repeat of the latest one.
    pub async fn record_sandbox_event_for_session(
        &self,
        session_id: Uuid,
        event: &NewSandboxEvent,
    ) -> Result<bool> {
        let mut message = event.message.clone();
        message = everruns_contracts::secret_scrub::scrub_secrets(&message);
        let mut detail = event.detail.clone();
        everruns_contracts::secret_scrub::scrub_secrets_in_value(&mut detail);
        let db = self.database();
        let written = sqlx::query(
            r#"
            WITH target AS (
                SELECT s.id, s.org_id, s.generation
                FROM sandboxes s
                WHERE s.session_id = $1 AND s.role = 'primary'
                ORDER BY s.created_at DESC
                LIMIT 1
            ),
            latest AS (
                SELECT e.actor, e.action, e.outcome, e.reason
                FROM sandbox_events e
                JOIN target t ON t.id = e.sandbox_id
                ORDER BY e.at DESC, e.id DESC
                LIMIT 1
            )
            INSERT INTO sandbox_events
                (sandbox_id, org_id, generation, actor, action, outcome, reason, message, detail)
            SELECT t.id, t.org_id, t.generation, $2, $3, $4, $5, $6, $7
            FROM target t
            WHERE $4 <> 'skipped'
               OR NOT EXISTS (
                    SELECT 1 FROM latest l
                    WHERE l.actor = $2 AND l.action = $3
                      AND l.outcome = $4 AND l.reason = $5)
            "#,
        )
        .bind(session_id)
        .bind(event.actor)
        .bind(event.action)
        .bind(event.outcome)
        .bind(event.reason)
        .bind(&message)
        .bind(&detail)
        .execute(db.pool())
        .await?
        .rows_affected();
        Ok(written > 0)
    }

    /// A Sandbox's events in `[from, to)`, oldest first, at most `limit`
    /// (the newest ones when there are more).
    pub async fn list_sandbox_events(
        &self,
        org_id: i64,
        sandbox_id: Uuid,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<SandboxEventRow>> {
        let db = self.database();
        let mut rows: Vec<SandboxEventRow> = sqlx::query_as(
            r#"
            SELECT id, sandbox_id, generation, at, actor, action, outcome, reason, message, detail
            FROM sandbox_events
            WHERE org_id = $1 AND sandbox_id = $2 AND at >= $3 AND at < $4
            ORDER BY at DESC, id DESC
            LIMIT $5
            "#,
        )
        .bind(org_id)
        .bind(sandbox_id)
        .bind(from)
        .bind(to)
        .bind(limit.clamp(1, MAX_SANDBOX_EVENTS))
        .fetch_all(db.pool())
        .await?;
        rows.reverse();
        Ok(rows)
    }

    /// The lifecycle policy a Sandbox was pinned with (`profile_snapshot`'s
    /// `lifecycle`), and its session's status, for diagnosis.
    pub async fn get_sandbox_lifecycle_inputs(
        &self,
        org_id: i64,
        sandbox_id: Uuid,
    ) -> Result<Option<SandboxLifecycleInputs>> {
        let db = self.database();
        Ok(sqlx::query_as(
            r#"
            SELECT s.observed_state, s.desired_state,
                   s.profile_snapshot->'lifecycle' AS lifecycle,
                   s.profile_snapshot AS profile,
                   sess.status AS session_status,
                   sess.updated_at AS session_updated_at,
                   s.last_activity_at, s.updated_at
            FROM sandboxes s
            LEFT JOIN sessions sess ON sess.id = s.session_id
            WHERE s.org_id = $1 AND s.id = $2
            "#,
        )
        .bind(org_id)
        .bind(sandbox_id)
        .fetch_optional(db.pool())
        .await?)
    }

    /// Orphan-scan sightings of one provider resource.
    pub async fn get_orphan_sighting(
        &self,
        provider: &str,
        external_id: &str,
    ) -> Result<Option<OrphanSightingRow>> {
        let db = self.database();
        Ok(sqlx::query_as(
            "SELECT first_seen_at, last_seen_at, stopped_at, deleted_at \
             FROM sandbox_orphan_sightings WHERE provider = $1 AND external_id = $2",
        )
        .bind(provider)
        .bind(external_id)
        .fetch_optional(db.pool())
        .await?)
    }
}

/// What diagnosis needs from a Sandbox row beyond the fleet projection.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SandboxLifecycleInputs {
    pub observed_state: String,
    pub desired_state: String,
    pub lifecycle: Option<serde_json::Value>,
    pub profile: Option<serde_json::Value>,
    pub session_status: Option<String>,
    pub session_updated_at: Option<DateTime<Utc>>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

/// An orphan-scan sighting.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OrphanSightingRow {
    pub first_seen_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub stopped_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}
