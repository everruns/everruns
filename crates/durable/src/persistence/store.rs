//! WorkflowEventStore trait definition

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::update_field::UpdateField;
use crate::workflow::{ActivityOptions, WorkflowEvent, WorkflowSignal};

/// Default snapshot interval: save a snapshot every N events.
/// Configurable via `DURABLE_SNAPSHOT_INTERVAL` env var.
pub const DEFAULT_SNAPSHOT_INTERVAL: i32 = 1000;

/// Read snapshot interval from env, falling back to default.
pub fn snapshot_interval_from_env() -> i32 {
    std::env::var("DURABLE_SNAPSHOT_INTERVAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_SNAPSHOT_INTERVAL)
}

/// Default max pending (unclaimed) tasks per workflow.
/// Prevents a single workflow from flooding the queue.
pub const DEFAULT_MAX_PENDING_TASKS_PER_WORKFLOW: u32 = 100;

/// Default max pending standalone tasks (generic queue).
/// Prevents unbounded queue growth when no workflow limits apply.
pub const DEFAULT_MAX_PENDING_STANDALONE_TASKS: u32 = 10_000;

/// Workers with no heartbeat within this many seconds are considered stale.
/// Used by get_system_health, list_workers, and reclaim_stale_tasks (stale worker cleanup).
pub const WORKER_HEARTBEAT_TIMEOUT_SECS: i64 = 60;

/// Read max pending tasks per workflow from env, falling back to default.
pub fn max_pending_tasks_per_workflow_from_env() -> u32 {
    std::env::var("DURABLE_MAX_PENDING_TASKS_PER_WORKFLOW")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_MAX_PENDING_TASKS_PER_WORKFLOW)
}

/// Error type for store operations
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Workflow not found
    #[error("workflow not found: {0}")]
    WorkflowNotFound(Uuid),

    /// Task not found
    #[error("task not found: {0}")]
    TaskNotFound(Uuid),

    /// Task not owned by the worker (was reclaimed)
    #[error("task {0} not owned by worker (was reclaimed or already completed)")]
    TaskNotOwned(Uuid),

    /// Circuit breaker not found
    #[error("circuit breaker not found: {0}")]
    CircuitBreakerNotFound(String),

    /// Schedule not found
    #[error("schedule not found: {0}")]
    ScheduleNotFound(Uuid),

    /// Schedule execution not found
    #[error("schedule execution not found: {0}")]
    ScheduleExecutionNotFound(Uuid),

    /// Schedule limit exceeded
    #[error("schedule limit exceeded: {current}/{limit} schedules")]
    ScheduleLimitExceeded { current: u32, limit: u32 },

    /// Task queue limit exceeded (per-workflow pending task cap)
    #[error(
        "task queue limit exceeded for workflow {workflow_id}: {current}/{limit} pending tasks"
    )]
    TaskQueueLimitExceeded {
        workflow_id: Uuid,
        current: u32,
        limit: u32,
    },

    /// Standalone task queue limit exceeded (global cap)
    #[error("standalone task queue limit exceeded: {current}/{limit} pending tasks")]
    StandaloneTaskQueueLimitExceeded { current: u32, limit: u32 },

    /// Invalid cron expression
    #[error("invalid cron expression: {0}")]
    InvalidCronExpression(String),

    /// Cron interval too short
    #[error("cron interval too short: {actual}s < {minimum}s minimum")]
    CronIntervalTooShort { actual: u64, minimum: u64 },

    /// Concurrency conflict (optimistic locking failed)
    #[error("concurrency conflict: expected sequence {expected}, got {actual}")]
    ConcurrencyConflict { expected: i32, actual: i32 },

    /// Database error
    #[error("database error: {0}")]
    Database(String),

    /// Serialization error
    #[error("serialization error: {0}")]
    Serialization(String),
}

/// Workflow status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    /// Workflow created but not started
    Pending,

    /// Workflow is running
    Running,

    /// Workflow completed successfully
    Completed,

    /// Workflow failed
    Failed,

    /// Workflow was cancelled
    Cancelled,

    /// Workflow continued as a new workflow (history rolled over)
    ContinuedAsNew,
}

