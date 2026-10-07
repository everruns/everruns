//! Step hand-offs and stranded-run recovery on PostgreSQL; see
//! [`crate::HandOff`].
//!
//! Lock order: every write here locks the workflow row first, as a run start
//! and a claimed enqueue do, so a hand-off, a sweep and a run start on one
//! workflow serialize, and each re-reads the task rows after it got the lock.

use super::*;
use crate::persistence::store::Enqueued;
use crate::persistence::{HandOff, HandedOff, NextStep, RequeuedWorkflow, SignalDrain};

type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

fn db(action: &'static str) -> impl Fn(sqlx::Error) -> StoreError {
    move |e| store_failure("durable.tasks.hand_off", action, e)
}

/// Insert `task` claimed by `worker_id` with its `ActivityStarted` event,
/// when that worker is registered and not draining. Returns whether it did;
/// the caller locked the workflow row (see `enqueue_claimed_task`).
pub(super) async fn insert_claimed_task(
    tx: &mut Tx<'_>,
    task_id: Uuid,
    task: &TaskDefinition,
    worker_id: &str,
) -> Result<bool, StoreError> {
    let task_input = sanitize_json_null_bytes(task.input.clone());
    let options_json = serde_json::to_value(&task.options)
        .map(sanitize_json_null_bytes)
        .map_err(|e| StoreError::Serialization(e.to_string()))?;
    let started = serde_json::to_value(WorkflowEvent::ActivityStarted {
        activity_id: task.activity_id.clone(),
        attempt: 1,
        worker_id: worker_id.to_string(),
    })
    .map(sanitize_json_null_bytes)
    .map_err(|e| StoreError::Serialization(e.to_string()))?;

    let inserted: Option<Uuid> = sqlx::query_scalar(
        r#"
        WITH task AS (
            INSERT INTO durable_task_queue (
                id, workflow_id, activity_id, activity_type, input, options,
                max_attempts, priority, visible_at,
                schedule_to_start_timeout_ms, start_to_close_timeout_ms, heartbeat_timeout_ms,
                status, claimed_by, claimed_at, heartbeat_at, attempt, queue
            )
            SELECT $1, $2, $3, $4, $5, $6, $7, $8, NOW(), $9, $10, $11,
                   'claimed', $12, NOW(), NOW(), 1, $14
            WHERE EXISTS (
                SELECT 1 FROM durable_workers WHERE id = $12 AND status != 'draining'
            )
            RETURNING id, workflow_id
        ),
        started AS (
            INSERT INTO durable_workflow_events (workflow_id, sequence_num, event_type, event_data)
            SELECT task.workflow_id,
                   COALESCE((
                       SELECT MAX(sequence_num) + 1 FROM durable_workflow_events
                       WHERE workflow_id = task.workflow_id
                   ), 0),
                   'activity_started', $13
            FROM task
            WHERE task.workflow_id IS NOT NULL
        )
        SELECT id FROM task
        "#,
    )
    .bind(task_id)
    .bind(task.workflow_id)
    .bind(&task.activity_id)
    .bind(&task.activity_type)
    .bind(&task_input)
    .bind(&options_json)
    .bind(task.options.retry_policy.max_attempts as i32)
    .bind(task.options.priority)
    .bind(task.options.schedule_to_start_timeout.as_millis() as i64)
    .bind(task.options.start_to_close_timeout.as_millis() as i64)
    .bind(task.options.heartbeat_timeout.map(|d| d.as_millis() as i64))
    .bind(worker_id)
    .bind(&started)
    .bind(task.options.queue.as_deref())
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| {
        store_failure(
            "durable.tasks.enqueue_claimed",
            "Failed to enqueue claimed task",
            e,
        )
    })?;
    Ok(inserted.is_some())
}

