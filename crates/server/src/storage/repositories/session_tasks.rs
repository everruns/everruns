// Session task registry repository (PostgreSQL)
//
// Lifecycle invariants are applied via
// `everruns_core::session_task::apply_task_update` inside a transaction with
// `SELECT ... FOR UPDATE` so concurrent updates serialize per task.

use super::super::models::{
    CreateSessionTaskPushConfig, NewSessionTaskMessageRow, SessionTaskMessageRow,
    SessionTaskPushConfigRow, SessionTaskRow,
};
use super::Database;
use anyhow::Result;
use everruns_core::session_task::{SessionTask, SessionTaskUpdate, apply_task_update};
use everruns_provider::typed_id::SessionId;

const PUSH_CONFIG_COLUMNS: &str =
    "id, public_id, session_id, task_id, url, secret, event_filter, created_at, updated_at";

const TASK_COLUMNS: &str = "id, session_id, root_session_id, kind, display_name, spec, state, \
     state_detail, progress, input_request, cancel_requested_at, summary, result_path, artifacts, \
     error, attempt, worker_id, heartbeat_at, links, wake_policy, created_at, started_at, \
     finished_at, updated_at";

const MESSAGE_COLUMNS: &str =
    "id, task_id, session_id, direction, content, in_reply_to, created_at";

