// PostgreSQL repository: Session Schedules, Leased Resources

pub(super) mod rows;
use rows::*;
#[cfg(test)]
mod tests;

use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{LeasedResourceId, ScheduleId, SessionId};

// Decision (EVERRUNS-4C): name the columns, never `*`. sqlx caches each
// prepared statement per connection, and PostgreSQL refuses to re-plan a
// cached statement whose result columns changed ("cached plan must not change
// result type"). With `*`, any added column changes them, so an instance still
// running during a rolling deploy failed every poll after the new release
// migrated the table. A named list keeps the result type fixed across additive
// migrations. Keep each list equal to its row struct's fields.
const SESSION_SCHEDULE_COLUMNS: &str = "id, public_id, org_id, session_id, owner_principal_id, \
     resolved_owner_user_id, description, cron_expression, scheduled_at, timezone, enabled, \
     next_trigger_at, last_triggered_at, trigger_count, claimed_by, claimed_at, created_at, \
     updated_at, deleted_at, ends_at, max_runs, stop_when, stop_reason";

const LEASED_RESOURCE_COLUMNS: &str = "id, public_id, org_id, session_id, provider, \
     resource_type, external_id, display_name, status, owner_user_id, connection_id, \
     lease_duration_seconds, last_touched_at, lease_expires_at, cleanup_started_at, \
     cleanup_completed_at, cleanup_attempts, last_cleanup_error, metadata, created_at, updated_at";

impl Database {
    // ============================================
    // Session Schedules
    // ============================================

    pub async fn create_session_schedule(
        &self,
        input: CreateSessionScheduleRow,
    ) -> Result<SessionScheduleRow> {
        let id = ScheduleId::new();
        let public_id = id.to_string();

        let row = sqlx::query_as::<_, SessionScheduleRow>(sqlx::AssertSqlSafe(format!(
            r#"
            INSERT INTO session_schedules (id, public_id, org_id, session_id, owner_principal_id, resolved_owner_user_id, description, cron_expression, scheduled_at, timezone, next_trigger_at, ends_at, max_runs, stop_when)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            RETURNING {SESSION_SCHEDULE_COLUMNS}
            "#
        )))
        .bind(id)
        .bind(&public_id)
        .bind(input.org_id)
        .bind(input.session_id)
        .bind(input.owner_principal_id)
        .bind(input.resolved_owner_user_id)
        .bind(&input.description)
        .bind(&input.cron_expression)
        .bind(input.scheduled_at)
        .bind(&input.timezone)
        .bind(input.next_trigger_at)
        .bind(input.ends_at)
        .bind(input.max_runs)
        .bind(&input.stop_when)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn create_session_schedule_with_limits(
        &self,
        input: CreateSessionScheduleRow,
        max_per_session: u32,
        max_per_org: i64,
    ) -> Result<Option<SessionScheduleRow>> {
        let mut tx = self.pool.begin().await?;
        // Serialize quota checks and insertion per org so concurrent creators
        // cannot all observe the same below-cap count before inserting.
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(input.org_id)
            .execute(&mut *tx)
            .await?;

        let active_session_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM session_schedules WHERE session_id = $1 AND enabled = true AND deleted_at IS NULL",
        )
        .bind(input.session_id)
        .fetch_one(&mut *tx)
        .await?;
        let active_org_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM session_schedules WHERE org_id = $1 AND enabled = true AND deleted_at IS NULL",
        )
        .bind(input.org_id)
        .fetch_one(&mut *tx)
        .await?;

        if active_session_count.0 >= i64::from(max_per_session) || active_org_count.0 >= max_per_org
        {
            tx.rollback().await?;
            return Ok(None);
        }

