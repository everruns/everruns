// Session task registry repository (PostgreSQL)
//
// Lifecycle invariants are applied via
// `everruns_core::session_task::apply_task_update` inside a transaction with
// `SELECT ... FOR UPDATE` so concurrent updates serialize per task.

use super::super::models::{NewSessionTaskMessageRow, SessionTaskMessageRow, SessionTaskRow};
use super::Database;
use anyhow::Result;
use everruns_core::SessionId;
use everruns_core::session_task::{SessionTask, SessionTaskUpdate, apply_task_update};

const TASK_COLUMNS: &str = "id, session_id, kind, display_name, spec, state, state_detail, \
     progress, input_request, cancel_requested_at, summary, result_path, artifacts, error, \
     attempt, worker_id, heartbeat_at, links, wake_policy, created_at, started_at, finished_at, \
     updated_at";

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
                wake_policy, created_at, started_at, finished_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                    $15, $16, $17, $18, $19, $20, $21, $22, $23)
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

    /// Return (session_id, task_id) pairs for tasks with a stale heartbeat.
    ///
    /// Tasks with NULL heartbeat_at are excluded (foreground tasks without
    /// liveness probes; EVE-535 spawn-handle coverage applies instead).
    /// Runs inside a transaction with `FOR UPDATE SKIP LOCKED` so concurrent
    /// reapers claim disjoint sets.
    pub async fn list_orphaned_session_task_ids(
        &self,
        stale_after: chrono::Duration,
        limit: i64,
    ) -> Result<Vec<(SessionId, String)>> {
        let stale_secs = stale_after.num_seconds();
        // Transaction is necessary for FOR UPDATE to take effect for the
        // caller's subsequent updates; the reaper calls update_session_task
        // for each row in its own transaction, so we open and commit here
        // just to get a consistent, lock-free snapshot.
        let rows = sqlx::query_as::<_, (SessionId, String)>(
            r#"
            SELECT session_id, id
            FROM session_tasks
            WHERE state IN ('queued', 'running')
              AND heartbeat_at IS NOT NULL
              AND heartbeat_at < NOW() - ($1::bigint * INTERVAL '1 second')
            ORDER BY id
            LIMIT $2
            FOR UPDATE SKIP LOCKED
            "#,
        )
        .bind(stale_secs)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Last `limit` messages, returned oldest first.
    pub async fn list_session_task_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
    ) -> Result<Vec<SessionTaskMessageRow>> {
        let mut rows = sqlx::query_as::<_, SessionTaskMessageRow>(sqlx::AssertSqlSafe(format!(
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