impl WorkflowStatus {
    /// Check if this status is terminal (workflow has ended)
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::ContinuedAsNew
        )
    }
}

impl std::fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
            Self::ContinuedAsNew => write!(f, "continued_as_new"),
        }
    }
}

/// Task status in the queue
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Claimed,
    Completed,
    Failed,
    Dead,
    Cancelled,
}

/// Definition of a task to be enqueued.
/// When `workflow_id` is `None`, this is a standalone (generic queue) task
/// that runs independently of any workflow.
#[derive(Debug, Clone)]
pub struct TaskDefinition {
    pub workflow_id: Option<Uuid>,
    pub activity_id: String,
    pub activity_type: String,
    pub input: serde_json::Value,
    pub options: ActivityOptions,
}

/// A task that has been claimed by a worker
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub id: Uuid,
    pub workflow_id: Option<Uuid>,
    pub activity_id: String,
    pub activity_type: String,
    pub input: serde_json::Value,
    pub options: ActivityOptions,
    pub attempt: u32,
    pub max_attempts: u32,
}

/// Response from heartbeat operation
#[derive(Debug, Clone)]
pub struct HeartbeatResponse {
    /// Whether the heartbeat was accepted
    pub accepted: bool,

    /// Whether cancellation was requested
    pub should_cancel: bool,
}

/// Outcome of failing a task
#[derive(Debug, Clone)]
pub enum TaskFailureOutcome {
    /// Task will be retried
    WillRetry { next_attempt: u32, delay: Duration },

    /// Task moved to dead letter queue
    MovedToDlq,

    /// Task completed (no more retries, workflow notified)
    ExhaustedRetries,
}

/// Filter for listing workers
#[derive(Debug, Clone, Default)]
pub struct WorkerFilter {
    pub status: Option<String>,
    pub worker_group: Option<String>,
}

impl WorkerFilter {
    pub fn active() -> Self {
        Self {
            status: Some("active".to_string()),
            worker_group: None,
        }
    }
}

/// Filter for listing workflows
#[derive(Debug, Clone, Default)]
pub struct WorkflowFilter {
    pub status: Option<WorkflowStatus>,
    pub workflow_type: Option<String>,
}

/// Filter for listing tasks
#[derive(Debug, Clone, Default)]
pub struct TaskFilter {
    pub status: Option<TaskStatus>,
    pub activity_type: Option<String>,
    pub workflow_id: Option<Uuid>,
    /// When true, only return standalone tasks (workflow_id IS NULL)
    pub standalone_only: bool,
}

/// Task information for listing
#[derive(Debug, Clone)]
pub struct TaskInfo {
    pub id: Uuid,
    pub workflow_id: Option<Uuid>,
    pub activity_id: String,
    pub activity_type: String,
    pub status: TaskStatus,
    pub priority: i32,
    pub attempt: u32,
    pub max_attempts: u32,
    pub claimed_by: Option<String>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub claimed_at: Option<DateTime<Utc>>,
}

/// Extended workflow information with timestamps
#[derive(Debug, Clone)]
pub struct WorkflowInfoExtended {
    pub id: Uuid,
    pub workflow_type: String,
    pub status: WorkflowStatus,
    pub input: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<crate::workflow::WorkflowError>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    /// If this workflow continued as a new workflow, the ID of the new workflow
    pub continued_as_new_id: Option<Uuid>,
}

/// System health summary
#[derive(Debug, Clone)]
pub struct SystemHealth {
    pub total_workers: usize,
    pub active_workers: usize,
    pub workers_accepting: usize,
    pub total_capacity: usize,
    pub current_load: usize,
    pub pending_tasks: usize,
    pub claimed_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    /// Cumulative: tasks that have ever been started (claimed_at IS NOT NULL)
    pub started_tasks: usize,
    pub running_workflows: usize,
    pub pending_workflows: usize,
    pub completed_workflows: usize,
    pub failed_workflows: usize,
    /// Cumulative: workflows that have ever been started (started_at IS NOT NULL)
    pub started_workflows: usize,
    pub dlq_size: usize,
}