impl Database {
    /// Insert a task. Idempotent on `id`: when the row already exists it is
    /// returned unchanged and the `bool` is false.
    pub async fn create_session_task(&self, task: &SessionTask) -> Result<(SessionTaskRow, bool)> {
        let row = SessionTaskRow::from_task(task)?;
        let inserted = sqlx::query_as::<_, SessionTaskRow>(sqlx::AssertSqlSafe(format!(
            r#"
            INSERT INTO session_tasks (
                id, session_id, kind, display_name, spec, state, state_detail,
                progress, input_request, cancel_requested_at, summary, result_path,
                artifacts, error, attempt, worker_id, heartbeat_at, links,
                wake_policy, created_at, started_at, finished_at, updated_at,
                root_session_id
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                    $15, $16, $17, $18, $19, $20, $21, $22, $23,
                    -- EVE-680: denormalize the owning session's tree root so the
                    -- org task list can filter a whole tree by a local column.
                    (SELECT root_session_id FROM sessions WHERE id = $2))
            ON CONFLICT (id) DO NOTHING
            RETURNING {TASK_COLUMNS}
            "#
        )))
        .bind(&row.id)
        .bind(row.session_id)
        .bind(&row.kind)
        .bind(&row.display_name)
        .bind(&row.spec)
        .bind(&row.state)
        .bind(&row.state_detail)
        .bind(&row.progress)
        .bind(&row.input_request)
        .bind(row.cancel_requested_at)
        .bind(&row.summary)
        .bind(&row.result_path)
        .bind(&row.artifacts)
        .bind(&row.error)
        .bind(row.attempt)
        .bind(&row.worker_id)
        .bind(row.heartbeat_at)
        .bind(&row.links)
        .bind(&row.wake_policy)
        .bind(row.created_at)
        .bind(row.started_at)
        .bind(row.finished_at)
        .bind(row.updated_at)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(inserted) = inserted {
            return Ok((inserted, true));
        }

        // Idempotency is scoped to the owning session: an id that exists under
        // a different session is a caller bug, not a replay.
        let existing = sqlx::query_as::<_, SessionTaskRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {TASK_COLUMNS} FROM session_tasks WHERE id = $1 AND session_id = $2"
        )))
        .bind(&row.id)
        .bind(row.session_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "session task id {} already exists in a different session",
                row.id
            )
        })?;
        Ok((existing, false))
    }

    pub async fn get_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTaskRow>> {
        let row = sqlx::query_as::<_, SessionTaskRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {TASK_COLUMNS} FROM session_tasks WHERE session_id = $1 AND id = $2"
        )))
        .bind(session_id)
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_session_tasks(
        &self,
        session_id: SessionId,
        kind: Option<&str>,
        state: Option<&str>,
    ) -> Result<Vec<SessionTaskRow>> {
        let rows = sqlx::query_as::<_, SessionTaskRow>(sqlx::AssertSqlSafe(format!(
            r#"
            SELECT {TASK_COLUMNS}
            FROM session_tasks
            WHERE session_id = $1
              AND ($2::text IS NULL OR kind = $2)
              AND ($3::text IS NULL OR state = $3)
            ORDER BY created_at ASC, id ASC
            "#
        )))
        .bind(session_id)
        .bind(kind)
        .bind(state)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// List tasks across every session owned by `org_id`, newest-first, with
    /// optional kind/state/age filters and a bounded limit. The org boundary is
    /// a semijoin on `sessions.org_id` — the authoritative multitenancy scope —
    /// so a task only appears when its owning session belongs to the org.
    #[allow(clippy::too_many_arguments)]
    pub async fn list_org_session_tasks(
        &self,
        org_id: i64,
        kind: Option<&str>,
        state: Option<&str>,
        created_after: Option<chrono::DateTime<chrono::Utc>>,
        root_session_id: Option<SessionId>,
        limit: i64,
    ) -> Result<Vec<SessionTaskRow>> {
        let rows = sqlx::query_as::<_, SessionTaskRow>(sqlx::AssertSqlSafe(format!(
            r#"
            SELECT {TASK_COLUMNS}
            FROM session_tasks
            WHERE session_id IN (SELECT id FROM sessions WHERE org_id = $1)
              AND ($2::text IS NULL OR kind = $2)
              AND ($3::text IS NULL OR state = $3)
              AND ($4::timestamptz IS NULL OR created_at >= $4)
              AND ($5::uuid IS NULL OR root_session_id = $5)
            ORDER BY created_at DESC, id DESC
            LIMIT $6
            "#
        )))
        .bind(org_id)
        .bind(kind)
        .bind(state)
        .bind(created_after)
        .bind(root_session_id.map(|id| id.uuid()))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Load the task, apply the update through core invariants, write back.
    /// Runs in a transaction with `SELECT ... FOR UPDATE`.
    pub async fn update_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTaskRow>> {
        let mut tx = self.pool.begin().await?;

        let row = sqlx::query_as::<_, SessionTaskRow>(sqlx::AssertSqlSafe(format!(
            "SELECT {TASK_COLUMNS} FROM session_tasks \
             WHERE session_id = $1 AND id = $2 FOR UPDATE"
        )))
        .bind(session_id)
        .bind(task_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(row) = row else {
            tx.rollback().await?;
            return Ok(None);
        };

        let mut task = row.to_task()?;
        apply_task_update(&mut task, update, chrono::Utc::now());
        let updated = SessionTaskRow::from_task(&task)?;

        sqlx::query(
            r#"
            UPDATE session_tasks SET
                state = $3, state_detail = $4, progress = $5, input_request = $6,
                summary = $7, result_path = $8, artifacts = $9, error = $10,
                attempt = $11, worker_id = $12, heartbeat_at = $13, links = $14,
                started_at = $15, finished_at = $16, updated_at = $17
            WHERE session_id = $1 AND id = $2
            "#,
        )
        .bind(session_id)
        .bind(task_id)
        .bind(&updated.state)
        .bind(&updated.state_detail)
        .bind(&updated.progress)
        .bind(&updated.input_request)
        .bind(&updated.summary)
        .bind(&updated.result_path)
        .bind(&updated.artifacts)
        .bind(&updated.error)
        .bind(updated.attempt)
        .bind(&updated.worker_id)
        .bind(updated.heartbeat_at)
        .bind(&updated.links)
        .bind(updated.started_at)
        .bind(updated.finished_at)
        .bind(updated.updated_at)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(updated))
    }

    /// Record cooperative cancel intent. Idempotent: sets
    /// `cancel_requested_at` only when NULL. Returns the row plus whether
    /// this call changed it.
    pub async fn request_cancel_session_task(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<(SessionTaskRow, bool)>> {
        let changed = sqlx::query_as::<_, SessionTaskRow>(sqlx::AssertSqlSafe(format!(
            r#"
            UPDATE session_tasks
            SET cancel_requested_at = NOW(), updated_at = NOW()
            WHERE session_id = $1 AND id = $2 AND cancel_requested_at IS NULL
            RETURNING {TASK_COLUMNS}
            "#
        )))
        .bind(session_id)
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = changed {
            return Ok(Some((row, true)));
        }

        Ok(self
            .get_session_task(session_id, task_id)
            .await?
            .map(|row| (row, false)))
    }

    // ============================================
    // Per-task push-notification configs (EVE-682)
    // ============================================

    /// Create a per-task push config. Authorization is the caller's concern:
    /// this method trusts `session_id`/`task_id` were resolved against the
    /// caller's org before insert.
    pub async fn create_task_push_config(
        &self,
        input: CreateSessionTaskPushConfig,
    ) -> Result<SessionTaskPushConfigRow> {
        let row = sqlx::query_as::<_, SessionTaskPushConfigRow>(sqlx::AssertSqlSafe(format!(
            r#"
            INSERT INTO session_task_push_configs
                (public_id, session_id, task_id, url, secret, event_filter)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING {PUSH_CONFIG_COLUMNS}
            "#
        )))
        .bind(&input.public_id)
        .bind(input.session_id)
        .bind(&input.task_id)
        .bind(&input.url)
        .bind(&input.secret)
        .bind(&input.event_filter)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// List push configs for one task, oldest-first.
    pub async fn list_task_push_configs(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Vec<SessionTaskPushConfigRow>> {
        let rows = sqlx::query_as::<_, SessionTaskPushConfigRow>(sqlx::AssertSqlSafe(format!(
            r#"
            SELECT {PUSH_CONFIG_COLUMNS}
            FROM session_task_push_configs
            WHERE session_id = $1 AND task_id = $2
            ORDER BY created_at ASC, id ASC
            "#
        )))
        .bind(session_id)
        .bind(task_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Delete one push config by its public id, scoped to the owning task.
    /// Returns whether a row was removed.
    pub async fn delete_task_push_config(
        &self,
        session_id: SessionId,
        task_id: &str,
        public_id: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            "DELETE FROM session_task_push_configs \
             WHERE session_id = $1 AND task_id = $2 AND public_id = $3",
        )
        .bind(session_id)
        .bind(task_id)
        .bind(public_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn insert_session_task_message(
        &self,
        input: NewSessionTaskMessageRow,
    ) -> Result<SessionTaskMessageRow> {
        let row = sqlx::query_as::<_, SessionTaskMessageRow>(sqlx::AssertSqlSafe(format!(
            r#"
            INSERT INTO session_task_messages
                (id, task_id, session_id, direction, content, in_reply_to)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING {MESSAGE_COLUMNS}
            "#
        )))
        .bind(&input.id)
        .bind(&input.task_id)
        .bind(input.session_id)
        .bind(&input.direction)
        .bind(&input.content)
        .bind(&input.in_reply_to)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Return (session_id, task_id, schedule_id) triples for running monitor
    /// tasks whose linked schedule is inactive.
    ///
    /// "Inactive" = the schedule row does not exist, OR it is disabled and
    /// either recurring or a never-triggered one-shot. Fired one-shots are
    /// excluded because `mark_triggered` disables them before the firing path
    /// can complete the linked monitor.
    ///
    /// Plain snapshot read — safe for concurrent sweepers because transitions
    /// are applied through `apply_task_update` where terminal states are final.
    pub async fn list_monitor_tasks_with_inactive_schedules(
        &self,
        limit: i64,
    ) -> Result<Vec<(SessionId, String, String)>> {
        // spec->>'schedule_id' holds the PREFIXED typed id ("sched_<32hex>",
        // see ScheduleId::to_string), so it cannot be cast to uuid directly.
        // Compare dash-stripped schedule UUIDs against the hex suffix instead
        // (no cast on user-shaped data — a malformed value can never error the
        // sweep), and only consider well-formed ids so malformed specs are
        // skipped rather than canceled.
        let rows = sqlx::query_as::<_, (SessionId, String, String)>(
            r#"
            SELECT st.session_id, st.id, st.spec->>'schedule_id'
            FROM session_tasks st
            LEFT JOIN session_schedules ss
                   ON REPLACE(ss.id::text, '-', '')
                      = SUBSTRING(st.spec->>'schedule_id' FROM 7)
            WHERE st.kind = 'monitor'
              AND st.state = 'running'
              AND st.spec->>'schedule_id' ~ '^sched_[0-9a-f]{32}$'
              AND (
                  ss.id IS NULL
                  OR (
                      ss.enabled = false
                      AND (
                          ss.cron_expression IS NOT NULL
                          OR (ss.last_triggered_at IS NULL AND ss.trigger_count = 0)
                      )
                  )
              )
            ORDER BY st.id
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Return (session_id, task_id) pairs for tasks with a stale heartbeat.
    ///
    /// Tasks with NULL heartbeat_at are excluded (foreground tasks without
    /// liveness probes; EVE-535 spawn-handle coverage applies instead).
    ///
    /// This is a plain snapshot read — no transaction is held across the
    /// reaper's subsequent per-task updates, so concurrent reapers may pick
    /// overlapping sets. That is safe: failing a task is applied through
    /// `apply_task_update`, where terminal states are final, so a second
    /// reaper's update is an idempotent no-op.
    pub async fn list_orphaned_session_task_ids(
        &self,
        stale_after: chrono::Duration,
        limit: i64,
    ) -> Result<Vec<(SessionId, String)>> {
        let stale_secs = stale_after.num_seconds();
        let rows = sqlx::query_as::<_, (SessionId, String)>(
            r#"
            SELECT session_id, id
            FROM session_tasks
            WHERE state IN ('queued', 'running')
              AND heartbeat_at IS NOT NULL
              AND heartbeat_at < NOW() - ($1::bigint * INTERVAL '1 second')
            ORDER BY id
            LIMIT $2
            "#,
        )
        .bind(stale_secs)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Prune terminal session tasks whose `finished_at` is older than the TTL
    /// cutoff, in a bounded batch. Deletes the `session_tasks` rows (their
    /// `session_task_messages` cascade via the ON DELETE CASCADE FK) and
    /// returns the `(session_id, task_id, result_path)` triples that were
    /// removed, so the caller can clean up `result_path` artifacts after the
    /// row delete commits (EVE-580).
    ///
    /// Only terminal states ('succeeded', 'failed', 'canceled') with a
    /// non-NULL `finished_at` strictly older than `cutoff` are eligible —
    /// live/running/queued tasks are never touched. The `LIMIT` bounds work
    /// per reaper tick so a large backlog can't wedge the tick or blow memory;
    /// a backlog is drained across successive ticks.
    ///
    /// The query is global/by-age (not org-scoped) but deletes are keyed on
    /// each task's own primary key, so it cannot cross-delete between orgs.
    pub async fn prune_terminal_session_tasks(
        &self,
        cutoff: chrono::DateTime<chrono::Utc>,
        limit: i64,
    ) -> Result<Vec<(SessionId, String, Option<String>)>> {
        // Single statement: select the eligible batch and delete it atomically,
        // returning what was removed. Messages cascade via the FK. Selecting in
        // a CTE with the partial terminal+finished_at index keeps the scan cheap.
        let rows = sqlx::query_as::<_, (SessionId, String, Option<String>)>(
            r#"
            WITH eligible AS (
                SELECT id
                FROM session_tasks
                WHERE state IN ('succeeded', 'failed', 'canceled')
                  AND finished_at IS NOT NULL
                  AND finished_at < $1
                ORDER BY finished_at ASC
                LIMIT $2
            )
            DELETE FROM session_tasks st
            USING eligible e
            WHERE st.id = e.id
            RETURNING st.session_id, st.id, st.result_path
            "#,
        )
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Messages on the task channel, oldest first.
    ///
    /// When `after_id` is `Some`, only messages newer than that message are
    /// returned (exclusive cursor). When `after_id` is `None`, the latest
    /// `limit` messages are returned.
    pub async fn list_session_task_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<SessionTaskMessageRow>> {
        if let Some(cursor_id) = after_id {
            let rows = sqlx::query_as::<_, SessionTaskMessageRow>(sqlx::AssertSqlSafe(format!(
                r#"
                SELECT {MESSAGE_COLUMNS}
                FROM session_task_messages
                WHERE session_id = $1 AND task_id = $2
                  AND (created_at, id) > (
                    SELECT created_at, id
                    FROM session_task_messages
                    WHERE session_id = $1 AND task_id = $2 AND id = $3
                  )
                ORDER BY created_at ASC, id ASC
                LIMIT $4
                "#
            )))
            .bind(session_id)
            .bind(task_id)
            .bind(cursor_id)
            .bind(limit.map(i64::from).unwrap_or(i64::MAX))
            .fetch_all(&self.pool)
            .await?;
            Ok(rows)
        } else {
            let mut rows =
                sqlx::query_as::<_, SessionTaskMessageRow>(sqlx::AssertSqlSafe(format!(
                    r#"
                    SELECT {MESSAGE_COLUMNS}
                    FROM session_task_messages
                    WHERE session_id = $1 AND task_id = $2
                    ORDER BY created_at DESC, id DESC
                    LIMIT $3
                    "#
                )))
                .bind(session_id)
                .bind(task_id)
                .bind(limit.map(i64::from).unwrap_or(i64::MAX))
                .fetch_all(&self.pool)
                .await?;
            rows.reverse();
            Ok(rows)
        }
    }
}
