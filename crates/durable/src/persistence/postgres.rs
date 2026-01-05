//! PostgreSQL implementation of WorkflowEventStore
//!
//! Production-ready persistence using PostgreSQL with:
//! - Optimistic concurrency control via sequence numbers
//! - Efficient task claiming with SKIP LOCKED
//! - Event sourcing for workflow replay

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use tracing::{debug, error, instrument};
use uuid::Uuid;

use super::store::*;
use crate::workflow::{ActivityOptions, WorkflowError, WorkflowEvent, WorkflowSignal};

/// PostgreSQL implementation of WorkflowEventStore
///
/// Uses a connection pool for efficient database access.
/// Designed for high-throughput with 1000+ concurrent workers.
///
/// # Example
///
/// ```ignore
/// use everruns_durable::PostgresWorkflowEventStore;
/// use sqlx::PgPool;
///
/// let pool = PgPool::connect("postgres://localhost/mydb").await?;
/// let store = PostgresWorkflowEventStore::new(pool);
/// ```
#[derive(Clone)]
pub struct PostgresWorkflowEventStore {
    pool: PgPool,
}

impl PostgresWorkflowEventStore {
    /// Create a new PostgreSQL store with the given connection pool
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Get a reference to the connection pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl WorkflowEventStore for PostgresWorkflowEventStore {
    #[instrument(skip(self, input, trace_context))]
    async fn create_workflow(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        trace_context: Option<&TraceContext>,
    ) -> Result<(), StoreError> {
        let (trace_id, span_id) = trace_context
            .map(|tc| (Some(tc.trace_id.clone()), Some(tc.span_id.clone())))
            .unwrap_or((None, None));

        sqlx::query(
            r#"
            INSERT INTO durable_workflow_instances (id, workflow_type, status, input, trace_id, span_id)
            VALUES ($1, $2, 'pending', $3, $4, $5)
            "#,
        )
        .bind(workflow_id)
        .bind(workflow_type)
        .bind(&input)
        .bind(&trace_id)
        .bind(&span_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to create workflow: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, %workflow_type, "created workflow");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT status FROM durable_workflow_instances WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get workflow status: {}", e);
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        let status: String = row.get("status");
        parse_workflow_status(&status)
    }

    #[instrument(skip(self))]
    async fn get_workflow_info(&self, workflow_id: Uuid) -> Result<WorkflowInfo, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, workflow_type, status, input, result, error
            FROM durable_workflow_instances
            WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get workflow info: {}", e);
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        let status_str: String = row.get("status");
        let error_json: Option<serde_json::Value> = row.get("error");

        Ok(WorkflowInfo {
            id: row.get("id"),
            workflow_type: row.get("workflow_type"),
            status: parse_workflow_status(&status_str)?,
            input: row.get("input"),
            result: row.get("result"),
            error: error_json.and_then(|v| serde_json::from_value(v).ok()),
        })
    }