/// Workflow event info for API responses
#[derive(Debug, Clone)]
pub struct WorkflowEventInfo {
    pub id: i64,
    pub workflow_id: Uuid,
    pub sequence_num: i32,
    pub event_type: String,
    pub event_data: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Helper to get event type name
pub fn event_type_name(event: &WorkflowEvent) -> &'static str {
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

/// Worker information
#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub id: String,
    pub worker_group: Option<String>,
    pub activity_types: Vec<String>,
    pub max_concurrency: u32,
    pub current_load: u32,
    pub status: String,
    pub accepting_tasks: bool,
    pub backpressure_reason: Option<String>,
    pub started_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub hostname: Option<String>,
    pub version: Option<String>,
    pub metadata: Option<serde_json::Value>,
    /// Total tasks completed by this worker
    pub tasks_completed: u64,
    /// Total tasks failed by this worker
    pub tasks_failed: u64,
    /// Average task duration in milliseconds
    pub avg_task_duration_ms: Option<u64>,
}

/// Snapshot of total system worker capacity for fair-share claiming.
#[derive(Debug, Clone, Default)]
pub struct CapacitySnapshot {
    /// Total available slots across all active, accepting workers
    pub total_available: u32,
    /// Number of active workers that are accepting tasks
    pub active_workers: u32,
}

/// Filter for listing DLQ entries
#[derive(Debug, Clone, Default)]
pub struct DlqFilter {
    pub workflow_id: Option<Uuid>,
    pub activity_type: Option<String>,
}

/// Pagination parameters
#[derive(Debug, Clone)]
pub struct Pagination {
    pub offset: u32,
    pub limit: u32,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: 100,
        }
    }
}

/// Dead letter queue entry
#[derive(Debug, Clone)]
pub struct DlqEntry {
    pub id: Uuid,
    pub original_task_id: Uuid,
    pub workflow_id: Option<Uuid>,
    pub activity_id: String,
    pub activity_type: String,
    pub input: serde_json::Value,
    pub attempts: u32,
    pub last_error: String,
    pub error_history: Vec<String>,
    pub dead_at: DateTime<Utc>,
}

/// Trace context for distributed tracing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceContext {
    pub trace_id: String,
    pub span_id: String,
    pub trace_flags: u8,
}

/// Workflow information stored in the database
#[derive(Debug, Clone)]
pub struct WorkflowInfo {
    pub id: Uuid,
    pub workflow_type: String,
    pub status: WorkflowStatus,
    pub input: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<crate::workflow::WorkflowError>,
    /// If this workflow continued as a new workflow, the ID of the new workflow
    pub continued_as_new_id: Option<Uuid>,
}

/// A snapshot of serialized workflow state at a specific event sequence.
///
/// Used to checkpoint replay: instead of replaying all events from sequence 0,
/// the engine loads the latest snapshot and replays only events after it.
#[derive(Debug, Clone)]
pub struct WorkflowSnapshot {
    /// The workflow this snapshot belongs to
    pub workflow_id: Uuid,

    /// The event sequence number at which this snapshot was taken.
    /// When restoring, events with sequence_num > this value are replayed.
    pub sequence_num: i32,

    /// Serialized workflow state (opaque bytes, typically JSON)
    pub snapshot_data: Vec<u8>,

    /// When the snapshot was created
    pub created_at: DateTime<Utc>,
}

