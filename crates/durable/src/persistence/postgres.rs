//! PostgreSQL implementation of WorkflowEventStore
//!
//! Production-ready persistence using PostgreSQL with:
//! - Optimistic concurrency control via sequence numbers
//! - Efficient task claiming with SKIP LOCKED
//! - Event sourcing for workflow replay

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use tracing::{debug, error, info, instrument};
use uuid::Uuid;

mod admin;
mod circuit_breakers;
mod dlq;
mod event_log;
mod hand_off;
mod schedules;
mod schema;
mod signals;
mod task_queue;
mod workers;

use super::db_failure::store_failure;
use super::store::{
    CapacitySnapshot, CircuitBreakerState, CircuitBreakers, ClaimedTask, CreateScheduleRow,
    DeadLetters, DeadTaskInfo, DlqEntry, DlqFilter, DurableAdmin, EventLog, HeartbeatResponse,
    Pagination, ReclaimResult, RunStart, RunSteering, ScheduleExecutionFilter,
    ScheduleExecutionRow, ScheduleExecutionStatus, ScheduleFilter, ScheduleRow, ScheduleStats,
    ScheduleTargetType, SchedulerInstanceInfo, Schedules, SealedTaskInfo, SignalStore, StoreError,
    SystemHealth, TaskDefinition, TaskFailureOutcome, TaskFilter, TaskInfo, TaskQueue, TaskStatus,
    TraceContext, UpdateSchedule, WORKER_HEARTBEAT_TIMEOUT_SECS, WorkerFilter, WorkerHeartbeat,
    WorkerInfo, WorkerRegistry, WorkflowEventInfo, WorkflowFilter, WorkflowInfo,
    WorkflowInfoExtended, WorkflowStatus, no_progress_seal_threshold_from_env,
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
/// ```no_run
/// # async fn run() -> Result<(), sqlx::Error> {
/// let pool = sqlx::PgPool::connect("postgres://localhost/everruns").await?;
/// let store = everruns_durable::PostgresWorkflowEventStore::new(pool);
/// # Ok(()) }
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

    async fn existing_initial_task_id(&self, workflow_id: Uuid) -> Result<Uuid, StoreError> {
        sqlx::query_scalar(
            r#"
            SELECT id
            FROM durable_task_queue
            WHERE workflow_id = $1
            ORDER BY created_at ASC, id ASC
            LIMIT 1
            "#,
        )
        .bind(workflow_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| {
            error!("Failed to load existing workflow task: {}", e);
            StoreError::Database(e.to_string())
        })?
        .ok_or_else(|| {
            StoreError::Database(format!(
                "workflow {workflow_id} already exists without an initial task"
            ))
        })
    }

    /// Create a workflow, write its initial events, and enqueue the first task in
    /// one transaction. This is the hot path for starting a new workflow run.
    pub async fn start_workflow_with_task(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        task: TaskDefinition,
    ) -> Result<Uuid, StoreError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::Database(e.to_string()))?;

        if !insert_started_workflow(&mut tx, workflow_id, workflow_type, input, &task).await? {
            tx.rollback().await.ok();
            let existing_task_id = self.existing_initial_task_id(workflow_id).await?;
            debug!(
                %workflow_id,
                %existing_task_id,
                "reused existing initial workflow task after duplicate start"
            );
            return Ok(existing_task_id);
        }

        let task_id = insert_workflow_task(&mut tx, workflow_id, &task).await?;

        tx.commit().await.map_err(|e| {
            error!("Failed to commit initial workflow start: {}", e);
            StoreError::Database(e.to_string())
        })?;

        debug!(
            %workflow_id,
            %task_id,
            activity_type = %task.activity_type,
            "started workflow and enqueued initial task"
        );
        Ok(task_id)
    }
}