/// Enqueue again, pending, the last step of each of `workflow_ids` that is
/// stranded: no pending or claimed task, and a last task that completed at
/// or before `completed_before`. The caller holds the workflow rows' locks.
async fn requeue_stranded_in(
    tx: &mut Tx<'_>,
    workflow_ids: &[Uuid],
    completed_before: DateTime<Utc>,
) -> Result<Vec<RequeuedWorkflow>, StoreError> {
    // The copy is a fresh task of the same step: same activity id, input,
    // options and queue, attempts from zero. `heartbeat_at` of a completed
    // task is its completion time (see `complete_task`).
    let rows = sqlx::query(
        r#"
        WITH last AS (
            SELECT DISTINCT ON (t.workflow_id) t.*
            FROM durable_task_queue t
            WHERE t.workflow_id = ANY($1)
            ORDER BY t.workflow_id, t.created_at DESC, t.id DESC
        )
        INSERT INTO durable_task_queue (
            workflow_id, activity_id, activity_type, input, options,
            max_attempts, priority, visible_at,
            schedule_to_start_timeout_ms, start_to_close_timeout_ms, heartbeat_timeout_ms,
            trace_id, span_id, queue
        )
        SELECT l.workflow_id, l.activity_id, l.activity_type, l.input, l.options,
               l.max_attempts, l.priority, NOW(),
               l.schedule_to_start_timeout_ms, l.start_to_close_timeout_ms, l.heartbeat_timeout_ms,
               l.trace_id, l.span_id, l.queue
        FROM last l
        WHERE l.status = 'completed'
          AND COALESCE(l.heartbeat_at, l.claimed_at, l.created_at) <= $2
          AND NOT EXISTS (
              SELECT 1 FROM durable_task_queue t
              WHERE t.workflow_id = l.workflow_id AND t.status IN ('pending', 'claimed')
          )
        RETURNING id, workflow_id, activity_type
        "#,
    )
    .bind(workflow_ids)
    .bind(completed_before)
    .fetch_all(&mut **tx)
    .await
    .map_err(db("Failed to requeue stranded workflow steps"))?;
    Ok(rows
        .into_iter()
        .map(|row| RequeuedWorkflow {
            workflow_id: row.get("workflow_id"),
            task_id: row.get("id"),
            activity_type: row.get("activity_type"),
        })
        .collect())
}

/// [`EventLog::start_run_with_task`]'s safeguard: resume `workflow_id` when
/// its run is stranded between steps. The caller holds the workflow lock.
pub(super) async fn resume_if_stranded(
    tx: &mut Tx<'_>,
    workflow_id: Uuid,
) -> Result<(), StoreError> {
    let resumed = requeue_stranded_in(tx, &[workflow_id], Utc::now()).await?;
    if !resumed.is_empty() {
        info!(%workflow_id, "resumed a run stranded between steps on run start");
    }
    Ok(())
}

