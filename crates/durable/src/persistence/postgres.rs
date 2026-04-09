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
use tracing::{debug, error, info, instrument};
use uuid::Uuid;

use super::store::{
    CapacitySnapshot, CircuitBreakerState, ClaimedTask, CreateScheduleRow, DeadTaskInfo, DlqEntry,
    DlqFilter, HeartbeatResponse, Pagination, ReclaimResult, ScheduleExecutionFilter,
    ScheduleExecutionRow, ScheduleExecutionStatus, ScheduleFilter, ScheduleRow, ScheduleStats,
    ScheduleTargetType, SchedulerInstanceInfo, StoreError, SystemHealth, TaskDefinition,
    TaskFailureOutcome, TaskFilter, TaskInfo, TaskStatus, TraceContext, UpdateSchedule,
    WORKER_HEARTBEAT_TIMEOUT_SECS, WorkerFilter, WorkerInfo, WorkflowEventInfo, WorkflowEventStore,
    WorkflowFilter, WorkflowInfo, WorkflowInfoExtended, WorkflowStatus,
};
use crate::reliability::{CircuitBreakerConfig, CircuitState};
use crate::workflow::{ActivityOptions, WorkflowError, WorkflowEvent, WorkflowSignal};

#[cfg(feature = "failpoints")]
use fail::fail_point;

/// Recursively strip `\0` (null bytes) from all string values in a JSON tree.
///
/// PostgreSQL `jsonb` rejects `\u0000`; LLM tool output or other external inputs
/// may contain them, so all JSONB writes in this module use this helper before binding.
fn sanitize_json_null_bytes(mut value: serde_json::Value) -> serde_json::Value {
    fn sanitize_in_place(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::String(s) => {
                s.retain(|c| c != '\0');
            }
            serde_json::Value::Array(arr) => {
                for item in arr.iter_mut() {
                    sanitize_in_place(item);
                }
            }
            serde_json::Value::Object(map) => {
                for (_k, val) in map.iter_mut() {
                    sanitize_in_place(val);
                }
            }
            _ => {}
        }
    }

    sanitize_in_place(&mut value);
    value
}

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
    max_pending_tasks_per_workflow: u32,
}