/// Insert `workflow_id` as Running with its `WorkflowStarted` and
/// `ActivityScheduled` events. Returns false, writing nothing, when the
/// workflow already exists.
async fn insert_started_workflow(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workflow_id: Uuid,
    workflow_type: &str,
    input: serde_json::Value,
    task: &TaskDefinition,
) -> Result<bool, StoreError> {
    let workflow_input = sanitize_json_null_bytes(input);
    let workflow_started = WorkflowEvent::started(workflow_input.clone());
    let activity_scheduled = WorkflowEvent::ActivityScheduled {
        activity_id: task.activity_id.clone(),
        activity_type: task.activity_type.clone(),
        input: sanitize_json_null_bytes(task.input.clone()),
        options: task.options.clone(),
    };
    let workflow_started_data = serde_json::to_value(&workflow_started)
        .map(sanitize_json_null_bytes)
        .map_err(|e| StoreError::Serialization(e.to_string()))?;
    let activity_scheduled_data = serde_json::to_value(&activity_scheduled)
        .map(sanitize_json_null_bytes)
        .map_err(|e| StoreError::Serialization(e.to_string()))?;

    let inserted_workflow_id = sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO durable_workflow_instances (
            id, workflow_type, status, input, started_at
        )
        VALUES ($1, $2, 'running', $3, NOW())
        ON CONFLICT (id) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(workflow_id)
    .bind(workflow_type)
    .bind(&workflow_input)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| {
        error!("Failed to create started workflow: {}", e);
        StoreError::Database(e.to_string())
    })?;

    if inserted_workflow_id.is_none() {
        return Ok(false);
    }

    sqlx::query(
        r#"
        INSERT INTO durable_workflow_events (
            workflow_id, sequence_num, event_type, event_data
        )
        VALUES
            ($1, 0, 'workflow_started', $2),
            ($1, 1, 'activity_scheduled', $3)
        "#,
    )
    .bind(workflow_id)
    .bind(&workflow_started_data)
    .bind(&activity_scheduled_data)
    .execute(&mut **tx)
    .await
    .map_err(|e| {
        error!("Failed to write initial workflow events: {}", e);
        StoreError::Database(e.to_string())
    })?;

    Ok(true)
}

/// Insert `task` as a pending task of `workflow_id`, with no pending-cap check
/// (callers use it right after creating or resetting the workflow).
async fn insert_workflow_task(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workflow_id: Uuid,
    task: &TaskDefinition,
) -> Result<Uuid, StoreError> {
    let task_id = Uuid::now_v7();
    let task_input = sanitize_json_null_bytes(task.input.clone());
    let options_json = serde_json::to_value(&task.options)
        .map(sanitize_json_null_bytes)
        .map_err(|e| StoreError::Serialization(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO durable_task_queue (
            id, workflow_id, activity_id, activity_type, input, options,
            max_attempts, priority, visible_at,
            schedule_to_start_timeout_ms, start_to_close_timeout_ms, heartbeat_timeout_ms,
            queue
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW() + $12::bigint * INTERVAL '1 millisecond', $9, $10, $11, $13)
        "#,
    )
    .bind(task_id)
    .bind(workflow_id)
    .bind(&task.activity_id)
    .bind(&task.activity_type)
    .bind(&task_input)
    .bind(&options_json)
    .bind(task.options.retry_policy.max_attempts as i32)
    .bind(task.options.priority)
    .bind(task.options.schedule_to_start_timeout.as_millis() as i64)
    .bind(task.options.start_to_close_timeout.as_millis() as i64)
    .bind(task.options.heartbeat_timeout.map(|d| d.as_millis() as i64))
    .bind(task.options.start_delay.map_or(0, |d| d.as_millis() as i64))
    .bind(task.options.queue.as_deref())
    .execute(&mut **tx)
    .await
    .map_err(|e| {
        error!("Failed to enqueue initial workflow task: {}", e);
        StoreError::Database(e.to_string())
    })?;

    Ok(task_id)
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

/// Map an `append_events` INSERT error to the appropriate `StoreError`.
///
/// EVE-639: the append path relies on DB constraints rather than a prior
/// FOR UPDATE + MAX(...) read, so we translate the two expected violations:
/// - unique `(workflow_id, sequence_num)` (SQLSTATE 23505) → a concurrency
///   conflict (a competing writer already occupied one of our sequences). The
///   actual conflicting sequence is not cheaply known here; callers recompute
///   it via `count_events` on retry, so `actual` is reported as -1 (unknown).
/// - foreign-key on `workflow_id` (SQLSTATE 23503) → workflow not found.
fn map_append_events_error(
    err: sqlx::Error,
    workflow_id: Uuid,
    expected_sequence: i32,
) -> StoreError {
    if let Some(db_err) = err.as_database_error() {
        if db_err.is_unique_violation() {
            return StoreError::ConcurrencyConflict {
                expected: expected_sequence,
                actual: -1,
            };
        }
        if db_err.is_foreign_key_violation() {
            return StoreError::WorkflowNotFound(workflow_id);
        }
    }
    StoreError::Database(err.to_string())
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