/// Store for workflow events and task queue
///
/// This trait defines the interface for persisting workflow state.
/// Implementations must be thread-safe and support concurrent access.
#[async_trait]
pub trait WorkflowEventStore: Send + Sync + 'static {
    // =========================================================================
    // Workflow Operations
    // =========================================================================

    /// Create a new workflow instance
    async fn create_workflow(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        trace_context: Option<&TraceContext>,
    ) -> Result<(), StoreError>;

    /// Get workflow status
    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError>;

    /// Get full workflow info
    async fn get_workflow_info(&self, workflow_id: Uuid) -> Result<WorkflowInfo, StoreError>;

    /// Append events to a workflow (with optimistic concurrency)
    ///
    /// Returns the new sequence number after appending.
    async fn append_events(
        &self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32, StoreError>;

    /// Load all events for a workflow (for replay)
    async fn load_events(&self, workflow_id: Uuid)
    -> Result<Vec<(i32, WorkflowEvent)>, StoreError>;

    /// Count events for a workflow without materializing the replay payload.
    ///
    /// Used to reject oversized full-history replays before fetching event data.
    async fn count_events(&self, workflow_id: Uuid) -> Result<usize, StoreError> {
        Ok(self.load_events(workflow_id).await?.len())
    }

    /// Count events after a given sequence number without materializing them.
    ///
    /// Used to reject oversized snapshot-based replays before fetching event data.
    async fn count_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<usize, StoreError> {
        Ok(self
            .load_events_after(workflow_id, after_sequence)
            .await?
            .len())
    }

    /// Load events for a workflow starting after a given sequence number.
    ///
    /// Used for snapshot-based replay: load only events after the snapshot point.
    async fn load_events_after(
        &self,
        workflow_id: Uuid,
        after_sequence: i32,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError> {
        // Default: filter from load_events (implementations should override for efficiency)
        let all = self.load_events(workflow_id).await?;
        Ok(all
            .into_iter()
            .filter(|(seq, _)| *seq > after_sequence)
            .collect())
    }

    // =========================================================================
    // Snapshot Operations (for replay checkpointing)
    // =========================================================================

    /// Save a workflow state snapshot at the given sequence number.
    ///
    /// The snapshot_data is opaque bytes (typically JSON-serialized workflow state).
    /// Implementations should use UPSERT semantics for idempotency.
    async fn save_snapshot(
        &self,
        _workflow_id: Uuid,
        _sequence_num: i32,
        _snapshot_data: Vec<u8>,
    ) -> Result<(), StoreError> {
        Ok(()) // Default no-op for stores that don't support snapshots
    }

    /// Load the latest snapshot for a workflow, if any.
    ///
    /// Returns None if no snapshots exist for this workflow.
    async fn load_latest_snapshot(
        &self,
        _workflow_id: Uuid,
    ) -> Result<Option<WorkflowSnapshot>, StoreError> {
        Ok(None) // Default: no snapshots
    }

    /// Delete all snapshots for a workflow (cleanup on workflow deletion).
    async fn delete_snapshots(&self, _workflow_id: Uuid) -> Result<(), StoreError> {
        Ok(()) // Default no-op
    }

    /// Update workflow status
    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        result: Option<serde_json::Value>,
        error: Option<crate::workflow::WorkflowError>,
    ) -> Result<(), StoreError>;

    // =========================================================================
    // Task Queue Operations
    // =========================================================================

    /// Enqueue an activity task
    async fn enqueue_task(&self, task: TaskDefinition) -> Result<Uuid, StoreError>;

    /// Claim tasks for execution
    ///
    /// Uses SELECT FOR UPDATE SKIP LOCKED for efficient concurrent claiming.
    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError>;

    /// Record task heartbeat
    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError>;

    /// Complete a task successfully
    ///
    /// The worker_id must match the worker that claimed the task.
    /// Returns `StoreError::TaskNotOwned` if the task was reclaimed by another worker.
    /// This prevents duplicate next activity scheduling when a task is reclaimed
    /// due to heartbeat timeout while the original worker is still completing it.
    async fn complete_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        result: serde_json::Value,
    ) -> Result<(), StoreError>;

    /// Fail a task (may requeue or send to DLQ)
    async fn fail_task(&self, task_id: Uuid, error: &str)
    -> Result<TaskFailureOutcome, StoreError>;

    /// Cancel all pending (unclaimed) tasks for a workflow.
    ///
    /// Returns the number of tasks cancelled. Does NOT affect claimed or
    /// completed tasks — only pending ones still in the queue.
    async fn cancel_pending_tasks_for_workflow(
        &self,
        _workflow_id: Uuid,
    ) -> Result<u64, StoreError> {
        Ok(0)
    }

    /// Get task info by ID
    async fn get_task(&self, task_id: Uuid) -> Result<TaskInfo, StoreError>;

    /// Find and reclaim stale tasks (no heartbeat)
    async fn reclaim_stale_tasks(&self, stale_threshold: Duration)
    -> Result<Vec<Uuid>, StoreError>;

    // =========================================================================
    // Signal Operations
    // =========================================================================

    /// Send a signal to a workflow
    async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), StoreError>;

    /// Get pending signals for a workflow
    async fn get_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError>;

    /// Mark signals as processed
    async fn mark_signals_processed(
        &self,
        workflow_id: Uuid,
        count: usize,
    ) -> Result<(), StoreError>;

    // =========================================================================
    // Worker Registry Operations (optional, default no-op)
    // =========================================================================

    /// Register a worker
    async fn register_worker(&self, _worker: WorkerInfo) -> Result<(), StoreError> {
        Ok(())
    }

    /// Update worker heartbeat and load
    async fn worker_heartbeat(
        &self,
        _worker_id: &str,
        _current_load: usize,
        _accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    /// Get all active workers
    async fn list_workers(&self, _filter: WorkerFilter) -> Result<Vec<WorkerInfo>, StoreError> {
        Ok(vec![])
    }

    /// Deregister a worker and reclaim all tasks claimed by it
    /// Returns the number of tasks that were reclaimed
    async fn deregister_worker(&self, _worker_id: &str) -> Result<usize, StoreError> {
        Ok(0)
    }

    /// Get a snapshot of total system worker capacity.
    /// Used by workers to compute fair-share claim limits.
    async fn get_capacity_snapshot(&self) -> Result<CapacitySnapshot, StoreError> {
        Ok(CapacitySnapshot::default())
    }

    // =========================================================================
    // Dead Letter Queue Operations
    // =========================================================================

    /// Move task to DLQ
    async fn move_to_dlq(
        &self,
        task_id: Uuid,
        error_history: Vec<String>,
    ) -> Result<(), StoreError>;

    /// Requeue task from DLQ
    async fn requeue_from_dlq(&self, dlq_id: Uuid) -> Result<Uuid, StoreError>;

    /// List DLQ entries
    async fn list_dlq(
        &self,
        filter: DlqFilter,
        pagination: Pagination,
    ) -> Result<Vec<DlqEntry>, StoreError>;

    // =========================================================================
    // Circuit Breaker Operations (FUTURE FEATURE - optional, default no-op)
    // =========================================================================
    // These operations are implemented in PostgresWorkflowEventStore but not yet
    // used in production. The DistributedCircuitBreaker struct in reliability/
    // is ready for integration when needed.
    // TODO: Integrate with LLM calls and external tool executions.

    /// Create a circuit breaker
    async fn create_circuit_breaker(
        &self,
        _key: &str,
        _config: &crate::reliability::CircuitBreakerConfig,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    /// Get circuit breaker state
    async fn get_circuit_breaker(
        &self,
        _key: &str,
    ) -> Result<Option<CircuitBreakerState>, StoreError> {
        Ok(None)
    }

    /// Update circuit breaker state
    async fn update_circuit_breaker(
        &self,
        _key: &str,
        _state: crate::reliability::CircuitState,
        _failure_count: u32,
        _success_count: u32,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    // =========================================================================
    // Utility Operations (optional, default no-op)
    // =========================================================================

    /// Count active (non-terminal) workflows
    async fn count_active_workflows(&self) -> Result<i64, StoreError> {
        Ok(0)
    }

    /// List workflows with filtering and pagination
    async fn list_workflows(
        &self,
        _filter: WorkflowFilter,
        _pagination: Pagination,
    ) -> Result<Vec<WorkflowInfoExtended>, StoreError> {
        Ok(vec![])
    }

    /// List tasks with filtering and pagination
    async fn list_tasks(
        &self,
        _filter: TaskFilter,
        _pagination: Pagination,
    ) -> Result<Vec<TaskInfo>, StoreError> {
        Ok(vec![])
    }

    /// List all circuit breakers
    async fn list_circuit_breakers(&self) -> Result<Vec<CircuitBreakerState>, StoreError> {
        Ok(vec![])
    }

    /// Force a circuit breaker to open (admin operation)
    async fn force_open_circuit_breaker(&self, _key: &str) -> Result<(), StoreError> {
        Ok(())
    }

    /// Force a circuit breaker to close (admin operation)
    async fn force_close_circuit_breaker(&self, _key: &str) -> Result<(), StoreError> {
        Ok(())
    }

    /// Delete a circuit breaker (reset to default)
    async fn delete_circuit_breaker(&self, _key: &str) -> Result<(), StoreError> {
        Ok(())
    }

    /// Drain a worker (set status to draining, stop accepting new tasks)
    async fn drain_worker(&self, _worker_id: &str) -> Result<(), StoreError> {
        Ok(())
    }

    /// Resume a draining worker (set status back to active, start accepting new tasks)
    async fn resume_worker(&self, _worker_id: &str) -> Result<(), StoreError> {
        Ok(())
    }

    /// Get system health summary
    async fn get_system_health(&self) -> Result<SystemHealth, StoreError> {
        Ok(SystemHealth {
            total_workers: 0,
            active_workers: 0,
            workers_accepting: 0,
            total_capacity: 0,
            current_load: 0,
            pending_tasks: 0,
            claimed_tasks: 0,
            completed_tasks: 0,
            failed_tasks: 0,
            started_tasks: 0,
            running_workflows: 0,
            pending_workflows: 0,
            completed_workflows: 0,
            failed_workflows: 0,
            started_workflows: 0,
            dlq_size: 0,
        })
    }

    /// Cancel a workflow
    async fn cancel_workflow(&self, _workflow_id: Uuid) -> Result<(), StoreError> {
        Ok(())
    }

    /// Continue a workflow as a new workflow (history rollover).
    ///
    /// Creates a new workflow from the given snapshot state, marks the old
    /// workflow as `ContinuedAsNew` with a reference to the new workflow,
    /// and archives (deletes) old event history and snapshots.
    ///
    /// Returns the new workflow ID.
    async fn continue_as_new(
        &self,
        _old_workflow_id: Uuid,
        _workflow_type: &str,
        _input: serde_json::Value,
        _snapshot_data: Vec<u8>,
    ) -> Result<Uuid, StoreError> {
        Err(StoreError::Database(
            "continue_as_new not supported by this store".to_string(),
        ))
    }

    /// Get workflow events
    async fn get_workflow_events(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowEventInfo>, StoreError> {
        // Default implementation uses load_events
        let events = self.load_events(workflow_id).await?;
        Ok(events
            .into_iter()
            .map(|(seq, event)| WorkflowEventInfo {
                id: seq as i64, // Use sequence as id fallback
                workflow_id,
                sequence_num: seq,
                event_type: event_type_name(&event).to_string(),
                event_data: serde_json::to_value(&event).unwrap_or_default(),
                created_at: Utc::now(), // Not available in base events
            })
            .collect())
    }

    /// Count workflow events without materializing them.
    /// Uses SELECT COUNT(*) in PostgreSQL; falls back to load_events().len() by default.
    async fn count_workflow_events(&self, workflow_id: Uuid) -> Result<i64, StoreError> {
        let events = self.load_events(workflow_id).await?;
        Ok(events.len() as i64)
    }

    // =========================================================================
    // Schedule Operations (optional, default no-op)
    // =========================================================================

    /// Create a new schedule
    async fn create_schedule(&self, _schedule: CreateScheduleRow) -> Result<Uuid, StoreError> {
        Ok(Uuid::nil())
    }

    /// Get a schedule by ID
    async fn get_schedule(&self, _id: Uuid) -> Result<ScheduleRow, StoreError> {
        Err(StoreError::ScheduleNotFound(Uuid::nil()))
    }

    /// List schedules with filtering and pagination
    async fn list_schedules(
        &self,
        _filter: ScheduleFilter,
        _pagination: Pagination,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        Ok(vec![])
    }

    /// Count schedules matching filter
    async fn count_schedules(&self, _filter: ScheduleFilter) -> Result<u64, StoreError> {
        Ok(0)
    }

    /// Update a schedule
    async fn update_schedule(&self, _id: Uuid, _update: UpdateSchedule) -> Result<(), StoreError> {
        Ok(())
    }

    /// Delete a schedule
    async fn delete_schedule(&self, _id: Uuid) -> Result<(), StoreError> {
        Ok(())
    }

    // =========================================================================
    // Scheduler Operations (for DurableScheduler component)
    // =========================================================================

    /// Claim due schedules for processing (uses SKIP LOCKED for multi-instance)
    /// Returns schedules with next_trigger_at <= now
    async fn claim_due_schedules(
        &self,
        _scheduler_id: &str,
        _limit: u32,
    ) -> Result<Vec<ScheduleRow>, StoreError> {
        Ok(vec![])
    }

    /// Update next trigger time after successful trigger
    async fn update_next_trigger(&self, _id: Uuid, _next: DateTime<Utc>) -> Result<(), StoreError> {
        Ok(())
    }

    /// Skip a schedule trigger (e.g., max_concurrent reached)
    async fn skip_schedule_trigger(&self, _id: Uuid) -> Result<(), StoreError> {
        Ok(())
    }

    /// Release a claimed schedule (e.g., on scheduler shutdown)
    async fn release_schedule(&self, _id: Uuid) -> Result<(), StoreError> {
        Ok(())
    }

    // =========================================================================
    // Schedule Execution Operations
    // =========================================================================

    /// Create a schedule execution record
    async fn create_schedule_execution(
        &self,
        _schedule_id: Uuid,
        _scheduled_at: DateTime<Utc>,
    ) -> Result<Uuid, StoreError> {
        Ok(Uuid::nil())
    }

    /// Get a schedule execution by ID
    async fn get_schedule_execution(&self, _id: Uuid) -> Result<ScheduleExecutionRow, StoreError> {
        Err(StoreError::ScheduleExecutionNotFound(Uuid::nil()))
    }

    /// Complete a schedule execution successfully
    async fn complete_schedule_execution(
        &self,
        _execution_id: Uuid,
        _target_id: Uuid,
        _is_workflow: bool,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    /// Fail a schedule execution
    async fn fail_schedule_execution(
        &self,
        _execution_id: Uuid,
        _error: &str,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    /// Skip a schedule execution
    async fn skip_schedule_execution(
        &self,
        _execution_id: Uuid,
        _reason: &str,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    /// List executions for a schedule
    async fn list_schedule_executions(
        &self,
        _filter: ScheduleExecutionFilter,
        _pagination: Pagination,
    ) -> Result<Vec<ScheduleExecutionRow>, StoreError> {
        Ok(vec![])
    }

    /// Count running executions for a schedule (for max_concurrent check)
    async fn count_running_executions(&self, _schedule_id: Uuid) -> Result<u32, StoreError> {
        Ok(0)
    }

    /// Get schedule statistics
    async fn get_schedule_stats(&self, _schedule_id: Uuid) -> Result<ScheduleStats, StoreError> {
        Ok(ScheduleStats::default())
    }

    // =========================================================================
    // Scheduler Instance Operations
    // =========================================================================

    /// Register a scheduler instance
    async fn register_scheduler_instance(
        &self,
        _instance: SchedulerInstanceInfo,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    /// Update scheduler instance heartbeat
    async fn heartbeat_scheduler_instance(
        &self,
        _instance_id: &str,
        _schedules_processed: u64,
    ) -> Result<(), StoreError> {
        Ok(())
    }

    /// List scheduler instances
    async fn list_scheduler_instances(&self) -> Result<Vec<SchedulerInstanceInfo>, StoreError> {
        Ok(vec![])
    }

    /// Deregister a scheduler instance
    async fn deregister_scheduler_instance(&self, _instance_id: &str) -> Result<(), StoreError> {
        Ok(())
    }
}

/// Circuit breaker state
#[derive(Debug, Clone)]
pub struct CircuitBreakerState {
    pub key: String,
    pub state: crate::reliability::CircuitState,
    pub failure_count: u32,
    pub success_count: u32,
    pub last_failure_at: Option<chrono::DateTime<chrono::Utc>>,
    pub opened_at: Option<chrono::DateTime<chrono::Utc>>,
    pub half_open_at: Option<chrono::DateTime<chrono::Utc>>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// =========================================================================
// Schedule Types
// =========================================================================

/// Target type for a schedule
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleTargetType {
    Workflow,
    Activity,
}

impl std::fmt::Display for ScheduleTargetType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Workflow => write!(f, "workflow"),
            Self::Activity => write!(f, "activity"),
        }
    }
}

/// Schedule execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

impl std::fmt::Display for ScheduleExecutionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

/// Schedule row from database
#[derive(Debug, Clone)]
pub struct ScheduleRow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub cron_expression: String,
    pub timezone: String,
    pub target_type: ScheduleTargetType,
    pub target_name: String,
    pub target_input: serde_json::Value,
    pub enabled: bool,
    pub max_concurrent: Option<u32>,
    pub catch_up_missed: bool,
    pub max_catch_up: Option<u32>,
    pub retry_policy: Option<serde_json::Value>,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub next_trigger_at: Option<DateTime<Utc>>,
    pub claimed_by: Option<String>,
    pub claimed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a schedule
#[derive(Debug, Clone)]
pub struct CreateScheduleRow {
    pub name: String,
    pub description: Option<String>,
    pub cron_expression: String,
    pub timezone: String,
    pub target_type: ScheduleTargetType,
    pub target_name: String,
    pub target_input: serde_json::Value,
    pub enabled: bool,
    pub max_concurrent: Option<u32>,
    pub catch_up_missed: bool,
    pub max_catch_up: Option<u32>,
    pub retry_policy: Option<serde_json::Value>,
    pub next_trigger_at: Option<DateTime<Utc>>,
}

/// Input for updating a schedule
#[derive(Debug, Clone, Default)]
pub struct UpdateSchedule {
    pub name: Option<String>,
    pub description: UpdateField<String>,
    pub cron_expression: Option<String>,
    pub timezone: Option<String>,
    pub target_type: Option<ScheduleTargetType>,
    pub target_name: Option<String>,
    pub target_input: Option<serde_json::Value>,
    pub enabled: Option<bool>,
    pub max_concurrent: UpdateField<u32>,
    pub catch_up_missed: Option<bool>,
    pub max_catch_up: UpdateField<u32>,
    pub retry_policy: UpdateField<serde_json::Value>,
    pub next_trigger_at: UpdateField<DateTime<Utc>>,
}

/// Filter for listing schedules
#[derive(Debug, Clone, Default)]
pub struct ScheduleFilter {
    pub enabled: Option<bool>,
    pub target_type: Option<ScheduleTargetType>,
}

/// Schedule execution row from database
#[derive(Debug, Clone)]
pub struct ScheduleExecutionRow {
    pub id: Uuid,
    pub schedule_id: Uuid,
    pub scheduled_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub status: ScheduleExecutionStatus,
    pub workflow_id: Option<Uuid>,
    pub task_id: Option<Uuid>,
    pub error: Option<String>,
    pub duration_ms: Option<i32>,
    pub created_at: DateTime<Utc>,
}

/// Schedule statistics
#[derive(Debug, Clone, Default)]
pub struct ScheduleStats {
    pub total_executions: u64,
    pub successful_executions: u64,
    pub failed_executions: u64,
    pub skipped_executions: u64,
    pub avg_duration_ms: Option<u64>,
    pub last_execution_status: Option<ScheduleExecutionStatus>,
}

/// Filter for listing schedule executions
#[derive(Debug, Clone, Default)]
pub struct ScheduleExecutionFilter {
    pub schedule_id: Option<Uuid>,
    pub status: Option<ScheduleExecutionStatus>,
}

/// Scheduler instance info
#[derive(Debug, Clone)]
pub struct SchedulerInstanceInfo {
    pub instance_id: String,
    pub started_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub schedules_processed: u64,
    pub hostname: Option<String>,
    pub version: Option<String>,
}