    #[instrument(skip(self, events))]
    async fn append_events(
        &self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32, StoreError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        // Check current sequence with lock
        let row = sqlx::query(
            r#"
            SELECT COALESCE(MAX(sequence_num) + 1, 0) as next_seq
            FROM durable_workflow_events
            WHERE workflow_id = $1
            FOR UPDATE
            "#,
        )
        .bind(workflow_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        let current_sequence: i32 = row.get::<i64, _>("next_seq") as i32;

        if current_sequence != expected_sequence {
            return Err(StoreError::ConcurrencyConflict {
                expected: expected_sequence,
                actual: current_sequence,
            });
        }

        // Insert events
        let mut new_sequence = current_sequence;
        for event in events {
            let event_type = event_type_name(&event);
            let event_data = serde_json::to_value(&event)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;

            sqlx::query(
                r#"
                INSERT INTO durable_workflow_events (workflow_id, sequence_num, event_type, event_data)
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(workflow_id)
            .bind(new_sequence)
            .bind(event_type)
            .bind(&event_data)
            .execute(&mut *tx)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            new_sequence += 1;
        }

        tx.commit()
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        debug!(%workflow_id, new_sequence, "appended events");
        Ok(new_sequence)
    }

    #[instrument(skip(self))]
    async fn load_events(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT sequence_num, event_data
            FROM durable_workflow_events
            WHERE workflow_id = $1
            ORDER BY sequence_num
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to load events: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i32 = row.get::<i64, _>("sequence_num") as i32;
            let data: serde_json::Value = row.get("event_data");
            let event: WorkflowEvent = serde_json::from_value(data)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            events.push((seq, event));
        }

        Ok(events)
    }

    #[instrument(skip(self, result, error))]
    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        result: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let status_str = status.to_string();
        let error_json = error
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        let (started_at, completed_at): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
            match status {
                WorkflowStatus::Running => (Some(Utc::now()), None),
                WorkflowStatus::Completed | WorkflowStatus::Failed | WorkflowStatus::Cancelled => {
                    (None, Some(Utc::now()))
                }
                WorkflowStatus::Pending => (None, None),
            };

        sqlx::query(
            r#"
            UPDATE durable_workflow_instances
            SET status = $2,
                result = COALESCE($3, result),
                error = COALESCE($4, error),
                started_at = COALESCE($5, started_at),
                completed_at = COALESCE($6, completed_at)
            WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .bind(&status_str)
        .bind(&result)
        .bind(&error_json)
        .bind(started_at)
        .bind(completed_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to update workflow status: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, %status_str, "updated workflow status");
        Ok(())
    }

    #[instrument(skip(self, task))]
    async fn enqueue_task(&self, task: TaskDefinition) -> Result<Uuid, StoreError> {
        let task_id = Uuid::now_v7();
        let options_json = serde_json::to_value(&task.options)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO durable_task_queue (
                id, workflow_id, activity_id, activity_type, input, options,
                max_attempts, priority,
                schedule_to_start_timeout_ms, start_to_close_timeout_ms, heartbeat_timeout_ms
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#,
        )
        .bind(task_id)
        .bind(task.workflow_id)
        .bind(&task.activity_id)
        .bind(&task.activity_type)
        .bind(&task.input)
        .bind(&options_json)
        .bind(task.options.retry_policy.max_attempts as i32)
        .bind(task.options.priority)
        .bind(task.options.schedule_to_start_timeout.as_millis() as i64)
        .bind(task.options.start_to_close_timeout.as_millis() as i64)
        .bind(task.options.heartbeat_timeout.map(|d| d.as_millis() as i64))
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to enqueue task: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%task_id, activity_type = %task.activity_type, "enqueued task");
        Ok(task_id)
    }

    #[instrument(skip(self, activity_types))]
    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        if activity_types.is_empty() {
            return Ok(vec![]);
        }

        // Use SKIP LOCKED for efficient concurrent claiming
        // This query:
        // 1. Finds pending tasks matching activity types
        // 2. Orders by priority (desc) then visibility time
        // 3. Limits to max_tasks
        // 4. Uses SKIP LOCKED to avoid contention
        // 5. Updates status and claiming info in one atomic operation
        let rows = sqlx::query(
            r#"
            WITH claimable AS (
                SELECT id
                FROM durable_task_queue
                WHERE status = 'pending'
                  AND activity_type = ANY($1)
                  AND visible_at <= NOW()
                ORDER BY priority DESC, visible_at
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            )
            UPDATE durable_task_queue t
            SET status = 'claimed',
                claimed_by = $3,
                claimed_at = NOW(),
                heartbeat_at = NOW(),
                attempt = attempt + 1
            FROM claimable c
            WHERE t.id = c.id
            RETURNING t.id, t.workflow_id, t.activity_id, t.activity_type,
                      t.input, t.options, t.attempt, t.max_attempts
            "#,
        )
        .bind(activity_types)
        .bind(max_tasks as i32)
        .bind(worker_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to claim tasks: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let options_json: serde_json::Value = row.get("options");
            let options: ActivityOptions = serde_json::from_value(options_json)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;

            claimed.push(ClaimedTask {
                id: row.get("id"),
                workflow_id: row.get("workflow_id"),
                activity_id: row.get("activity_id"),
                activity_type: row.get("activity_type"),
                input: row.get("input"),
                options,
                attempt: row.get::<i32, _>("attempt") as u32,
                max_attempts: row.get::<i32, _>("max_attempts") as u32,
            });
        }

        if !claimed.is_empty() {
            debug!(worker_id, count = claimed.len(), "claimed tasks");
        }