impl PostgresWorkflowEventStore {
    /// Create a new PostgreSQL store with the given connection pool
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            max_pending_tasks_per_workflow: super::store::max_pending_tasks_per_workflow_from_env(),
        }
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

        let input = sanitize_json_null_bytes(input);

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
            SELECT id, workflow_type, status, input, result, error, continued_as_new_id
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
            continued_as_new_id: row.try_get("continued_as_new_id").ok().flatten(),
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

        // Lock the workflow instance row to prevent concurrent modifications
        sqlx::query(
            r#"
            SELECT id FROM durable_workflow_instances
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?
        .ok_or(StoreError::WorkflowNotFound(workflow_id))?;

        // Get current sequence (now safe since we hold the lock)
        let row = sqlx::query(
            r#"
            SELECT COALESCE(MAX(sequence_num) + 1, 0) as next_seq
            FROM durable_workflow_events
            WHERE workflow_id = $1
            "#,
        )
        .bind(workflow_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

        let current_sequence: i32 = row.get::<i32, _>("next_seq");

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
                .map(sanitize_json_null_bytes)
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

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_append_events_after_insert", |_| {
            Err(StoreError::Database("injected: after insert".into()))
        });

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_append_events_before_commit", |_| {
            Err(StoreError::Database("injected: before commit".into()))
        });

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

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_load_events_after_query", |_| {
            Err(StoreError::Database(
                "injected: after load events query".into(),
            ))
        });

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i32 = row.get::<i32, _>("sequence_num");
            let data: serde_json::Value = row.get("event_data");
            let event: WorkflowEvent = serde_json::from_value(data)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            events.push((seq, event));
        }

        Ok(events)
    }

    #[instrument(skip(self))]
    async fn count_events(&self, workflow_id: Uuid) -> Result<usize, StoreError> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM durable_workflow_events
            WHERE workflow_id = $1
            "#,
        )
        .bind(workflow_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to count events: {}", e);
            StoreError::Database(e.to_string())
        })?;

        usize::try_from(count).map_err(|_| {
            StoreError::Database(format!(
                "event count overflow for workflow {workflow_id}: {count}"
            ))
        })
    }

    #[instrument(skip(self))]
    async fn count_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<usize, StoreError> {
        let count = sqlx::query_scalar::<_, i64>(
            r#"
            SELECT COUNT(*)
            FROM durable_workflow_events
            WHERE workflow_id = $1 AND sequence_num > $2
            "#,
        )
        .bind(workflow_id)
        .bind(after_sequence)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!(
                "Failed to count events after sequence {}: {}",
                after_sequence, e
            );
            StoreError::Database(e.to_string())
        })?;

        usize::try_from(count).map_err(|_| {
            StoreError::Database(format!(
                "event count overflow for workflow {workflow_id}: {count}"
            ))
        })
    }

    #[instrument(skip(self))]
    async fn load_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT sequence_num, event_data
            FROM durable_workflow_events
            WHERE workflow_id = $1 AND sequence_num > $2
            ORDER BY sequence_num
            "#,
        )
        .bind(workflow_id)
        .bind(after_sequence)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!(
                "Failed to load events after sequence {}: {}",
                after_sequence, e
            );
            StoreError::Database(e.to_string())
        })?;

        let mut events = Vec::with_capacity(rows.len());
        for row in rows {
            let seq: i32 = row.get::<i32, _>("sequence_num");
            let data: serde_json::Value = row.get("event_data");
            let event: WorkflowEvent = serde_json::from_value(data)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            events.push((seq, event));
        }

        Ok(events)
    }

    #[instrument(skip(self, snapshot_data))]
    async fn save_snapshot(
        &self,
        workflow_id: Uuid,
        sequence_num: i32,
        snapshot_data: Vec<u8>,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO durable_workflow_snapshots (workflow_id, sequence_num, snapshot_data)
            VALUES ($1, $2, $3)
            ON CONFLICT (workflow_id, sequence_num)
            DO UPDATE SET snapshot_data = EXCLUDED.snapshot_data, created_at = NOW()
            "#,
        )
        .bind(workflow_id)
        .bind(sequence_num)
        .bind(&snapshot_data)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to save snapshot: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, sequence_num, "saved workflow snapshot");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn load_latest_snapshot(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<super::store::WorkflowSnapshot>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT sequence_num, snapshot_data, created_at
            FROM durable_workflow_snapshots
            WHERE workflow_id = $1
            ORDER BY sequence_num DESC
            LIMIT 1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to load latest snapshot: {}", e);
            StoreError::Database(e.to_string())
        })?;

        Ok(row.map(|r| super::store::WorkflowSnapshot {
            workflow_id,
            sequence_num: r.get("sequence_num"),
            snapshot_data: r.get("snapshot_data"),
            created_at: r.get("created_at"),
        }))
    }

    #[instrument(skip(self))]
    async fn delete_snapshots(&self, workflow_id: Uuid) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            DELETE FROM durable_workflow_snapshots WHERE workflow_id = $1
            "#,
        )
        .bind(workflow_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to delete snapshots: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, "deleted workflow snapshots");
        Ok(())
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

        // When resetting to Pending (for multi-turn reuse), clear timestamps,
        // result, and error so the workflow starts clean. For other transitions,
        // use COALESCE to preserve existing values.
        let reset_to_pending = matches!(status, WorkflowStatus::Pending);

        let (started_at, completed_at): (Option<DateTime<Utc>>, Option<DateTime<Utc>>) =
            match status {
                WorkflowStatus::Running => (Some(Utc::now()), None),
                WorkflowStatus::Completed
                | WorkflowStatus::Failed
                | WorkflowStatus::Cancelled
                | WorkflowStatus::ContinuedAsNew => (None, Some(Utc::now())),
                WorkflowStatus::Pending => (None, None),
            };

        let query = if reset_to_pending {
            // Clear all transient fields when resetting for a new turn
            r#"
            UPDATE durable_workflow_instances
            SET status = $2,
                result = NULL,
                error = NULL,
                started_at = NULL,
                completed_at = NULL
            WHERE id = $1
            "#
        } else {
            r#"
            UPDATE durable_workflow_instances
            SET status = $2,
                result = COALESCE($3, result),
                error = COALESCE($4, error),
                started_at = COALESCE($5, started_at),
                completed_at = COALESCE($6, completed_at)
            WHERE id = $1
            "#
        };

        let result = result.map(sanitize_json_null_bytes);
        let error_json = error_json.map(sanitize_json_null_bytes);

        sqlx::query(query)
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
        // Check pending task limits: per-workflow for workflow tasks, global for standalone
        if let Some(wf_id) = task.workflow_id {
            let limit = self.max_pending_tasks_per_workflow;
            let pending_count: i64 = sqlx::query_scalar(
                r#"
                SELECT COUNT(*) FROM durable_task_queue
                WHERE workflow_id = $1 AND status = 'pending'
                "#,
            )
            .bind(wf_id)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                error!("Failed to count pending tasks: {}", e);
                StoreError::Database(e.to_string())
            })?;

            if pending_count >= limit as i64 {
                return Err(StoreError::TaskQueueLimitExceeded {
                    workflow_id: wf_id,
                    current: pending_count as u32,
                    limit,
                });
            }
        } else {
            // Standalone task: check global standalone limit
            let limit = super::store::DEFAULT_MAX_PENDING_STANDALONE_TASKS;
            let pending_count: i64 = sqlx::query_scalar(
                r#"
                SELECT COUNT(*) FROM durable_task_queue
                WHERE workflow_id IS NULL AND status = 'pending'
                "#,
            )
            .fetch_one(&self.pool)
            .await
            .map_err(|e| {
                error!("Failed to count standalone pending tasks: {}", e);
                StoreError::Database(e.to_string())
            })?;

            if pending_count >= limit as i64 {
                return Err(StoreError::StandaloneTaskQueueLimitExceeded {
                    current: pending_count as u32,
                    limit,
                });
            }
        }

        let task_id = Uuid::now_v7();
        let task_input = sanitize_json_null_bytes(task.input.clone());
        let options_json = serde_json::to_value(&task.options)
            .map(sanitize_json_null_bytes)
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
        .bind(&task_input)
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

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_enqueue_task_after_insert", |_| {
            Err(StoreError::Database(
                "injected: after enqueue insert".into(),
            ))
        });

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
        // 1. Verifies worker is not draining (early exit if draining)
        // 2. Finds pending tasks matching activity types
        // 3. Orders by priority (desc) then visibility time
        // 4. Limits to max_tasks
        // 5. Uses SKIP LOCKED to avoid contention
        // 6. Updates status and claiming info in one atomic operation
        // NOTE: The `attempt < max_attempts` check is critical for preventing infinite
        // retries when workers panic. Without fail_task being called, the task becomes
        // stale, gets reclaimed, but must not be claimed if attempts are exhausted.
        let rows = sqlx::query(
            r#"
            WITH worker_check AS (
                SELECT 1 FROM durable_workers
                WHERE id = $3 AND status != 'draining'
            ),
            claimable AS (
                SELECT tq.id
                FROM durable_task_queue tq, worker_check
                WHERE tq.status = 'pending'
                  AND tq.activity_type = ANY($1)
                  AND tq.visible_at <= NOW()
                  AND tq.attempt < tq.max_attempts
                ORDER BY tq.priority DESC, tq.visible_at
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

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_claim_task_after_query", |_| {
            Err(StoreError::Database("injected: after claim query".into()))
        });

        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let options_json: serde_json::Value = row.get("options");
            let options: ActivityOptions = serde_json::from_value(options_json)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;

            claimed.push(ClaimedTask {
                id: row.get("id"),
                workflow_id: row.get::<Option<Uuid>, _>("workflow_id"),
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

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_heartbeat_update", |_| {
            Err(StoreError::Database("injected: heartbeat update".into()))
        });

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
        worker_id: &str,
        _result: serde_json::Value,
    ) -> Result<(), StoreError> {
        // Only complete if:
        // 1. Task is still claimed by this worker, OR
        // 2. Task is still claimed (status = 'claimed') by this worker
        // This prevents duplicate scheduling when a task is reclaimed due to heartbeat timeout
        let result = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'completed'
            WHERE id = $1 AND claimed_by = $2 AND status = 'claimed'
            RETURNING id
            "#,
        )
        .bind(task_id)
        .bind(worker_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to complete task: {}", e);
            StoreError::Database(e.to_string())
        })?;

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_complete_task_after_update", |_| {
            Err(StoreError::Database("injected: complete task".into()))
        });

        match result {
            Some(_) => {
                debug!(%task_id, %worker_id, "completed task");
                Ok(())
            }
            None => {
                // Task was reclaimed by another worker or already completed
                debug!(%task_id, %worker_id, "task not owned - was reclaimed or already completed");
                Err(StoreError::TaskNotOwned(task_id))
            }
        }
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
    async fn try_claim_workflow_for_new_turn(&self, workflow_id: Uuid) -> Result<bool, StoreError> {
        // Atomic CAS: transition from terminal/pending → running.
        // Also cancels stale pending tasks in the same transaction.
        // Uses SELECT FOR UPDATE to prevent concurrent claims across replicas.
        let mut tx = self.pool.begin().await.map_err(|e| {
            error!("Failed to begin transaction: {}", e);
            StoreError::Database(e.to_string())
        })?;

        // Lock the workflow row and check status
        let row = sqlx::query(
            r#"
            SELECT status FROM durable_workflow_instances
            WHERE id = $1
            FOR UPDATE
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            error!("Failed to lock workflow for claim: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let Some(row) = row else {
            tx.rollback().await.ok();
            return Err(StoreError::WorkflowNotFound(workflow_id));
        };

        let status: String = row.get("status");
        if status == "running" {
            // Active turn — cannot claim, caller should send steering signal
            tx.rollback().await.ok();
            return Ok(false);
        }

        // Claim: set to running, clear transient fields
        sqlx::query(
            r#"
            UPDATE durable_workflow_instances
            SET status = 'running',
                result = NULL,
                error = NULL,
                started_at = NOW(),
                completed_at = NULL
            WHERE id = $1
            "#,
        )
        .bind(workflow_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!("Failed to update workflow to running: {}", e);
            StoreError::Database(e.to_string())
        })?;

        // Cancel any stale pending tasks from previous turn
        sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'cancelled'
            WHERE workflow_id = $1 AND status = 'pending'
            "#,
        )
        .bind(workflow_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!("Failed to cancel stale tasks during claim: {}", e);
            StoreError::Database(e.to_string())
        })?;

        tx.commit().await.map_err(|e| {
            error!("Failed to commit workflow claim: {}", e);
            StoreError::Database(e.to_string())
        })?;

        Ok(true)
    }

    #[instrument(skip(self))]
    async fn cancel_pending_tasks_for_workflow(
        &self,
        workflow_id: Uuid,
    ) -> Result<u64, StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'cancelled'
            WHERE workflow_id = $1 AND status = 'pending'
            "#,
        )
        .bind(workflow_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to cancel pending tasks for workflow: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let count = result.rows_affected();
        if count > 0 {
            debug!(%workflow_id, count, "cancelled pending tasks for workflow");
        }
        Ok(count)
    }

    #[instrument(skip(self))]
    async fn get_task(&self, task_id: Uuid) -> Result<TaskInfo, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, workflow_id, activity_id, activity_type, status,
                   priority, attempt, max_attempts, claimed_by, last_error,
                   created_at, claimed_at
            FROM durable_task_queue
            WHERE id = $1
            "#,
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get task: {}", e);
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::TaskNotFound(task_id))?;

        let status_str: String = row.get("status");
        let status = match status_str.as_str() {
            "pending" => TaskStatus::Pending,
            "claimed" => TaskStatus::Claimed,
            "completed" => TaskStatus::Completed,
            "failed" => TaskStatus::Failed,
            "dead" => TaskStatus::Dead,
            "cancelled" => TaskStatus::Cancelled,
            _ => TaskStatus::Pending,
        };

        Ok(TaskInfo {
            id: row.get("id"),
            workflow_id: row.get::<Option<Uuid>, _>("workflow_id"),
            activity_id: row.get("activity_id"),
            activity_type: row.get("activity_type"),
            status,
            priority: row.get("priority"),
            attempt: row.get::<i32, _>("attempt") as u32,
            max_attempts: row.get::<i32, _>("max_attempts") as u32,
            claimed_by: row.get("claimed_by"),
            last_error: row.get("last_error"),
            created_at: row.get("created_at"),
            claimed_at: row.get("claimed_at"),
        })
    }

    #[instrument(skip(self))]
    async fn reclaim_stale_tasks(
        &self,
        stale_threshold: Duration,
    ) -> Result<ReclaimResult, StoreError> {
        let threshold =
            Utc::now() - chrono::Duration::from_std(stale_threshold).unwrap_or_default();

        // First, mark exhausted tasks as dead (they've used all attempts via stale reclaims)
        // This handles the case where workers panic without calling fail_task.
        let dead_rows = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'dead',
                last_error = COALESCE(last_error, 'Worker became unresponsive after exhausting all retry attempts')
            WHERE status = 'claimed'
              AND heartbeat_at < $1
              AND attempt >= max_attempts
            RETURNING id, workflow_id, activity_id, last_error
            "#,
        )
        .bind(threshold)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to mark exhausted stale tasks as dead: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let dead_tasks: Vec<DeadTaskInfo> = dead_rows
            .iter()
            .map(|r| DeadTaskInfo {
                task_id: r.get("id"),
                workflow_id: r.get("workflow_id"),
                activity_id: r.get("activity_id"),
                last_error: r.get("last_error"),
            })
            .collect();

        if !dead_tasks.is_empty() {
            info!(
                count = dead_tasks.len(),
                "marked exhausted stale tasks as dead"
            );
        }

        // Then, reclaim stale tasks that still have attempts remaining
        let rows = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'pending',
                claimed_by = NULL,
                claimed_at = NULL
            WHERE status = 'claimed'
              AND heartbeat_at < $1
              AND attempt < max_attempts
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

        let reclaimed_ids: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();

        #[cfg(feature = "failpoints")]
        fail_point!("postgres_reclaim_stale_after_update", |_| {
            Err(StoreError::Database(
                "injected: after reclaim update".into(),
            ))
        });

        if !reclaimed_ids.is_empty() || !dead_tasks.is_empty() {
            debug!(
                reclaimed = reclaimed_ids.len(),
                dead = dead_tasks.len(),
                "processed stale tasks"
            );
        }

        // Also mark workers with stale heartbeats as stopped.
        // This cleans up workers that crashed without calling deregister_worker.
        let worker_heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);
        let stale_workers = sqlx::query(
            r#"
            UPDATE durable_workers
            SET status = 'stopped'
            WHERE status = 'active'
              AND last_heartbeat_at < $1
            RETURNING id
            "#,
        )
        .bind(worker_heartbeat_threshold)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to mark stale workers as stopped: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if !stale_workers.is_empty() {
            let ids: Vec<String> = stale_workers.iter().map(|r| r.get("id")).collect();
            info!(count = stale_workers.len(), worker_ids = ?ids, "marked stale workers as stopped");
        }

        Ok(ReclaimResult {
            reclaimed_ids,
            dead_tasks,
        })
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

    #[instrument(skip(self))]
    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        // Atomic: UPDATE ... RETURNING in a single statement.
        // Marks all pending signals as processed and returns them.
        let rows = sqlx::query(
            r#"
            UPDATE durable_signals
            SET processed_at = NOW()
            WHERE id IN (
                SELECT id FROM durable_signals
                WHERE workflow_id = $1 AND processed_at IS NULL
                ORDER BY sequence_num
                FOR UPDATE
            )
            RETURNING signal_type, payload, sent_at
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to consume pending signals: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let signals = rows
            .into_iter()
            .map(|row| WorkflowSignal {
                signal_type: row.get("signal_type"),
                payload: row.get("payload"),
                sent_at: row.get("sent_at"),
            })
            .collect::<Vec<_>>();

        debug!(%workflow_id, count = signals.len(), "consumed pending signals atomically");
        Ok(signals)
    }

    #[instrument(skip(self, worker))]
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        // Default worker_group to "default" if not specified
        let worker_group = worker.worker_group.as_deref().unwrap_or("default");

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
        .bind(worker_group)
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
        // Only update accepting_tasks if worker is NOT draining.
        // When draining, we preserve accepting_tasks = false set by drain_worker.
        sqlx::query(
            r#"
            UPDATE durable_workers
            SET last_heartbeat_at = NOW(),
                current_load = $2,
                accepting_tasks = CASE
                    WHEN status = 'draining' THEN false
                    ELSE $3
                END
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
        // Only show workers with recent heartbeat (within WORKER_HEARTBEAT_TIMEOUT_SECS)
        // Join with task queue to compute completed/failed counts
        let heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);

        let query = r#"
            SELECT w.id, w.worker_group, w.activity_types, w.max_concurrency, w.current_load,
                   w.status, w.started_at, w.last_heartbeat_at, w.accepting_tasks,
                   w.backpressure_reason, w.hostname, w.version, w.metadata,
                   COALESCE(stats.tasks_completed, 0) AS tasks_completed,
                   COALESCE(stats.tasks_failed, 0) AS tasks_failed,
                   stats.avg_task_duration_ms
            FROM durable_workers w
            LEFT JOIN (
                SELECT claimed_by,
                       COUNT(*) FILTER (WHERE status = 'completed') AS tasks_completed,
                       COUNT(*) FILTER (WHERE status IN ('failed', 'dead')) AS tasks_failed,
                       (AVG(EXTRACT(EPOCH FROM (heartbeat_at - claimed_at)) * 1000)
                           FILTER (WHERE status = 'completed' AND claimed_at IS NOT NULL AND heartbeat_at IS NOT NULL))::FLOAT8
                           AS avg_task_duration_ms
                FROM durable_task_queue
                WHERE claimed_by IS NOT NULL
                GROUP BY claimed_by
            ) stats ON stats.claimed_by = w.id
            WHERE w.last_heartbeat_at > $1
              AND ($2::text IS NULL OR w.status = $2)
              AND ($3::text IS NULL OR w.worker_group = $3)
            "#;

        let rows = sqlx::query(query)
            .bind(heartbeat_threshold)
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
                backpressure_reason: row.get("backpressure_reason"),
                started_at: row.get("started_at"),
                last_heartbeat_at: row.get("last_heartbeat_at"),
                hostname: row.get("hostname"),
                version: row.get("version"),
                metadata: row.get("metadata"),
                tasks_completed: row.get::<i64, _>("tasks_completed") as u64,
                tasks_failed: row.get::<i64, _>("tasks_failed") as u64,
                avg_task_duration_ms: row
                    .get::<Option<f64>, _>("avg_task_duration_ms")
                    .map(|v| v as u64),
            })
            .collect();

        Ok(workers)
    }

    #[instrument(skip(self))]
    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        // Reclaim all tasks claimed by this worker (set back to pending)
        // This allows immediate task reassignment instead of waiting for heartbeat timeout
        let reclaimed = sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'pending',
                claimed_by = NULL,
                claimed_at = NULL
            WHERE status = 'claimed'
              AND claimed_by = $1
            "#,
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to reclaim worker tasks: {}", e);
            StoreError::Database(e.to_string())
        })?
        .rows_affected() as usize;

        // Mark worker as stopped
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

        if reclaimed > 0 {
            info!(
                worker_id,
                reclaimed, "deregistered worker and reclaimed tasks"
            );
        } else {
            debug!(worker_id, "deregistered worker (no tasks to reclaim)");
        }

        Ok(reclaimed)
    }

    #[instrument(skip(self))]
    async fn get_capacity_snapshot(&self) -> Result<CapacitySnapshot, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT
                COALESCE(SUM(max_concurrency - current_load), 0)::INT AS total_available,
                COUNT(*)::INT AS active_workers
            FROM durable_workers
            WHERE status = 'active'
              AND accepting_tasks = true
              AND last_heartbeat_at > NOW() - INTERVAL '30 seconds'
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get capacity snapshot: {}", e);
            StoreError::Database(e.to_string())
        })?;

        Ok(CapacitySnapshot {
            total_available: row.get::<i32, _>("total_available") as u32,
            active_workers: row.get::<i32, _>("active_workers") as u32,
        })
    }

    #[instrument(skip(self, error_history))]
    async fn move_to_dlq(
        &self,
        task_id: Uuid,
        error_history: Vec<String>,
    ) -> Result<(), StoreError> {
        let error_json = serde_json::to_value(&error_history)
            .map(sanitize_json_null_bytes)
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
        let default_options = serde_json::to_value(ActivityOptions::default())
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

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
                   $3, 3, 0, 60000, 300000
            FROM dlq_entry
            RETURNING id
            "#,
        )
        .bind(dlq_id)
        .bind(task_id)
        .bind(&default_options)
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
                    workflow_id: row.get::<Option<Uuid>, _>("workflow_id"),
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

    // =========================================================================
    // Circuit Breaker Operations
    // =========================================================================

    #[instrument(skip(self, config))]
    async fn create_circuit_breaker(
        &self,
        key: &str,
        config: &CircuitBreakerConfig,
    ) -> Result<(), StoreError> {
        // Use INSERT ... ON CONFLICT DO NOTHING to handle race conditions
        // when multiple workers try to create the same circuit breaker
        sqlx::query(
            r#"
            INSERT INTO durable_circuit_breaker_state (key, state, failure_count, success_count, updated_at)
            VALUES ($1, 'closed', 0, 0, NOW())
            ON CONFLICT (key) DO NOTHING
            "#,
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to create circuit breaker: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(
            key,
            failure_threshold = config.failure_threshold,
            "created circuit breaker"
        );
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_circuit_breaker(
        &self,
        key: &str,
    ) -> Result<Option<CircuitBreakerState>, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT key, state, failure_count, success_count,
                   last_failure_at, opened_at, half_open_at, updated_at
            FROM durable_circuit_breaker_state
            WHERE key = $1
            "#,
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get circuit breaker: {}", e);
            StoreError::Database(e.to_string())
        })?;

        match row {
            Some(row) => {
                let state_str: String = row.get("state");
                let state = parse_circuit_state(&state_str)?;

                Ok(Some(CircuitBreakerState {
                    key: row.get("key"),
                    state,
                    failure_count: row.get::<i32, _>("failure_count") as u32,
                    success_count: row.get::<i32, _>("success_count") as u32,
                    last_failure_at: row.get("last_failure_at"),
                    opened_at: row.get("opened_at"),
                    half_open_at: row.get("half_open_at"),
                    updated_at: row.get("updated_at"),
                }))
            }
            None => Ok(None),
        }
    }

    #[instrument(skip(self))]
    async fn update_circuit_breaker(
        &self,
        key: &str,
        state: CircuitState,
        failure_count: u32,
        success_count: u32,
    ) -> Result<(), StoreError> {
        let state_str = state.to_string();

        // Build the query dynamically based on state transitions
        let (opened_at_update, half_open_at_update, last_failure_update) = match state {
            CircuitState::Open => (
                "NOW()",
                "NULL",
                if failure_count > 0 {
                    "NOW()"
                } else {
                    "last_failure_at"
                },
            ),
            CircuitState::HalfOpen => ("opened_at", "NOW()", "last_failure_at"),
            CircuitState::Closed => ("NULL", "NULL", "NULL"),
        };

        let query = format!(
            r#"
            UPDATE durable_circuit_breaker_state
            SET state = $2,
                failure_count = $3,
                success_count = $4,
                opened_at = {},
                half_open_at = {},
                last_failure_at = {},
                updated_at = NOW()
            WHERE key = $1
            "#,
            opened_at_update, half_open_at_update, last_failure_update
        );

        sqlx::query(&query)
            .bind(key)
            .bind(&state_str)
            .bind(failure_count as i32)
            .bind(success_count as i32)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!("Failed to update circuit breaker: {}", e);
                StoreError::Database(e.to_string())
            })?;

        debug!(key, %state_str, failure_count, success_count, "updated circuit breaker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn count_active_workflows(&self) -> Result<i64, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count
            FROM durable_workflow_instances
            WHERE status IN ('pending', 'running')
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to count active workflows: {}", e);
            StoreError::Database(e.to_string())
        })?;

        Ok(row.get("count"))
    }

    #[instrument(skip(self))]
    async fn list_workflows(
        &self,
        filter: WorkflowFilter,
        pagination: Pagination,
    ) -> Result<Vec<WorkflowInfoExtended>, StoreError> {
        let status_str = filter.status.map(|s| s.to_string());

        let rows = sqlx::query(
            r#"
            SELECT id, workflow_type, status, input, result, error,
                   created_at, started_at, completed_at, continued_as_new_id
            FROM durable_workflow_instances
            WHERE ($1::text IS NULL OR status = $1)
              AND ($2::text IS NULL OR workflow_type = $2)
            ORDER BY created_at DESC
            OFFSET $3
            LIMIT $4
            "#,
        )
        .bind(&status_str)
        .bind(&filter.workflow_type)
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to list workflows: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let mut workflows = Vec::with_capacity(rows.len());
        for row in rows {
            let status_str: String = row.get("status");
            let error_json: Option<serde_json::Value> = row.get("error");

            workflows.push(WorkflowInfoExtended {
                id: row.get("id"),
                workflow_type: row.get("workflow_type"),
                status: parse_workflow_status(&status_str)?,
                input: row.get("input"),
                result: row.get("result"),
                error: error_json.and_then(|v| serde_json::from_value(v).ok()),
                created_at: row.get("created_at"),
                started_at: row.get("started_at"),
                completed_at: row.get("completed_at"),
                continued_as_new_id: row.try_get("continued_as_new_id").ok().flatten(),
            });
        }

        Ok(workflows)
    }

    #[instrument(skip(self))]
    async fn list_tasks(
        &self,
        filter: TaskFilter,
        pagination: Pagination,
    ) -> Result<Vec<TaskInfo>, StoreError> {
        let status_str = filter.status.map(|s| match s {
            TaskStatus::Pending => "pending",
            TaskStatus::Claimed => "claimed",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Dead => "dead",
            TaskStatus::Cancelled => "cancelled",
        });

        let rows = sqlx::query(
            r#"
            SELECT id, workflow_id, activity_id, activity_type, status,
                   priority, attempt, max_attempts, claimed_by, last_error,
                   created_at, claimed_at
            FROM durable_task_queue
            WHERE ($1::text IS NULL OR status = $1)
              AND ($2::text IS NULL OR activity_type = $2)
              AND ($3::uuid IS NULL OR workflow_id = $3)
              AND (NOT $6 OR workflow_id IS NULL)
            ORDER BY created_at ASC
            OFFSET $4
            LIMIT $5
            "#,
        )
        .bind(status_str)
        .bind(&filter.activity_type)
        .bind(filter.workflow_id)
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .bind(filter.standalone_only)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to list tasks: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let tasks = rows
            .into_iter()
            .map(|row| {
                let status_str: String = row.get("status");
                let status = match status_str.as_str() {
                    "pending" => TaskStatus::Pending,
                    "claimed" => TaskStatus::Claimed,
                    "completed" => TaskStatus::Completed,
                    "failed" => TaskStatus::Failed,
                    "dead" => TaskStatus::Dead,
                    "cancelled" => TaskStatus::Cancelled,
                    _ => TaskStatus::Pending,
                };

                TaskInfo {
                    id: row.get("id"),
                    workflow_id: row.get::<Option<Uuid>, _>("workflow_id"),
                    activity_id: row.get("activity_id"),
                    activity_type: row.get("activity_type"),
                    status,
                    priority: row.get("priority"),
                    attempt: row.get::<i32, _>("attempt") as u32,
                    max_attempts: row.get::<i32, _>("max_attempts") as u32,
                    claimed_by: row.get("claimed_by"),
                    last_error: row.get("last_error"),
                    created_at: row.get("created_at"),
                    claimed_at: row.get("claimed_at"),
                }
            })
            .collect();

        Ok(tasks)
    }

    #[instrument(skip(self))]
    async fn list_circuit_breakers(&self) -> Result<Vec<CircuitBreakerState>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT key, state, failure_count, success_count,
                   last_failure_at, opened_at, half_open_at, updated_at
            FROM durable_circuit_breaker_state
            ORDER BY key
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to list circuit breakers: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let mut breakers = Vec::with_capacity(rows.len());
        for row in rows {
            let state_str: String = row.get("state");
            breakers.push(CircuitBreakerState {
                key: row.get("key"),
                state: parse_circuit_state(&state_str)?,
                failure_count: row.get::<i32, _>("failure_count") as u32,
                success_count: row.get::<i32, _>("success_count") as u32,
                last_failure_at: row.get("last_failure_at"),
                opened_at: row.get("opened_at"),
                half_open_at: row.get("half_open_at"),
                updated_at: row.get("updated_at"),
            });
        }

        Ok(breakers)
    }

    #[instrument(skip(self))]
    async fn force_open_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_circuit_breaker_state
            SET state = 'open',
                failure_count = 0,
                success_count = 0,
                opened_at = NOW(),
                updated_at = NOW()
            WHERE key = $1
            "#,
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to force open circuit breaker: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            // Circuit breaker doesn't exist, create it in open state
            sqlx::query(
                r#"
                INSERT INTO durable_circuit_breaker_state
                    (key, state, failure_count, success_count, opened_at, updated_at)
                VALUES ($1, 'open', 0, 0, NOW(), NOW())
                "#,
            )
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!("Failed to create circuit breaker in open state: {}", e);
                StoreError::Database(e.to_string())
            })?;
        }

        info!(key, "force opened circuit breaker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn force_close_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_circuit_breaker_state
            SET state = 'closed',
                failure_count = 0,
                success_count = 0,
                opened_at = NULL,
                half_open_at = NULL,
                updated_at = NOW()
            WHERE key = $1
            "#,
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to force close circuit breaker: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::CircuitBreakerNotFound(key.to_string()));
        }

        info!(key, "force closed circuit breaker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn delete_circuit_breaker(&self, key: &str) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            DELETE FROM durable_circuit_breaker_state
            WHERE key = $1
            "#,
        )
        .bind(key)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to delete circuit breaker: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::CircuitBreakerNotFound(key.to_string()));
        }

        info!(key, "deleted circuit breaker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn drain_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_workers
            SET status = 'draining',
                accepting_tasks = false
            WHERE id = $1
            "#,
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to drain worker: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(worker_id, "drained worker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn resume_worker(&self, worker_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_workers
            SET status = 'active',
                accepting_tasks = true
            WHERE id = $1 AND status = 'draining'
            "#,
        )
        .bind(worker_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to resume worker: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(worker_id, "resumed worker");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_system_health(&self) -> Result<SystemHealth, StoreError> {
        // Single query to get all health stats.
        // $1 = heartbeat threshold: only workers with heartbeat after this are "active".
        let heartbeat_threshold =
            Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);

        let row = sqlx::query(
            r#"
            SELECT
                (SELECT COUNT(*) FROM durable_workers) as total_workers,
                (SELECT COUNT(*) FROM durable_workers WHERE status = 'active' AND last_heartbeat_at > $1) as active_workers,
                (SELECT COUNT(*) FROM durable_workers WHERE status = 'active' AND accepting_tasks = true AND last_heartbeat_at > $1) as workers_accepting,
                (SELECT COALESCE(SUM(max_concurrency), 0) FROM durable_workers WHERE status = 'active' AND last_heartbeat_at > $1) as total_capacity,
                (SELECT COALESCE(SUM(current_load), 0) FROM durable_workers WHERE status = 'active' AND last_heartbeat_at > $1) as current_load,
                (SELECT COUNT(*) FROM durable_task_queue WHERE status = 'pending') as pending_tasks,
                (SELECT COUNT(*) FROM durable_task_queue WHERE status = 'claimed') as claimed_tasks,
                (SELECT COUNT(*) FROM durable_task_queue WHERE status = 'completed') as completed_tasks,
                (SELECT COUNT(*) FROM durable_task_queue WHERE status IN ('failed', 'dead')) as failed_tasks,
                (SELECT COUNT(*) FROM durable_task_queue WHERE claimed_at IS NOT NULL) as started_tasks,
                (SELECT COUNT(*) FROM durable_workflow_instances WHERE status = 'running') as running_workflows,
                (SELECT COUNT(*) FROM durable_workflow_instances WHERE status = 'pending') as pending_workflows,
                (SELECT COUNT(*) FROM durable_workflow_instances WHERE status = 'completed') as completed_workflows,
                (SELECT COUNT(*) FROM durable_workflow_instances WHERE status IN ('failed', 'cancelled')) as failed_workflows,
                (SELECT COUNT(*) FROM durable_workflow_instances WHERE started_at IS NOT NULL) as started_workflows,
                (SELECT COUNT(*) FROM durable_dead_letter_queue WHERE requeued_at IS NULL) as dlq_size
            "#,
        )
        .bind(heartbeat_threshold)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get system health: {}", e);
            StoreError::Database(e.to_string())
        })?;

        Ok(SystemHealth {
            total_workers: row.get::<i64, _>("total_workers") as usize,
            active_workers: row.get::<i64, _>("active_workers") as usize,
            workers_accepting: row.get::<i64, _>("workers_accepting") as usize,
            total_capacity: row.get::<i64, _>("total_capacity") as usize,
            current_load: row.get::<i64, _>("current_load") as usize,
            pending_tasks: row.get::<i64, _>("pending_tasks") as usize,
            claimed_tasks: row.get::<i64, _>("claimed_tasks") as usize,
            completed_tasks: row.get::<i64, _>("completed_tasks") as usize,
            failed_tasks: row.get::<i64, _>("failed_tasks") as usize,
            started_tasks: row.get::<i64, _>("started_tasks") as usize,
            running_workflows: row.get::<i64, _>("running_workflows") as usize,
            pending_workflows: row.get::<i64, _>("pending_workflows") as usize,
            completed_workflows: row.get::<i64, _>("completed_workflows") as usize,
            failed_workflows: row.get::<i64, _>("failed_workflows") as usize,
            started_workflows: row.get::<i64, _>("started_workflows") as usize,
            dlq_size: row.get::<i64, _>("dlq_size") as usize,
        })
    }

    #[instrument(skip(self))]
    async fn cancel_workflow(&self, workflow_id: Uuid) -> Result<(), StoreError> {
        // Update workflow status to cancelled
        let result = sqlx::query(
            r#"
            UPDATE durable_workflow_instances
            SET status = 'cancelled',
                completed_at = NOW()
            WHERE id = $1 AND status IN ('pending', 'running')
            RETURNING id
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to cancel workflow: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.is_none() {
            return Err(StoreError::WorkflowNotFound(workflow_id));
        }

        // Cancel any pending tasks for this workflow
        sqlx::query(
            r#"
            UPDATE durable_task_queue
            SET status = 'cancelled'
            WHERE workflow_id = $1 AND status = 'pending'
            "#,
        )
        .bind(workflow_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to cancel workflow tasks: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%workflow_id, "cancelled workflow");
        Ok(())
    }

    #[instrument(skip(self, snapshot_data))]
    async fn continue_as_new(
        &self,
        old_workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        snapshot_data: Vec<u8>,
    ) -> Result<Uuid, StoreError> {
        let new_workflow_id = Uuid::now_v7();
        let input = sanitize_json_null_bytes(input);
        let start_event = WorkflowEvent::WorkflowStarted {
            input: input.clone(),
        };
        let event_data = serde_json::to_value(&start_event)
            .map(sanitize_json_null_bytes)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        let mut tx = self.pool.begin().await.map_err(|e| {
            error!("Failed to begin transaction: {}", e);
            StoreError::Database(e.to_string())
        })?;

        // 1. Mark old workflow as continued_as_new
        let result = sqlx::query(
            r#"
            UPDATE durable_workflow_instances
            SET status = 'continued_as_new',
                completed_at = NOW(),
                continued_as_new_id = $2
            WHERE id = $1 AND status IN ('pending', 'running')
            RETURNING id
            "#,
        )
        .bind(old_workflow_id)
        .bind(new_workflow_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| {
            error!("Failed to mark workflow as continued: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.is_none() {
            return Err(StoreError::WorkflowNotFound(old_workflow_id));
        }

        // 2. Delete old workflow events (archive)
        sqlx::query(r#"DELETE FROM durable_workflow_events WHERE workflow_id = $1"#)
            .bind(old_workflow_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!("Failed to delete old events: {}", e);
                StoreError::Database(e.to_string())
            })?;

        // 3. Delete old snapshots
        sqlx::query(r#"DELETE FROM durable_workflow_snapshots WHERE workflow_id = $1"#)
            .bind(old_workflow_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                error!("Failed to delete old snapshots: {}", e);
                StoreError::Database(e.to_string())
            })?;

        // 4. Create new workflow
        sqlx::query(
            r#"
            INSERT INTO durable_workflow_instances (id, workflow_type, status, input, started_at)
            VALUES ($1, $2, 'running', $3, NOW())
            "#,
        )
        .bind(new_workflow_id)
        .bind(workflow_type)
        .bind(&input)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!("Failed to create new workflow: {}", e);
            StoreError::Database(e.to_string())
        })?;

        // 5. Write WorkflowStarted event on new workflow
        sqlx::query(
            r#"
            INSERT INTO durable_workflow_events (workflow_id, sequence_num, event_type, event_data)
            VALUES ($1, 0, 'workflow_started', $2)
            "#,
        )
        .bind(new_workflow_id)
        .bind(&event_data)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!("Failed to write start event: {}", e);
            StoreError::Database(e.to_string())
        })?;

        // 6. Save snapshot on new workflow at sequence 0
        sqlx::query(
            r#"
            INSERT INTO durable_workflow_snapshots (workflow_id, sequence_num, snapshot_data)
            VALUES ($1, 0, $2)
            "#,
        )
        .bind(new_workflow_id)
        .bind(&snapshot_data)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            error!("Failed to save snapshot: {}", e);
            StoreError::Database(e.to_string())
        })?;

        tx.commit().await.map_err(|e| {
            error!("Failed to commit continue_as_new: {}", e);
            StoreError::Database(e.to_string())
        })?;

        info!(
            %old_workflow_id,
            %new_workflow_id,
            "continued workflow as new"
        );

        Ok(new_workflow_id)
    }

    #[instrument(skip(self))]
    async fn get_workflow_events(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowEventInfo>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, workflow_id, sequence_num, event_type, event_data, created_at
            FROM durable_workflow_events
            WHERE workflow_id = $1
            ORDER BY sequence_num
            "#,
        )
        .bind(workflow_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get workflow events: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let events = rows
            .into_iter()
            .map(|row| WorkflowEventInfo {
                id: row.get("id"),
                workflow_id: row.get("workflow_id"),
                sequence_num: row.get("sequence_num"),
                event_type: row.get("event_type"),
                event_data: row.get("event_data"),
                created_at: row.get("created_at"),
            })
            .collect();

        Ok(events)
    }

    #[instrument(skip(self))]
    async fn count_workflow_events(&self, workflow_id: Uuid) -> Result<i64, StoreError> {
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM durable_workflow_events WHERE workflow_id = $1")
                .bind(workflow_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| {
                    error!("Failed to count workflow events: {}", e);
                    StoreError::Database(e.to_string())
                })?;

        Ok(count)
    }

    // =========================================================================
    // Schedule Operations
    // =========================================================================

    #[instrument(skip(self, schedule))]
    async fn create_schedule(&self, schedule: CreateScheduleRow) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();

        sqlx::query(
            r#"
            INSERT INTO durable_schedules (
                id, name, description, cron_expression, timezone,
                target_type, target_name, target_input, enabled,
                max_concurrent, catch_up_missed, max_catch_up, retry_policy,
                next_trigger_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            "#,
        )
        .bind(id)
        .bind(&schedule.name)
        .bind(&schedule.description)
        .bind(&schedule.cron_expression)
        .bind(&schedule.timezone)
        .bind(schedule.target_type.to_string())
        .bind(&schedule.target_name)
        .bind(&schedule.target_input)
        .bind(schedule.enabled)
        .bind(schedule.max_concurrent.map(|v| v as i32))
        .bind(schedule.catch_up_missed)
        .bind(schedule.max_catch_up.map(|v| v as i32))
        .bind(&schedule.retry_policy)
        .bind(schedule.next_trigger_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to create schedule: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%id, name = %schedule.name, "created schedule");
        Ok(id)
    }

    #[instrument(skip(self))]
    async fn get_schedule(&self, id: Uuid) -> Result<ScheduleRow, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, name, description, cron_expression, timezone,
                   target_type, target_name, target_input, enabled,
                   max_concurrent, catch_up_missed, max_catch_up, retry_policy,
                   last_triggered_at, next_trigger_at, claimed_by, claimed_at,
                   created_at, updated_at
            FROM durable_schedules
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get schedule: {}", e);
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::ScheduleNotFound(id))?;

        parse_schedule_row(row)
    }

    #[instrument(skip(self))]
    async fn list_schedules(
        &self,
        filter: ScheduleFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, name, description, cron_expression, timezone,
                   target_type, target_name, target_input, enabled,
                   max_concurrent, catch_up_missed, max_catch_up, retry_policy,
                   last_triggered_at, next_trigger_at, claimed_by, claimed_at,
                   created_at, updated_at
            FROM durable_schedules
            WHERE ($1::boolean IS NULL OR enabled = $1)
              AND ($2::text IS NULL OR target_type = $2)
            ORDER BY created_at DESC
            OFFSET $3 LIMIT $4
            "#,
        )
        .bind(filter.enabled)
        .bind(
            filter
                .target_type
                .map(|t: ScheduleTargetType| t.to_string()),
        )
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to list schedules: {}", e);
            StoreError::Database(e.to_string())
        })?;

        rows.into_iter().map(parse_schedule_row).collect()
    }

    #[instrument(skip(self))]
    async fn count_schedules(&self, filter: ScheduleFilter) -> Result<u64, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count
            FROM durable_schedules
            WHERE ($1::boolean IS NULL OR enabled = $1)
              AND ($2::text IS NULL OR target_type = $2)
            "#,
        )
        .bind(filter.enabled)
        .bind(
            filter
                .target_type
                .map(|t: ScheduleTargetType| t.to_string()),
        )
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to count schedules: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let count: i64 = row.get("count");
        Ok(count as u64)
    }

    #[instrument(skip(self, update))]
    async fn update_schedule(&self, id: Uuid, update: UpdateSchedule) -> Result<(), StoreError> {
        // Build dynamic update query
        let result = sqlx::query(
            r#"
            UPDATE durable_schedules
            SET name = COALESCE($2, name),
                description = CASE WHEN $3 THEN $4 ELSE description END,
                cron_expression = COALESCE($5, cron_expression),
                timezone = COALESCE($6, timezone),
                target_type = COALESCE($7, target_type),
                target_name = COALESCE($8, target_name),
                target_input = COALESCE($9, target_input),
                enabled = COALESCE($10, enabled),
                max_concurrent = CASE WHEN $11 THEN $12 ELSE max_concurrent END,
                catch_up_missed = COALESCE($13, catch_up_missed),
                max_catch_up = CASE WHEN $14 THEN $15 ELSE max_catch_up END,
                retry_policy = CASE WHEN $16 THEN $17 ELSE retry_policy END,
                next_trigger_at = CASE WHEN $18 THEN $19 ELSE next_trigger_at END,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(update.name)
        .bind(update.description.is_changed())
        .bind(update.description.into_value())
        .bind(update.cron_expression)
        .bind(update.timezone)
        .bind(
            update
                .target_type
                .map(|t: ScheduleTargetType| t.to_string()),
        )
        .bind(update.target_name)
        .bind(update.target_input)
        .bind(update.enabled)
        .bind(update.max_concurrent.is_changed())
        .bind(update.max_concurrent.into_value().map(|v| v as i32))
        .bind(update.catch_up_missed)
        .bind(update.max_catch_up.is_changed())
        .bind(update.max_catch_up.into_value().map(|v| v as i32))
        .bind(update.retry_policy.is_changed())
        .bind(update.retry_policy.into_value())
        .bind(update.next_trigger_at.is_changed())
        .bind(update.next_trigger_at.into_value())
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to update schedule: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleNotFound(id));
        }

        debug!(%id, "updated schedule");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn delete_schedule(&self, id: Uuid) -> Result<(), StoreError> {
        let result = sqlx::query("DELETE FROM durable_schedules WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                error!("Failed to delete schedule: {}", e);
                StoreError::Database(e.to_string())
            })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleNotFound(id));
        }

        debug!(%id, "deleted schedule");
        Ok(())
    }

    // =========================================================================
    // Scheduler Operations
    // =========================================================================

    #[instrument(skip(self))]
    async fn claim_due_schedules(
        &self,
        scheduler_id: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        // Claim due schedules ordered by next_trigger_at
        let rows = sqlx::query(
            r#"
            WITH due_schedules AS (
                SELECT id
                FROM durable_schedules
                WHERE enabled = true
                  AND next_trigger_at <= NOW()
                  AND (claimed_by IS NULL OR claimed_at < NOW() - INTERVAL '30 seconds')
                ORDER BY next_trigger_at
                FOR UPDATE SKIP LOCKED
                LIMIT $2
            )
            UPDATE durable_schedules s
            SET claimed_by = $1, claimed_at = NOW()
            FROM due_schedules d
            WHERE s.id = d.id
            RETURNING s.id, s.name, s.description, s.cron_expression, s.timezone,
                      s.target_type, s.target_name, s.target_input, s.enabled,
                      s.max_concurrent, s.catch_up_missed, s.max_catch_up, s.retry_policy,
                      s.last_triggered_at, s.next_trigger_at, s.claimed_by, s.claimed_at,
                      s.created_at, s.updated_at
            "#,
        )
        .bind(scheduler_id)
        .bind(limit as i32)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to claim due schedules: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let schedules: Result<Vec<_>, _> = rows.into_iter().map(parse_schedule_row).collect();
        let schedules = schedules?;

        debug!(
            count = schedules.len(),
            scheduler_id, "claimed due schedules"
        );
        Ok(schedules)
    }

    #[instrument(skip(self))]
    async fn update_next_trigger(
        &self,
        id: Uuid,
        next: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_schedules
            SET next_trigger_at = $2,
                last_triggered_at = NOW(),
                claimed_by = NULL,
                claimed_at = NULL,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(next)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to update next trigger: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleNotFound(id));
        }

        debug!(%id, %next, "updated next trigger");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn skip_schedule_trigger(&self, id: Uuid) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_schedules
            SET claimed_by = NULL, claimed_at = NULL
            WHERE id = $1
            "#,
        )
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to skip schedule trigger: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%id, "skipped schedule trigger");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn release_schedule(&self, id: Uuid) -> Result<(), StoreError> {
        self.skip_schedule_trigger(id).await
    }

    // =========================================================================
    // Schedule Execution Operations
    // =========================================================================

    #[instrument(skip(self))]
    async fn create_schedule_execution(
        &self,
        schedule_id: Uuid,
        scheduled_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Uuid, StoreError> {
        let id = Uuid::now_v7();

        sqlx::query(
            r#"
            INSERT INTO durable_schedule_executions (id, schedule_id, scheduled_at, status)
            VALUES ($1, $2, $3, 'running')
            "#,
        )
        .bind(id)
        .bind(schedule_id)
        .bind(scheduled_at)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to create schedule execution: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(%id, %schedule_id, "created schedule execution");
        Ok(id)
    }

    #[instrument(skip(self))]
    async fn get_schedule_execution(&self, id: Uuid) -> Result<ScheduleExecutionRow, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT id, schedule_id, scheduled_at, started_at, completed_at,
                   status, workflow_id, task_id, error, duration_ms, created_at
            FROM durable_schedule_executions
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get schedule execution: {}", e);
            StoreError::Database(e.to_string())
        })?
        .ok_or(StoreError::ScheduleExecutionNotFound(id))?;

        parse_schedule_execution_row(row)
    }

    #[instrument(skip(self))]
    async fn complete_schedule_execution(
        &self,
        execution_id: Uuid,
        target_id: Uuid,
        is_workflow: bool,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_schedule_executions
            SET status = 'completed',
                completed_at = NOW(),
                duration_ms = EXTRACT(MILLISECONDS FROM (NOW() - started_at))::int,
                workflow_id = CASE WHEN $2 THEN $3 ELSE workflow_id END,
                task_id = CASE WHEN NOT $2 THEN $3 ELSE task_id END
            WHERE id = $1
            "#,
        )
        .bind(execution_id)
        .bind(is_workflow)
        .bind(target_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to complete schedule execution: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleExecutionNotFound(execution_id));
        }

        debug!(%execution_id, "completed schedule execution");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn fail_schedule_execution(
        &self,
        execution_id: Uuid,
        error: &str,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_schedule_executions
            SET status = 'failed',
                completed_at = NOW(),
                duration_ms = EXTRACT(MILLISECONDS FROM (NOW() - started_at))::int,
                error = $2
            WHERE id = $1
            "#,
        )
        .bind(execution_id)
        .bind(error)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to fail schedule execution: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleExecutionNotFound(execution_id));
        }

        debug!(%execution_id, "failed schedule execution");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn skip_schedule_execution(
        &self,
        execution_id: Uuid,
        reason: &str,
    ) -> Result<(), StoreError> {
        let result = sqlx::query(
            r#"
            UPDATE durable_schedule_executions
            SET status = 'skipped',
                completed_at = NOW(),
                duration_ms = EXTRACT(MILLISECONDS FROM (NOW() - started_at))::int,
                error = $2
            WHERE id = $1
            "#,
        )
        .bind(execution_id)
        .bind(reason)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to skip schedule execution: {}", e);
            StoreError::Database(e.to_string())
        })?;

        if result.rows_affected() == 0 {
            return Err(StoreError::ScheduleExecutionNotFound(execution_id));
        }

        debug!(%execution_id, "skipped schedule execution");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn list_schedule_executions(
        &self,
        filter: ScheduleExecutionFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleExecutionRow>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT id, schedule_id, scheduled_at, started_at, completed_at,
                   status, workflow_id, task_id, error, duration_ms, created_at
            FROM durable_schedule_executions
            WHERE ($1::uuid IS NULL OR schedule_id = $1)
              AND ($2::text IS NULL OR status = $2)
            ORDER BY created_at DESC
            OFFSET $3 LIMIT $4
            "#,
        )
        .bind(filter.schedule_id)
        .bind(
            filter
                .status
                .map(|s: ScheduleExecutionStatus| s.to_string()),
        )
        .bind(pagination.offset as i64)
        .bind(pagination.limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to list schedule executions: {}", e);
            StoreError::Database(e.to_string())
        })?;

        rows.into_iter().map(parse_schedule_execution_row).collect()
    }

    #[instrument(skip(self))]
    async fn count_running_executions(&self, schedule_id: Uuid) -> Result<u32, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as count
            FROM durable_schedule_executions
            WHERE schedule_id = $1 AND status = 'running'
            "#,
        )
        .bind(schedule_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to count running executions: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let count: i64 = row.get("count");
        Ok(count as u32)
    }

    #[instrument(skip(self))]
    async fn get_schedule_stats(&self, schedule_id: Uuid) -> Result<ScheduleStats, StoreError> {
        let row = sqlx::query(
            r#"
            SELECT
                COUNT(*) as total,
                COUNT(*) FILTER (WHERE status = 'completed') as successful,
                COUNT(*) FILTER (WHERE status = 'failed') as failed,
                COUNT(*) FILTER (WHERE status = 'skipped') as skipped,
                AVG(duration_ms) FILTER (WHERE duration_ms IS NOT NULL) as avg_duration
            FROM durable_schedule_executions
            WHERE schedule_id = $1
            "#,
        )
        .bind(schedule_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get schedule stats: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let total: i64 = row.get("total");
        let successful: i64 = row.get("successful");
        let failed: i64 = row.get("failed");
        let skipped: i64 = row.get("skipped");
        let avg_duration: Option<f64> = row.get("avg_duration");

        // Get last execution status
        let last_row = sqlx::query(
            r#"
            SELECT status
            FROM durable_schedule_executions
            WHERE schedule_id = $1
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(schedule_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to get last execution status: {}", e);
            StoreError::Database(e.to_string())
        })?;

        let last_execution_status = last_row
            .map(|r| {
                let status: String = r.get("status");
                parse_schedule_execution_status(&status)
            })
            .transpose()?;

        Ok(ScheduleStats {
            total_executions: total as u64,
            successful_executions: successful as u64,
            failed_executions: failed as u64,
            skipped_executions: skipped as u64,
            avg_duration_ms: avg_duration.map(|v| v as u64),
            last_execution_status,
        })
    }

    // =========================================================================
    // Scheduler Instance Operations
    // =========================================================================

    #[instrument(skip(self, instance))]
    async fn register_scheduler_instance(
        &self,
        instance: SchedulerInstanceInfo,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            INSERT INTO durable_scheduler_instances (instance_id, started_at, last_heartbeat_at, schedules_processed, hostname, version)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (instance_id) DO UPDATE
            SET started_at = $2, last_heartbeat_at = $3, schedules_processed = $4, hostname = $5, version = $6
            "#,
        )
        .bind(&instance.instance_id)
        .bind(instance.started_at)
        .bind(instance.last_heartbeat_at)
        .bind(instance.schedules_processed as i64)
        .bind(&instance.hostname)
        .bind(&instance.version)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to register scheduler instance: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(instance_id = %instance.instance_id, "registered scheduler instance");
        Ok(())
    }

    #[instrument(skip(self))]
    async fn heartbeat_scheduler_instance(
        &self,
        instance_id: &str,
        schedules_processed: u64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            UPDATE durable_scheduler_instances
            SET last_heartbeat_at = NOW(), schedules_processed = $2
            WHERE instance_id = $1
            "#,
        )
        .bind(instance_id)
        .bind(schedules_processed as i64)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to heartbeat scheduler instance: {}", e);
            StoreError::Database(e.to_string())
        })?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn list_scheduler_instances(&self) -> Result<Vec<SchedulerInstanceInfo>, StoreError> {
        let rows = sqlx::query(
            r#"
            SELECT instance_id, started_at, last_heartbeat_at, schedules_processed, hostname, version
            FROM durable_scheduler_instances
            ORDER BY started_at DESC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to list scheduler instances: {}", e);
            StoreError::Database(e.to_string())
        })?;

        Ok(rows
            .into_iter()
            .map(|row| SchedulerInstanceInfo {
                instance_id: row.get("instance_id"),
                started_at: row.get("started_at"),
                last_heartbeat_at: row.get("last_heartbeat_at"),
                schedules_processed: row.get::<i64, _>("schedules_processed") as u64,
                hostname: row.get("hostname"),
                version: row.get("version"),
            })
            .collect())
    }

    #[instrument(skip(self))]
    async fn deregister_scheduler_instance(&self, instance_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"
            DELETE FROM durable_scheduler_instances WHERE instance_id = $1
            "#,
        )
        .bind(instance_id)
        .execute(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to deregister scheduler instance: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(instance_id, "deregistered scheduler instance");
        Ok(())
    }
}