        let id = ScheduleId::new();
        let public_id = id.to_string();
        let row = sqlx::query_as::<_, SessionScheduleRow>(sqlx::AssertSqlSafe(format!(
            r#"
            INSERT INTO session_schedules (id, public_id, org_id, session_id, owner_principal_id, resolved_owner_user_id, description, cron_expression, scheduled_at, timezone, next_trigger_at, ends_at, max_runs, stop_when)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            RETURNING {SESSION_SCHEDULE_COLUMNS}
            "#
        )))
        .bind(id)
        .bind(&public_id)
        .bind(input.org_id)
        .bind(input.session_id)
        .bind(input.owner_principal_id)
        .bind(input.resolved_owner_user_id)
        .bind(&input.description)
        .bind(&input.cron_expression)
        .bind(input.scheduled_at)
        .bind(&input.timezone)
        .bind(input.next_trigger_at)
        .bind(input.ends_at)
        .bind(input.max_runs)
        .bind(&input.stop_when)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(row))
    }
    pub async fn get_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<Option<SessionScheduleRow>> {
        let row = sqlx::query_as::<_, SessionScheduleRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SESSION_SCHEDULE_COLUMNS} FROM session_schedules WHERE id = $1 AND org_id = $2"
        )))
        .bind(schedule_id)
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Schedules the agent can still act on. Deleted rows stay in the table
    /// for the session recording; this list omits them.
    pub async fn list_session_schedules(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionScheduleRow>> {
        let rows = sqlx::query_as::<_, SessionScheduleRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SESSION_SCHEDULE_COLUMNS} FROM session_schedules WHERE org_id = $1 AND session_id = $2 AND deleted_at IS NULL ORDER BY created_at DESC"
        )))
        .bind(org_id)
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Every schedule this session registered, including ones later deleted.
    /// The session view uses this so a removed schedule is still visible.
    pub async fn list_session_schedule_history(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<Vec<SessionScheduleRow>> {
        let rows = sqlx::query_as::<_, SessionScheduleRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {SESSION_SCHEDULE_COLUMNS} FROM session_schedules WHERE org_id = $1 AND session_id = $2 ORDER BY (deleted_at IS NULL) DESC, created_at DESC"
        )))
        .bind(org_id)
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
        input: UpdateSessionScheduleRow,
    ) -> Result<Option<SessionScheduleRow>> {
        let row = sqlx::query_as::<_, SessionScheduleRow>(sqlx::AssertSqlSafe(format!(
            r#"
            UPDATE session_schedules SET
                enabled = COALESCE($3, enabled),
                next_trigger_at = CASE WHEN $4 THEN $5 ELSE next_trigger_at END,
                last_triggered_at = COALESCE($6, last_triggered_at),
                trigger_count = trigger_count + CASE WHEN $7 THEN 1 ELSE 0 END,
                ends_at = CASE WHEN $8 THEN $9 ELSE ends_at END,
                max_runs = CASE WHEN $10 THEN $11 ELSE max_runs END,
                stop_reason = CASE WHEN $12 THEN $13 ELSE stop_reason END,
                updated_at = NOW()
            WHERE id = $1 AND org_id = $2 AND deleted_at IS NULL
            RETURNING {SESSION_SCHEDULE_COLUMNS}
            "#
        )))
        .bind(schedule_id)
        .bind(org_id)
        .bind(input.enabled)
        .bind(input.next_trigger_at.is_changed()) // flag: should update next_trigger_at
        .bind(input.next_trigger_at.into_value()) // value (may be NULL for one-shot)
        .bind(input.last_triggered_at)
        .bind(input.trigger_count_increment)
        .bind(input.ends_at.is_changed())
        .bind(input.ends_at.into_value())
        .bind(input.max_runs.is_changed())
        .bind(input.max_runs.into_value())
        .bind(input.stop_reason.is_changed())
        .bind(input.stop_reason.into_value())
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Keep the row and mark it deleted. A second delete matches nothing, so
    /// the caller still reports the schedule as already gone.
    pub async fn delete_session_schedule(
        &self,
        org_id: i64,
        schedule_id: ScheduleId,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE session_schedules
            SET deleted_at = NOW(),
                enabled = false,
                next_trigger_at = NULL,
                claimed_by = NULL,
                claimed_at = NULL,
                updated_at = NOW()
            WHERE id = $1 AND org_id = $2 AND deleted_at IS NULL
            "#,
        )
        .bind(schedule_id)
        .bind(org_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn count_active_session_schedules(&self, session_id: SessionId) -> Result<u32> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM session_schedules WHERE session_id = $1 AND enabled = true AND deleted_at IS NULL",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0 as u32)
    }

    pub async fn count_active_org_session_schedules(&self, org_id: i64) -> Result<u32> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM session_schedules WHERE org_id = $1 AND enabled = true AND deleted_at IS NULL",
        )
        .bind(org_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0 as u32)
    }

    /// Claim due session schedules for processing.
    /// Uses SELECT FOR UPDATE SKIP LOCKED for multi-instance safety.
    /// Claim due session schedules for this instance.
    ///
    /// Selecting and stamping in ONE statement is the whole point. The previous
    /// implementation ran a bare `SELECT ... FOR UPDATE SKIP LOCKED` straight on
    /// the pool: its implicit transaction committed as soon as the query
    /// returned, releasing the row locks long before the caller advanced
    /// `next_trigger_at`, and nothing recorded that a row had been picked up. A
    /// second server instance polling in that window saw the same schedules as
    /// still due and fired them again -- duplicate agent turns and duplicate
    /// model spend. Mirrors the durable scheduler's claim.
    ///
    /// The claim is a lease, not a permanent mark: an instance that dies between
    /// claiming and firing releases its schedules once the lease ages out.
    pub async fn claim_due_session_schedules(
        &self,
        scheduler_id: &str,
        limit: i32,
    ) -> Result<Vec<SessionScheduleRow>> {
        let rows = sqlx::query_as::<_, SessionScheduleRow>(sqlx::AssertSqlSafe(format!(
            r#"
            WITH due AS (
                SELECT id AS due_id FROM session_schedules
                WHERE enabled = true
                  AND deleted_at IS NULL
                  AND next_trigger_at IS NOT NULL
                  AND next_trigger_at <= NOW()
                  AND (
                      claimed_by IS NULL
                      OR claimed_at IS NULL
                      OR claimed_at < NOW() - ($3 * INTERVAL '1 second')
                  )
                ORDER BY next_trigger_at ASC
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            )
            UPDATE session_schedules s
            SET claimed_by = $1, claimed_at = NOW()
            FROM due d
            WHERE s.id = d.due_id
            RETURNING {SESSION_SCHEDULE_COLUMNS}
            "#
        )))
        .bind(scheduler_id)
        .bind(limit)
        .bind(SESSION_SCHEDULE_CLAIM_LEASE_SECONDS)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    // ============================================
    // Leased Resources
    // ============================================

    /// Resolve a session's organization ID without requiring the caller to know it.
    pub async fn get_session_organization_id(&self, session_id: SessionId) -> Result<Option<i64>> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT org_id FROM sessions WHERE id = $1 LIMIT 1")
                .bind(session_id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(|(org_id,)| org_id))
    }

    /// Insert or refresh a leased resource keyed by provider/type/external_id.
    pub async fn upsert_leased_resource(
        &self,
        input: UpsertLeasedResourceRow,
    ) -> Result<LeasedResourceRow> {
        let id = LeasedResourceId::new();
        let public_id = id.to_string();
        let now = Utc::now();

        let row = sqlx::query_as::<_, LeasedResourceRow>(sqlx::AssertSqlSafe(format!(
            r#"
            INSERT INTO leased_resources (
                id,
                public_id,
                org_id,
                session_id,
                provider,
                resource_type,
                external_id,
                display_name,
                status,
                owner_user_id,
                connection_id,
                lease_duration_seconds,
                last_touched_at,
                lease_expires_at,
                metadata
            )
            VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, 'active', $9, $10, $11, $12, $13, $14
            )
            ON CONFLICT (org_id, provider, resource_type, external_id)
            DO UPDATE SET
                session_id = EXCLUDED.session_id,
                display_name = COALESCE(EXCLUDED.display_name, leased_resources.display_name),
                status = 'active',
                owner_user_id = COALESCE(EXCLUDED.owner_user_id, leased_resources.owner_user_id),
                connection_id = COALESCE(EXCLUDED.connection_id, leased_resources.connection_id),
                lease_duration_seconds = EXCLUDED.lease_duration_seconds,
                last_touched_at = EXCLUDED.last_touched_at,
                lease_expires_at = EXCLUDED.lease_expires_at,
                cleanup_started_at = NULL,
                cleanup_completed_at = NULL,
                last_cleanup_error = NULL,
                metadata = EXCLUDED.metadata
            RETURNING {LEASED_RESOURCE_COLUMNS}
            "#
        )))
        .bind(id)
        .bind(&public_id)
        .bind(input.org_id)
        .bind(input.session_id)
        .bind(&input.provider)
        .bind(&input.resource_type)
        .bind(&input.external_id)
        .bind(&input.display_name)
        .bind(input.owner_user_id)
        .bind(input.connection_id)
        .bind(input.lease_duration_seconds)
        .bind(now)
        .bind(input.lease_expires_at)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Mark a resource as explicitly released after the provider cleanup succeeded.
    pub async fn release_leased_resource(
        &self,
        input: ReleaseLeasedResourceRow,
    ) -> Result<Option<LeasedResourceRow>> {
        let row = sqlx::query_as::<_, LeasedResourceRow>(sqlx::AssertSqlSafe(format!(
            r#"
            UPDATE leased_resources
            SET
                status = 'released',
                cleanup_started_at = NULL,
                cleanup_completed_at = NOW(),
                lease_expires_at = NOW(),
                last_cleanup_error = NULL,
                updated_at = NOW()
            WHERE org_id = $1
              AND session_id = $2
              AND provider = $3
              AND resource_type = $4
              AND external_id = $5
            RETURNING {LEASED_RESOURCE_COLUMNS}
            "#
        )))
        .bind(input.org_id)
        .bind(input.session_id)
        .bind(&input.provider)
        .bind(&input.resource_type)
        .bind(&input.external_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all leased resources attached to a session.
    pub async fn list_session_leased_resources(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<LeasedResourceRow>> {
        let rows = sqlx::query_as::<_, LeasedResourceRow>(sqlx::AssertSqlSafe(format!(
            r#"
            SELECT {LEASED_RESOURCE_COLUMNS}
            FROM leased_resources
            WHERE session_id = $1
            ORDER BY created_at DESC
            "#
        )))
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Claim due resources for cleanup, including stale in-progress claims.
    pub async fn claim_due_leased_resources(
        &self,
        limit: i32,
        stale_after_seconds: i32,
    ) -> Result<Vec<LeasedResourceRow>> {
        let rows = sqlx::query_as::<_, LeasedResourceRow>(sqlx::AssertSqlSafe(format!(
            r#"
            WITH due AS (
                SELECT id AS due_id
                FROM leased_resources
                WHERE (
                    status IN ('active', 'cleanup_failed')
                    AND lease_expires_at <= NOW()
                ) OR (
                    status = 'cleaning'
                    AND cleanup_started_at IS NOT NULL
                    AND cleanup_started_at <= NOW() - make_interval(secs => $2)
                )
                ORDER BY
                    lease_expires_at ASC NULLS FIRST,
                    cleanup_started_at ASC NULLS FIRST,
                    created_at ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            )
            UPDATE leased_resources lr
            SET
                status = 'cleaning',
                cleanup_started_at = NOW(),
                cleanup_attempts = lr.cleanup_attempts + 1,
                last_cleanup_error = NULL,
                updated_at = NOW()
            FROM due
            WHERE lr.id = due.due_id
            RETURNING {LEASED_RESOURCE_COLUMNS}
            "#
        )))
        .bind(limit)
        .bind(stale_after_seconds)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Mark a claimed resource as released after durable cleanup succeeds.
    pub async fn mark_leased_resource_released(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
    ) -> Result<Option<LeasedResourceRow>> {
        let row = sqlx::query_as::<_, LeasedResourceRow>(sqlx::AssertSqlSafe(format!(
            r#"
            UPDATE leased_resources
            SET
                status = 'released',
                cleanup_started_at = NULL,
                cleanup_completed_at = NOW(),
                lease_expires_at = NOW(),
                last_cleanup_error = NULL,
                updated_at = NOW()
            WHERE id = $1
              AND status = 'cleaning'
              AND cleanup_started_at = $2
            RETURNING {LEASED_RESOURCE_COLUMNS}
            "#
        )))
        .bind(resource_id)
        .bind(expected_cleanup_started_at)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Record a cleanup failure and move the next retry deadline forward.
    pub async fn mark_leased_resource_cleanup_failed(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: DateTime<Utc>,
        retry_after_seconds: i32,
        error: &str,
    ) -> Result<Option<LeasedResourceRow>> {
        let row = sqlx::query_as::<_, LeasedResourceRow>(sqlx::AssertSqlSafe(format!(
            r#"
            UPDATE leased_resources
            SET
                status = 'cleanup_failed',
                cleanup_started_at = NULL,
                lease_expires_at = NOW() + make_interval(secs => $2),
                last_cleanup_error = $3,
                updated_at = NOW()
            WHERE id = $1
              AND status = 'cleaning'
              AND cleanup_started_at = $4
            RETURNING {LEASED_RESOURCE_COLUMNS}
            "#
        )))
        .bind(resource_id)
        .bind(retry_after_seconds)
        .bind(error)
        .bind(expected_cleanup_started_at)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }
}