        Ok(claimed)
    }

    #[instrument(skip(self, _details))]
    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        _details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        // Update heartbeat and check if task is still claimed by this worker
        let result = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET heartbeat_at = NOW()
            WHERE id = $1 AND claimed_by = $2 AND status = 'claimed'
            RETURNING status
            "#,
        )
        .bind(task_id)
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to heartbeat task: {}", e);
            StoreError::Database(e.to_string())
        })?;

        match result {
            Some(_) => Ok(HeartbeatResponse {
                accepted: true,
                should_cancel: false, // TODO: Check for cancellation requests
            }),
            None => {
                // Task no longer claimed by this worker (maybe reclaimed or completed)
                Ok(HeartbeatResponse {
                    accepted: false,
                    should_cancel: true,
                })
            }
        }
    }

    #[instrument(skip(self, _result))]
    async fn complete_task(
        &self,
        task_id: Uuid,
        _result: serde_json::Value,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'completed'
            WHERE id = $1
            "#,
        )
        .bind(task_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to complete task: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%task_id, "completed task");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn fail_task(
        &self,
        task_id: Uuid,
        error: &str,
    ) -> Result<TaskFailureOutcome, StoreError> {
        // Get current task state
        let row = sqlx::query(
            r#"
            SELECT attempt, max_attempts, options
            FROM durable_task_queue
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?
        .ok_or(StoreError::TaskNotFound(task_id))?;

        let attempt: i32 = row.get("attempt");
        let max_attempts: i32 = row.get("max_attempts");
        let options_json: serde_json::Value = row.get("options");
        let options: ActivityOptions = serde_json::from_value(options_json)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        if attempt < max_attempts {
            // Calculate retry delay
            let delay = options.retry_policy.delay_for_attempt((attempt + 1) as u32);
            let visible_at = Utc::now() + chrono::Duration::from_std(delay).unwrap_or_default();

            // Requeue for retry
            sqlx::query(
                r#"
                UPDATE durable_task_queue
                SET status = 'pending',
                    claimed_by = NULL,
                    claimed_at = NULL,
                    heartbeat_at = NULL,
                    last_error = $2,
                    visible_at = $3
                WHERE id = $1
                "#,
            )
            .bind(task_id)
            .bind(error)
            .bind(visible_at)
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            debug!(%task_id, next_attempt = attempt + 1, "task will retry");
            Ok(TaskFailureOutcome::WillRetry {
                next_attempt: (attempt + 1) as u32,
                delay,
            })
        } else {
            // Move to DLQ
            sqlx::query(
                r#"
                UPDATE durable_task_queue
                SET status = 'dead',
                    last_error = $2
                WHERE id = $1
                "#,
            )
            .bind(task_id)
            .bind(error)
            .execute(&self.pool)
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

            debug!(%task_id, "task moved to DLQ");
            Ok(TaskFailureOutcome::MovedToDlq)
        }
    }

    #[instrument(skip(self))]
    async fn reclaim_stale_tasks(
        &self,
        stale_threshold: Duration,
    ) -> Result<Vec<Uuid>, StoreError> {
        let threshold =
            Utc::now() - chrono::Duration::from_std(stale_threshold).unwrap_or_default();

        // Find and reclaim stale tasks
        let rows = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'pending',
                claimed_by = NULL,
                claimed_at = NULL
            WHERE status = 'claimed'
              AND heartbeat_at < $1
            RETURNING id
            "#,
        )
        .bind(threshold)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to reclaim stale tasks: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let reclaimed: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();

        if !reclaimed.is_empty() {
            debug!(count = reclaimed.len(), "reclaimed stale tasks");
        }

        Ok(reclaimed)
    }

    #[instrument(skip(self, signal))]
    async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO durable_signals (workflow_id, signal_type, payload, sent_at)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(workflow_id)
        .bind(&signal.signal_type)
        .bind(&signal.payload)
        .bind(signal.sent_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to send signal: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, signal_type = %signal.signal_type, "sent signal");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT signal_type, payload, sent_at
            FROM durable_signals
            WHERE workflow_id = $1 AND processed_at IS NULL
            ORDER BY sequence_num
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get pending signals: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let signals = rows
            .into_iter()
            .map(|row| WorkflowSignal {
                signal_type: row.get("signal_type"),
                payload: row.get("payload"),
                sent_at: row.get("sent_at"),
            })
            .collect();

        Ok(signals)
    }

    #[instrument(skip(self))]
    async fn mark_signals_processed(
        &self,
        workflow_id: Uuid,
        count: usize,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_signals
            SET processed_at = NOW()
            WHERE id IN (
                SELECT id FROM durable_signals
                WHERE workflow_id = $1 AND processed_at IS NULL
                ORDER BY sequence_num
                LIMIT $2
            )
            "#,
        )
        .bind(workflow_id)
        .bind(count as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to mark signals processed: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, count, "marked signals as processed");
        Ok(())
    }

    #[instrument(skip(self, worker))]
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO durable_workers (
                id, worker_group, activity_types, max_concurrency, current_load,
                status, started_at, last_heartbeat_at, accepting_tasks, hostname, metadata
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (id) DO UPDATE SET
                worker_group = EXCLUDED.worker_group,
                activity_types = EXCLUDED.activity_types,
                max_concurrency = EXCLUDED.max_concurrency,
                current_load = EXCLUDED.current_load,
                status = EXCLUDED.status,
                last_heartbeat_at = EXCLUDED.last_heartbeat_at,
                accepting_tasks = EXCLUDED.accepting_tasks
            "#,
        )
        .bind(&worker.id)
        .bind(&worker.worker_group)
        .bind(&worker.activity_types)
        .bind(worker.max_concurrency as i32)
        .bind(worker.current_load as i32)
        .bind(&worker.status)
        .bind(worker.started_at)
        .bind(worker.last_heartbeat_at)
        .bind(worker.accepting_tasks)
        .bind::<Option<String>>(None) // hostname
        .bind::<Option<serde_json::Value>>(None) // metadata
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to register worker: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(worker_id = %worker.id, "registered worker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_workers
            SET last_heartbeat_at = NOW(),
                current_load = $2,
                accepting_tasks = $3
            WHERE id = $1
            "#,
        )
        .bind(worker_id)
        .bind(current_load as i32)
        .bind(accepting_tasks)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to update worker heartbeat: {}", e);
            StoreError::Database(e.to_string())
        })?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn list_workers(&self, filter: WorkerFilter) -> Result<Vec<WorkerInfo>, StoreError> {
        let mut query = String::from(
            r#"
            SELECT id, worker_group, activity_types, max_concurrency, current_load,
                   status, started_at, last_heartbeat_at, accepting_tasks
            FROM durable_workers
            WHERE 1=1
            "#,
        );

        if filter.status.is_some() {
            query.push_str(" AND status = $1");
        }
        if filter.worker_group.is_some() {
            query.push_str(" AND worker_group = $2");
        }

        let rows = sqlx::query(&query)
            .bind(&filter.status)
            .bind(&filter.worker_group)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                error!("Failed to list workers: {}", e);
                StoreError::Database(e.to_string())
            })?;

        let workers = rows
            .into_iter()
            .map(|row| WorkerInfo {
                id: row.get("id"),
                worker_group: row.get("worker_group"),
                activity_types: row.get("activity_types"),
                max_concurrency: row.get::<i32, _>("max_concurrency") as u32,
                current_load: row.get::<i32, _>("current_load") as u32,
                status: row.get("status"),
                accepting_tasks: row.get("accepting_tasks"),
                started_at: row.get("started_at"),
                last_heartbeat_at: row.get("last_heartbeat_at"),
            })
            .collect();

        Ok(workers)
    }

    #[instrument(skip(self))]
    async fn deregister_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_workers
            SET status = 'stopped'
            WHERE id = $1
            "#,
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to deregister worker: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(worker_id, "deregistered worker");
        Ok(())
    }

    #[instrument(skip(self, error_history))]
    async fn move_to_dlq(
        &self,
        task_id: Uuid,
        error_history: Vec<String>,
    ) -> Result<(), StoreError> {
        let error_json = serde_json::to_value(&error_history)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        // Get task details and move to DLQ
        sqlx::query(
            r#"
            INSERT INTO durable_dead_letter_queue (
                original_task_id, workflow_id, activity_id, activity_type,
                input, attempts, last_error, error_history
            )
            SELECT id, workflow_id, activity_id, activity_type,
                   input, attempt, COALESCE(last_error, 'unknown'), $2
            FROM durable_task_queue
            WHERE id = $1
            "#,
        )
        .bind(task_id)
        .bind(&error_json)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to move task to DLQ: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%task_id, "moved task to DLQ");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn requeue_from_dlq(&self, dlq_id: Uuid) -> Result<Uuid, StoreError> {
        let task_id = Uuid::now_v7();

        // Create new task from DLQ entry
        let result = sqlx::query(
            r#"
            WITH dlq_entry AS (
                SELECT workflow_id, activity_id, activity_type, input
                FROM durable_dead_letter_queue
                WHERE id = $1
            )
            INSERT INTO durable_task_queue (
                id, workflow_id, activity_id, activity_type, input, options,
                max_attempts, priority,
                schedule_to_start_timeout_ms, start_to_close_timeout_ms
            )
            SELECT $2, workflow_id, activity_id, activity_type, input,
                   '{}', 3, 0, 60000, 300000
            FROM dlq_entry
            RETURNING id
            "#,
        )
        .bind(dlq_id)
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to requeue from DLQ: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.is_none() {
            return Err(StoreError::TaskNotFound(dlq_id));
        }

        // Update DLQ entry
        sqlx::query(
            r#"
            UPDATE durable_dead_letter_queue
            SET requeued_at = NOW(),
                requeue_count = requeue_count + 1
            WHERE id = $1
            "#,
        )
        .bind(dlq_id)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        debug!(%dlq_id, %task_id, "requeued task from DLQ");
        Ok(task_id)
    }

    #[instrument(skip(self))]
    async fn list_dlq(
        &self,
        filter: DlqFilter,
        pagination: Pagination,
    ) -> Result<Vec<DlqEntry>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, original_task_id, workflow_id, activity_id, activity_type,
                   input, attempts, last_error, error_history, dead_at
            FROM durable_dead_letter_queue
            WHERE ($1::uuid IS NULL OR workflow_id = $1)
              AND ($2::text IS NULL OR activity_type = $2)
            ORDER BY dead_at DESC
            OFFSET $3
            LIMIT $4
            "#,
        )
        .bind(filter.workflow_id)
        .bind(&filter.activity_type)
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to list DLQ: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let entries = rows
            .into_iter()
            .map(|row| {
                let error_history_json: serde_json::Value = row.get("error_history");
                let error_history: Vec<String> =
                    serde_json::from_value(error_history_json).unwrap_or_default();

                DlqEntry {
                    id: row.get("id"),
                    original_task_id: row.get("original_task_id"),
                    workflow_id: row.get("workflow_id"),
                    activity_id: row.get("activity_id"),
                    activity_type: row.get("activity_type"),
                    input: row.get("input"),
                    attempts: row.get::<i32, _>("attempts") as u32,
                    last_error: row.get("last_error"),
                    error_history,
                    dead_at: row.get("dead_at"),
                }
            })
            .collect();

        Ok(entries)
    }
}