impl PostgresWorkflowEventStore {
    pub(super) async fn hand_off(
        &self,
        task_id: Uuid,
        worker_id: &str,
        hand_off: HandOff,
    ) -> Result<HandedOff, StoreError> {
        let HandOff {
            workflow_id,
            drain,
            next,
        } = hand_off;
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(db("Failed to begin hand-off"))?;

        let status: Option<String> = sqlx::query_scalar(
            "SELECT status FROM durable_workflow_instances WHERE id = $1 FOR UPDATE",
        )
        .bind(workflow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db("Failed to lock workflow for hand-off"))?;
        let Some(status) = status else {
            return Err(StoreError::WorkflowNotFound(workflow_id));
        };
        let workflow_status = parse_workflow_status(&status)?;

        // As `complete_task`: only the claiming worker completes, and
        // `heartbeat_at` records when.
        let completed: Option<Uuid> = sqlx::query_scalar(
            r#"
            UPDATE durable_task_queue
            SET status = 'completed', heartbeat_at = NOW()
            WHERE id = $1 AND claimed_by = $2 AND status = 'claimed' AND workflow_id = $3
            RETURNING id
            "#,
        )
        .bind(task_id)
        .bind(worker_id)
        .bind(workflow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db("Failed to complete task in hand-off"))?;
        if completed.is_none() {
            tx.rollback().await.ok();
            debug!(%task_id, %worker_id, "hand-off rejected: task not owned");
            return Err(StoreError::TaskNotOwned(task_id));
        }

        let drained = match drain {
            Some(SignalDrain { signal_type, limit }) if limit > 0 => sqlx::query_scalar::<_, i64>(
                r#"
                    WITH taken AS (
                        UPDATE durable_signals
                        SET processed_at = NOW()
                        WHERE id IN (
                            SELECT id FROM durable_signals
                            WHERE workflow_id = $1 AND processed_at IS NULL AND signal_type = $2
                            ORDER BY sequence_num
                            LIMIT $3
                            FOR UPDATE
                        )
                        RETURNING id
                    )
                    SELECT COUNT(*) FROM taken
                    "#,
            )
            .bind(workflow_id)
            .bind(&signal_type)
            .bind(i64::try_from(limit).unwrap_or(i64::MAX))
            .fetch_one(&mut *tx)
            .await
            .map_err(db("Failed to drain signals in hand-off"))?,
            _ => 0,
        };

        let next = match next {
            NextStep::Enqueue {
                mut task,
                claim_for,
            } => {
                task.workflow_id = Some(workflow_id);
                let next_id = Uuid::now_v7();
                let may_claim =
                    task.options.start_delay.is_none() && !task.options.dedupe_by_activity_id;
                let claimed = match claim_for.as_deref() {
                    Some(worker) if may_claim => {
                        insert_claimed_task(&mut tx, next_id, &task, worker).await?
                    }
                    _ => false,
                };
                if claimed {
                    let worker = claim_for.unwrap_or_default();
                    debug!(%workflow_id, task_id = %next_id, %worker, "handed off claimed");
                    Some(Enqueued::Claimed(Box::new(ClaimedTask {
                        id: next_id,
                        workflow_id: Some(workflow_id),
                        max_attempts: task.options.retry_policy.max_attempts,
                        activity_id: task.activity_id,
                        activity_type: task.activity_type,
                        input: sanitize_json_null_bytes(task.input),
                        options: task.options,
                        attempt: 1,
                        workflow_status: Some(workflow_status),
                    })))
                } else {
                    let id = insert_workflow_task(&mut tx, workflow_id, &task).await?;
                    Some(Enqueued::Queued(id))
                }
            }
            NextStep::Complete { result, error } => {
                let error_json = error
                    .map(serde_json::to_value)
                    .transpose()
                    .map_err(|e| StoreError::Serialization(e.to_string()))?
                    .map(sanitize_json_null_bytes);
                // As `update_workflow_status` to Completed.
                sqlx::query(
                    r#"
                    UPDATE durable_workflow_instances
                    SET status = 'completed',
                        result = COALESCE($2, result),
                        error = COALESCE($3, error),
                        completed_at = NOW()
                    WHERE id = $1
                    "#,
                )
                .bind(workflow_id)
                .bind(result.map(sanitize_json_null_bytes))
                .bind(error_json)
                .execute(&mut *tx)
                .await
                .map_err(db("Failed to complete workflow in hand-off"))?;
                None
            }
        };

        tx.commit().await.map_err(db("Failed to commit hand-off"))?;
        Ok(HandedOff {
            drained: usize::try_from(drained).unwrap_or_default(),
            next,
        })
    }

    pub(super) async fn requeue_stranded(
        &self,
        workflow_type: &str,
        completed_before: Duration,
        limit: usize,
    ) -> Result<Vec<RequeuedWorkflow>, StoreError> {
        let cutoff = Utc::now() - chrono::Duration::from_std(completed_before).unwrap_or_default();
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(db("Failed to begin stranded sweep"))?;
        // Lock the candidates (skipping rows another sweep, start or hand-off
        // holds), then requeue in a second statement, which sees every task
        // committed before the locks were granted.
        let candidates: Vec<Uuid> = sqlx::query_scalar(
            r#"
            SELECT w.id
            FROM durable_workflow_instances w
            CROSS JOIN LATERAL (
                SELECT t.status, COALESCE(t.heartbeat_at, t.claimed_at, t.created_at) AS done_at
                FROM durable_task_queue t
                WHERE t.workflow_id = w.id
                ORDER BY t.created_at DESC, t.id DESC
                LIMIT 1
            ) last
            WHERE w.status = 'running'
              AND w.workflow_type = $1
              AND last.status = 'completed'
              AND last.done_at <= $2
            ORDER BY last.done_at
            LIMIT $3
            FOR UPDATE OF w SKIP LOCKED
            "#,
        )
        .bind(workflow_type)
        .bind(cutoff)
        .bind(i64::try_from(limit).unwrap_or(i64::MAX))
        .fetch_all(&mut *tx)
        .await
        .map_err(db("Failed to find stranded workflows"))?;
        if candidates.is_empty() {
            tx.rollback().await.ok();
            return Ok(Vec::new());
        }
        let requeued = requeue_stranded_in(&mut tx, &candidates, cutoff).await?;
        tx.commit()
            .await
            .map_err(db("Failed to commit stranded sweep"))?;
        Ok(requeued)
    }
}