// Helper functions

fn parse_schedule_row(row: sqlx::postgres::PgRow) -> Result<ScheduleRow, StoreError> {
    use sqlx::Row;
    let target_type_str: String = row.get("target_type");
    let target_type = parse_schedule_target_type(&target_type_str)?;

    Ok(ScheduleRow {
        id: row.get("id"),
        name: row.get("name"),
        description: row.get("description"),
        cron_expression: row.get("cron_expression"),
        timezone: row.get("timezone"),
        target_type,
        target_name: row.get("target_name"),
        target_input: row.get("target_input"),
        enabled: row.get("enabled"),
        max_concurrent: row
            .get::<Option<i32>, _>("max_concurrent")
            .map(|v| v as u32),
        catch_up_missed: row.get("catch_up_missed"),
        max_catch_up: row.get::<Option<i32>, _>("max_catch_up").map(|v| v as u32),
        retry_policy: row.get("retry_policy"),
        last_triggered_at: row.get("last_triggered_at"),
        next_trigger_at: row.get("next_trigger_at"),
        claimed_by: row.get("claimed_by"),
        claimed_at: row.get("claimed_at"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

fn parse_schedule_execution_row(
    row: sqlx::postgres::PgRow,
) -> Result<ScheduleExecutionRow, StoreError> {
    use sqlx::Row;
    let status_str: String = row.get("status");
    let status = parse_schedule_execution_status(&status_str)?;

    Ok(ScheduleExecutionRow {
        id: row.get("id"),
        schedule_id: row.get("schedule_id"),
        scheduled_at: row.get("scheduled_at"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        status,
        workflow_id: row.get("workflow_id"),
        task_id: row.get("task_id"),
        error: row.get("error"),
        duration_ms: row.get("duration_ms"),
        created_at: row.get("created_at"),
    })
}

fn parse_schedule_target_type(target_type: &str) -> Result<ScheduleTargetType, StoreError> {
    match target_type {
        "workflow" => Ok(ScheduleTargetType::Workflow),
        "activity" => Ok(ScheduleTargetType::Activity),
        _ => Err(StoreError::Database(format!(
            "Unknown schedule target type: {}",
            target_type
        ))),
    }
}

fn parse_schedule_execution_status(status: &str) -> Result<ScheduleExecutionStatus, StoreError> {
    match status {
        "pending" => Ok(ScheduleExecutionStatus::Pending),
        "running" => Ok(ScheduleExecutionStatus::Running),
        "completed" => Ok(ScheduleExecutionStatus::Completed),
        "failed" => Ok(ScheduleExecutionStatus::Failed),
        "skipped" => Ok(ScheduleExecutionStatus::Skipped),
        _ => Err(StoreError::Database(format!(
            "Unknown schedule execution status: {}",
            status
        ))),
    }
}

fn parse_circuit_state(state: &str) -> Result<CircuitState, StoreError> {
    match state {
        "closed" => Ok(CircuitState::Closed),
        "open" => Ok(CircuitState::Open),
        "half_open" => Ok(CircuitState::HalfOpen),
        _ => Err(StoreError::Database(format!(
            "Unknown circuit state: {}",
            state
        ))),
    }
}

fn parse_workflow_status(status: &str) -> Result<WorkflowStatus, StoreError> {
    match status {
        "pending" => Ok(WorkflowStatus::Pending),
        "running" => Ok(WorkflowStatus::Running),
        "completed" => Ok(WorkflowStatus::Completed),
        "failed" => Ok(WorkflowStatus::Failed),
        "cancelled" => Ok(WorkflowStatus::Cancelled),
        "continued_as_new" => Ok(WorkflowStatus::ContinuedAsNew),
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
    // Unit tests for JSON sanitization that do not require a PostgreSQL database.
    // PostgreSQL-backed integration tests can be run with:
    // cargo test -p everruns-durable --test integration_test -- --test-threads=1

    use super::*;
    use serde_json::json;

    #[test]
    fn sanitize_removes_null_bytes_from_strings() {
        let input = json!({"key": "hello\0world"});
        let result = sanitize_json_null_bytes(input);
        assert_eq!(result, json!({"key": "helloworld"}));
    }

    #[test]
    fn sanitize_handles_nested_structures() {
        let input = json!({
            "outer": {
                "inner": "has\0null",
                "clean": "no nulls"
            },
            "arr": ["a\0b", "clean"]
        });
        let result = sanitize_json_null_bytes(input);
        assert_eq!(
            result,
            json!({
                "outer": {
                    "inner": "hasnull",
                    "clean": "no nulls"
                },
                "arr": ["ab", "clean"]
            })
        );
    }

    #[test]
    fn sanitize_preserves_non_string_values() {
        let input = json!({"num": 42, "bool": true, "null": null});
        let result = sanitize_json_null_bytes(input.clone());
        assert_eq!(result, input);
    }

    #[test]
    fn sanitize_noop_when_no_null_bytes() {
        let input = json!({"key": "normal string", "nested": [1, 2, 3]});
        let result = sanitize_json_null_bytes(input.clone());
        assert_eq!(result, input);
    }
}