// Helper functions

fn parse_workflow_status(status: &str) -> Result<WorkflowStatus, StoreError> {
    match status {
        "pending" => Ok(WorkflowStatus::Pending),
        "running" => Ok(WorkflowStatus::Running),
        "completed" => Ok(WorkflowStatus::Completed),
        "failed" => Ok(WorkflowStatus::Failed),
        "cancelled" => Ok(WorkflowStatus::Cancelled),
        _ => Err(StoreError::Database(format!(
            "Unknown workflow status: {}",
            status
        ))),
    }
}

fn event_type_name(event: &WorkflowEvent) -> &'static str {
    match event {
        WorkflowEvent::WorkflowStarted { .. } => "workflow_started",
        WorkflowEvent::WorkflowCompleted { .. } => "workflow_completed",
        WorkflowEvent::WorkflowFailed { .. } => "workflow_failed",
        WorkflowEvent::WorkflowCancelled { .. } => "workflow_cancelled",
        WorkflowEvent::ActivityScheduled { .. } => "activity_scheduled",
        WorkflowEvent::ActivityStarted { .. } => "activity_started",
        WorkflowEvent::ActivityCompleted { .. } => "activity_completed",
        WorkflowEvent::ActivityFailed { .. } => "activity_failed",
        WorkflowEvent::ActivityTimedOut { .. } => "activity_timed_out",
        WorkflowEvent::ActivityCancelled { .. } => "activity_cancelled",
        WorkflowEvent::TimerStarted { .. } => "timer_started",
        WorkflowEvent::TimerFired { .. } => "timer_fired",
        WorkflowEvent::TimerCancelled { .. } => "timer_cancelled",
        WorkflowEvent::SignalReceived { .. } => "signal_received",
        WorkflowEvent::ChildWorkflowStarted { .. } => "child_workflow_started",
        WorkflowEvent::ChildWorkflowCompleted { .. } => "child_workflow_completed",
        WorkflowEvent::ChildWorkflowFailed { .. } => "child_workflow_failed",
    }
}

#[cfg(test)]
mod tests {
    // Integration tests require a PostgreSQL database
    // Run with: cargo test -p everruns-durable --test integration_test -- --test-threads=1
}
