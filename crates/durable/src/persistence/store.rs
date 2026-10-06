//! Store trait definitions: focused traits plus the `WorkflowEventStore` umbrella

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

/// Outcome of [`EventLog::start_run_with_task`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStart {
    /// A new run started and its first task was enqueued. `created` is true
    /// when this call also created the workflow.
    Started { task_id: Uuid, created: bool },
    /// A run is already active; nothing changed.
    Active,
}

/// A task that has been claimed by a worker
#[derive(Debug, Clone, Default)]
pub struct ClaimedTask {
    pub id: Uuid,
    pub workflow_id: Option<Uuid>,
    pub activity_id: String,
    pub activity_type: String,
    pub input: serde_json::Value,
    pub options: ActivityOptions,
    pub attempt: u32,
    pub max_attempts: u32,
    /// Status of the task's workflow, read in the same statement as the
    /// claim, so a worker can skip work for a cancelled or finished workflow
    /// without a second read. `None` for standalone tasks.
    pub workflow_status: Option<WorkflowStatus>,
}

/// How [`TaskQueue::enqueue_claimed_task`] enqueued a task.
#[derive(Debug, Clone)]
pub enum Enqueued {
    /// Enqueued claimed by the requesting worker, which runs it now.
    Claimed(Box<ClaimedTask>),
    /// Enqueued pending, with this id, for whichever worker claims it.
    Queued(Uuid),
}

impl Enqueued {
    /// The task's id.
    pub fn task_id(&self) -> Uuid {
        match self {
            Self::Claimed(task) => task.id,
            Self::Queued(task_id) => *task_id,
        }
    }

    /// The claimed task, when it was enqueued claimed.
    pub fn into_claimed(self) -> Option<ClaimedTask> {
        match self {
            Self::Claimed(task) => Some(*task),
            Self::Queued(_) => None,
        }
    }
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

/// Information about a task that was marked as dead during stale reclamation.
#[derive(Debug, Clone)]
pub struct DeadTaskInfo {
    pub task_id: Uuid,
    pub workflow_id: Option<Uuid>,
    pub activity_id: String,
    pub activity_type: String,
    /// Serialized task input, so the application can recover its own context.
    pub input: serde_json::Value,
    pub last_error: Option<String>,
}

/// Information about a task that the no-progress guard sealed during stale
/// reclamation: its workflow recorded no new event across `N` consecutive
/// recoveries, so retrying it again would only crash-loop (EVE-534).
///
/// A sealed task is terminal and non-retryable: the reclaim path marks it dead
/// (routing it to the DLQ) instead of returning it to `pending`. What a seal
/// means to the application (a user-facing event, a status change) is decided
/// by the consumer of [`ReclaimResult::sealed_tasks`].
#[derive(Debug, Clone)]
pub struct SealedTaskInfo {
    pub task_id: Uuid,
    pub workflow_id: Option<Uuid>,
    pub activity_id: String,
    pub activity_type: String,
    /// The task's serialized input, so consumers can recover their own context
    /// for whatever a seal means to them.
    pub input: serde_json::Value,
    /// Stable seal-reason wire string (always `"no_progress"` here: the reclaim
    /// path's only seal is the no-progress guard).
    pub reason: String,
    /// Number of consecutive no-progress recoveries that triggered the seal.
    pub no_progress_count: u32,
}

/// Default number of consecutive no-progress recoveries before a task is sealed.
/// Override with `DURABLE_NO_PROGRESS_SEAL_THRESHOLD`; see
/// [`no_progress_seal_threshold_from_env`]. EVE-534.
pub const DEFAULT_NO_PROGRESS_SEAL_THRESHOLD: u32 = 3;

/// Read the no-progress seal threshold from the environment, falling back to
/// [`DEFAULT_NO_PROGRESS_SEAL_THRESHOLD`]. A value of 0 is coerced to 1 so the
/// guard can never be disabled into an infinite crash-loop.
pub fn no_progress_seal_threshold_from_env() -> u32 {
    std::env::var("DURABLE_NO_PROGRESS_SEAL_THRESHOLD")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(DEFAULT_NO_PROGRESS_SEAL_THRESHOLD)
        .max(1)
}

/// Result of stale task reclamation.
#[derive(Debug, Clone, Default)]
pub struct ReclaimResult {
    /// Tasks that were reclaimed (returned to pending for retry).
    pub reclaimed_ids: Vec<Uuid>,
    /// Tasks that were marked as dead (exhausted all retries).
    pub dead_tasks: Vec<DeadTaskInfo>,
    /// Tasks that were sealed for making no forward progress across repeated
    /// recoveries (EVE-534). These are terminal/non-retryable (routed to DLQ).
    pub sealed_tasks: Vec<SealedTaskInfo>,
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
    /// The task queue it was enqueued to; `None` is the default queue.
    pub queue: Option<String>,
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

impl WorkerInfo {
    /// An active worker that accepts `activity_types`, ready for
    /// [`WorkerRegistry::register_worker`].
    ///
    /// Both stores hand tasks only to a registered worker that is not
    /// draining, so register before claiming.
    ///
    /// ```
    /// use everruns_durable::{InMemoryWorkflowEventStore, TaskQueue, WorkerInfo, WorkerRegistry};
    ///
    /// # #[tokio::main]
    /// # async fn main() -> Result<(), everruns_durable::StoreError> {
    /// let store = InMemoryWorkflowEventStore::new();
    /// store.register_worker(WorkerInfo::new("worker-1", ["send_email"])).await?;
    /// let claimed = store.claim_task("worker-1", &["send_email".into()], 10).await?;
    /// assert!(claimed.is_empty()); // registered, nothing queued yet
    /// # Ok(()) }
    /// ```
    pub fn new<I, S>(id: impl Into<String>, activity_types: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let now = Utc::now();
        Self {
            id: id.into(),
            worker_group: None,
            activity_types: activity_types.into_iter().map(Into::into).collect(),
            max_concurrency: 10,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: now,
            last_heartbeat_at: now,
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        }
    }
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

/// Workflow lifecycle and the append-only event log.
///
/// Covers workflow creation and status, optimistic-concurrency event appends,
/// replay loads, replay snapshots, terminal transitions (fail, cancel,
/// continue-as-new) and the atomic new-run claim. This is the core of the
/// engine: anything that executes or replays a workflow needs it.
///
/// Every method must be implemented explicitly except the derived ones
/// (`count_events`, `count_events_after`, `load_events_after`), which are
/// correct but materialize history; stores should override them for efficiency.
#[async_trait]
pub trait EventLog: Send + Sync + 'static {
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
        // Derived: filter from load_events (implementations should override for efficiency)
        let all = self.load_events(workflow_id).await?;
        Ok(all
            .into_iter()
            .filter(|(seq, _)| *seq > after_sequence)
            .collect())
    }

    /// Save a workflow state snapshot at the given sequence number.
    ///
    /// The snapshot_data is opaque bytes (typically JSON-serialized workflow state).
    /// Implementations should use UPSERT semantics for idempotency.
    async fn save_snapshot(
        &self,
        workflow_id: Uuid,
        sequence_num: i32,
        snapshot_data: Vec<u8>,
    ) -> Result<(), StoreError>;

    /// Load the latest snapshot for a workflow, if any.
    ///
    /// Returns None if no snapshots exist for this workflow.
    async fn load_latest_snapshot(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<WorkflowSnapshot>, StoreError>;

    /// Delete all snapshots for a workflow (cleanup on workflow deletion).
    async fn delete_snapshots(&self, workflow_id: Uuid) -> Result<(), StoreError>;

    /// Update workflow status
    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        result: Option<serde_json::Value>,
        error: Option<crate::workflow::WorkflowError>,
    ) -> Result<(), StoreError>;

    /// Atomically transition a running workflow to failed.
    ///
    /// Returns true only to the caller that won the terminal transition. This
    /// elects one owner for external terminal lifecycle effects when a worker
    /// failure races stale-task reclamation.
    async fn try_fail_workflow(
        &self,
        workflow_id: Uuid,
        error: crate::workflow::WorkflowError,
    ) -> Result<bool, StoreError>;

    /// Atomically start a new run of a long-lived workflow, unless one is
    /// already active.
    ///
    /// A workflow instance can be reused for successive runs (for example one
    /// per inbound request). This transitions it from any terminal status (or
    /// pending) back to Running and clears its result, error and timestamps.
    /// Returns true if the claim succeeded, false if a run is still active:
    /// the workflow is Running or one of its tasks is claimed by a worker.
    ///
    /// Also cancels any stale pending tasks left by the previous run, in the
    /// same atomic operation. Safe under horizontal scaling: only one caller
    /// wins the claim.
    async fn try_start_new_run(&self, workflow_id: Uuid) -> Result<bool, StoreError>;

    /// Start a run of `workflow_id` and enqueue its first task, atomically.
    ///
    /// The one-step form of "create or `try_start_new_run`, then
    /// `enqueue_task`" that callers starting a run on a long-lived workflow
    /// need. Doing it in one step means a run is never left Running without a
    /// task, and concurrent callers need no lock of their own: exactly one
    /// wins, the rest get [`RunStart::Active`] and can signal the run instead.
    ///
    /// - Unknown workflow: create it Running with `WorkflowStarted` and
    ///   `ActivityScheduled` events and enqueue `task`.
    /// - Known workflow with no active run: start a new run exactly as
    ///   `try_start_new_run` does (stale pending tasks are cancelled) and
    ///   enqueue `task`.
    /// - Active run (Running, or a task claimed): change nothing and return
    ///   [`RunStart::Active`].
    ///
    /// `task.workflow_id` is ignored; the task always belongs to `workflow_id`.
    async fn start_run_with_task(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        task: TaskDefinition,
    ) -> Result<RunStart, StoreError>;

    /// Cancel a workflow
    async fn cancel_workflow(&self, workflow_id: Uuid) -> Result<(), StoreError>;

    /// Continue a workflow as a new workflow (history rollover).
    ///
    /// Creates a new workflow from the given snapshot state, marks the old
    /// workflow as `ContinuedAsNew` with a reference to the new workflow,
    /// and archives (deletes) old event history and snapshots.
    ///
    /// Returns the new workflow ID.
    async fn continue_as_new(
        &self,
        old_workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        snapshot_data: Vec<u8>,
    ) -> Result<Uuid, StoreError>;
}

/// Activity task queue: enqueue, claim, heartbeat and settle tasks.
///
/// Needed by workers (claim/heartbeat/complete/fail), by the engine that
/// schedules activities (enqueue, cancel pending) and by the stale-task
/// reaper (`reclaim_stale_tasks`). `fail_task` is derived from
/// `fail_task_with_retry`; everything else must be implemented explicitly.
#[async_trait]
pub trait TaskQueue: Send + Sync + 'static {
    /// Enqueue an activity task
    async fn enqueue_task(&self, task: TaskDefinition) -> Result<Uuid, StoreError>;

    /// Enqueue a task already claimed by `worker_id`: the same as
    /// [`enqueue_task`](Self::enqueue_task) followed by a claim of that task
    /// by `worker_id`, without the queue in between.
    ///
    /// A worker that just finished one step of a workflow runs the next step
    /// itself this way, without waiting for a wakeup and a claim. The task is
    /// an ordinary claimed row: it heartbeats, retries and is reclaimed when
    /// stale like any other.
    ///
    /// Returns [`Enqueued::Queued`] when the task was enqueued as pending
    /// instead, for any worker to claim: the worker is not registered or is
    /// draining, or the task is delayed or deduplicated. The default always
    /// does that.
    async fn enqueue_claimed_task(
        &self,
        task: TaskDefinition,
        worker_id: &str,
    ) -> Result<Enqueued, StoreError> {
        let _ = worker_id;
        self.enqueue_task(task).await.map(Enqueued::Queued)
    }

    /// Claim tasks of the default queue for execution; see
    /// [`claim_queue_tasks`](Self::claim_queue_tasks).
    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        self.claim_queue_tasks(worker_id, None, activity_types, max_tasks)
            .await
    }

    /// Claim up to `max_tasks` pending tasks of `activity_types` from one
    /// task queue: the named one, or the default queue for `None`
    /// ([`ActivityOptions::queue`](crate::ActivityOptions::queue)). A claim
    /// never takes another queue's tasks.
    ///
    /// Uses SELECT FOR UPDATE SKIP LOCKED for efficient concurrent claiming.
    async fn claim_queue_tasks(
        &self,
        worker_id: &str,
        queue: Option<&str>,
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

    /// Fail a task using its normal retry policy.
    async fn fail_task(
        &self,
        task_id: Uuid,
        error: &str,
    ) -> Result<TaskFailureOutcome, StoreError> {
        self.fail_task_with_retry(task_id, error, true).await
    }

    /// Fail a task, optionally bypassing retries for deterministic failures.
    async fn fail_task_with_retry(
        &self,
        task_id: Uuid,
        error: &str,
        retryable: bool,
    ) -> Result<TaskFailureOutcome, StoreError>;

    /// Cancel all pending (unclaimed) tasks for a workflow.
    ///
    /// Returns the number of tasks cancelled. Does NOT affect claimed or
    /// completed tasks — only pending ones still in the queue.
    async fn cancel_pending_tasks_for_workflow(&self, workflow_id: Uuid)
    -> Result<u64, StoreError>;

    /// Get task info by ID
    async fn get_task(&self, task_id: Uuid) -> Result<TaskInfo, StoreError>;

    /// Find and reclaim stale tasks (no heartbeat)
    async fn reclaim_stale_tasks(
        &self,
        stale_threshold: Duration,
    ) -> Result<ReclaimResult, StoreError>;

    /// List tasks with filtering and pagination
    async fn list_tasks(
        &self,
        filter: TaskFilter,
        pagination: Pagination,
    ) -> Result<Vec<TaskInfo>, StoreError>;
}

/// Per-workflow signal inbox.
///
/// Needed by the engine to deliver and consume signals (application
/// messages, cancellation, timers). `consume_pending_signals` has a non-atomic derived
/// default; stores should override it with an atomic implementation.
#[async_trait]
pub trait SignalStore: Send + Sync + 'static {
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

    /// Atomically get and consume all pending signals for a workflow.
    ///
    /// The derived default calls `get_pending_signals` + `mark_signals_processed`,
    /// which is not atomic; stores should override it to avoid races.
    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError> {
        let signals = self.get_pending_signals(workflow_id).await?;
        if !signals.is_empty() {
            self.mark_signals_processed(workflow_id, signals.len())
                .await?;
        }
        Ok(signals)
    }

    /// Atomically get and consume pending signals of one type for a workflow.
    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<WorkflowSignal>, StoreError>;
}

/// Worker registration, liveness and capacity.
///
/// Needed by workers (register, heartbeat, capacity snapshot for fair-share
/// claiming, deregister) and by operators (list, drain, resume). A worker
/// process needs only `TaskQueue + SignalStore + WorkerRegistry`.
#[async_trait]
pub trait WorkerRegistry: Send + Sync + 'static {
    /// Register a worker
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError>;

    /// Update worker heartbeat and load
    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError>;

    /// Get all active workers
    async fn list_workers(&self, filter: WorkerFilter) -> Result<Vec<WorkerInfo>, StoreError>;

    /// Deregister a worker and reclaim all tasks claimed by it
    /// Returns the number of tasks that were reclaimed
    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError>;

    /// Get a snapshot of total system worker capacity.
    /// Used by workers to compute fair-share claim limits.
    async fn get_capacity_snapshot(&self) -> Result<CapacitySnapshot, StoreError>;

    /// Drain a worker (set status to draining, stop accepting new tasks)
    async fn drain_worker(&self, worker_id: &str) -> Result<(), StoreError>;

    /// Resume a draining worker (set status back to active, start accepting new tasks)
    async fn resume_worker(&self, worker_id: &str) -> Result<(), StoreError>;
}

/// Dead letter queue for tasks that exhausted their retries.
///
/// Needed by the task-failure path (`move_to_dlq`) and by operators who
/// inspect and requeue dead tasks.
#[async_trait]
pub trait DeadLetters: Send + Sync + 'static {
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
}

/// Persistent circuit breaker state shared across workers.
///
/// Backs `DistributedCircuitBreaker` in `reliability/` and the admin
/// force-open/close operations. Not yet wired into production call paths.
#[async_trait]
pub trait CircuitBreakers: Send + Sync + 'static {
    /// Create a circuit breaker
    async fn create_circuit_breaker(
        &self,
        key: &str,
        config: &crate::reliability::CircuitBreakerConfig,
    ) -> Result<(), StoreError>;

    /// Get circuit breaker state
    async fn get_circuit_breaker(
        &self,
        key: &str,
    ) -> Result<Option<CircuitBreakerState>, StoreError>;

    /// Update circuit breaker state
    async fn update_circuit_breaker(
        &self,
        key: &str,
        state: crate::reliability::CircuitState,
        failure_count: u32,
        success_count: u32,
    ) -> Result<(), StoreError>;

    /// List all circuit breakers
    async fn list_circuit_breakers(&self) -> Result<Vec<CircuitBreakerState>, StoreError>;

    /// Force a circuit breaker to open (admin operation)
    async fn force_open_circuit_breaker(&self, key: &str) -> Result<(), StoreError>;

    /// Force a circuit breaker to close (admin operation)
    async fn force_close_circuit_breaker(&self, key: &str) -> Result<(), StoreError>;

    /// Delete a circuit breaker (reset to default)
    async fn delete_circuit_breaker(&self, key: &str) -> Result<(), StoreError>;
}

/// Cron/interval schedules, their executions and scheduler instances.
///
/// Needed by the `DurableScheduler` component (claim due schedules, record
/// executions, instance heartbeats) and by the schedule management API.
#[async_trait]
pub trait Schedules: Send + Sync + 'static {
    /// Create a new schedule
    async fn create_schedule(&self, schedule: CreateScheduleRow) -> Result<Uuid, StoreError>;

    /// Get a schedule by ID
    async fn get_schedule(&self, id: Uuid) -> Result<ScheduleRow, StoreError>;

    /// List schedules with filtering and pagination
    async fn list_schedules(
        &self,
        filter: ScheduleFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleRow>, StoreError>;

    /// Count schedules matching filter
    async fn count_schedules(&self, filter: ScheduleFilter) -> Result<u64, StoreError>;

    /// Update a schedule
    async fn update_schedule(&self, id: Uuid, _update: UpdateSchedule) -> Result<(), StoreError>;

    /// Delete a schedule
    async fn delete_schedule(&self, id: Uuid) -> Result<(), StoreError>;

    /// Claim due schedules for processing (uses SKIP LOCKED for multi-instance)
    /// Returns schedules with next_trigger_at <= now
    async fn claim_due_schedules(
        &self,
        scheduler_id: &str,
        limit: u32,
    ) -> Result<Vec<ScheduleRow>, StoreError>;

    /// Update next trigger time after successful trigger
    async fn update_next_trigger(&self, id: Uuid, _next: DateTime<Utc>) -> Result<(), StoreError>;

    /// Skip a schedule trigger (e.g., max_concurrent reached)
    async fn skip_schedule_trigger(&self, id: Uuid) -> Result<(), StoreError>;

    /// Release a claimed schedule (e.g., on scheduler shutdown)
    async fn release_schedule(&self, id: Uuid) -> Result<(), StoreError>;

    /// Create a schedule execution record
    async fn create_schedule_execution(
        &self,
        schedule_id: Uuid,
        scheduled_at: DateTime<Utc>,
    ) -> Result<Uuid, StoreError>;

    /// Get a schedule execution by ID
    async fn get_schedule_execution(&self, id: Uuid) -> Result<ScheduleExecutionRow, StoreError>;

    /// Complete a schedule execution successfully
    async fn complete_schedule_execution(
        &self,
        execution_id: Uuid,
        target_id: Uuid,
        is_workflow: bool,
    ) -> Result<(), StoreError>;

    /// Fail a schedule execution
    async fn fail_schedule_execution(
        &self,
        execution_id: Uuid,
        error: &str,
    ) -> Result<(), StoreError>;

    /// Skip a schedule execution
    async fn skip_schedule_execution(
        &self,
        execution_id: Uuid,
        reason: &str,
    ) -> Result<(), StoreError>;

    /// List executions for a schedule
    async fn list_schedule_executions(
        &self,
        filter: ScheduleExecutionFilter,
        pagination: Pagination,
    ) -> Result<Vec<ScheduleExecutionRow>, StoreError>;

    /// Count running executions for a schedule (for max_concurrent check)
    async fn count_running_executions(&self, schedule_id: Uuid) -> Result<u32, StoreError>;

    /// Get schedule statistics
    async fn get_schedule_stats(&self, schedule_id: Uuid) -> Result<ScheduleStats, StoreError>;

    /// Register a scheduler instance
    async fn register_scheduler_instance(
        &self,
        instance: SchedulerInstanceInfo,
    ) -> Result<(), StoreError>;

    /// Update scheduler instance heartbeat
    async fn heartbeat_scheduler_instance(
        &self,
        instance_id: &str,
        schedules_processed: u64,
    ) -> Result<(), StoreError>;

    /// List scheduler instances
    async fn list_scheduler_instances(&self) -> Result<Vec<SchedulerInstanceInfo>, StoreError>;

    /// Deregister a scheduler instance
    async fn deregister_scheduler_instance(&self, instance_id: &str) -> Result<(), StoreError>;
}

/// Read-mostly dashboard and admin queries across workflows and system health.
///
/// Needed by the HTTP API and operator tooling. Workers and the engine do not
/// need it. It requires [`EventLog`] because `get_workflow_events` and `count_workflow_events` have derived
/// defaults over `EventLog::load_events`; stores should override them.
#[async_trait]
pub trait DurableAdmin: EventLog + Send + Sync + 'static {
    /// Count active (non-terminal) workflows
    async fn count_active_workflows(&self) -> Result<i64, StoreError>;

    /// List workflows with filtering and pagination
    async fn list_workflows(
        &self,
        filter: WorkflowFilter,
        pagination: Pagination,
    ) -> Result<Vec<WorkflowInfoExtended>, StoreError>;

    /// Direct lookup of a single workflow by id, returning the same
    /// `WorkflowInfoExtended` shape as `list_workflows`. Returns `Ok(None)`
    /// when the workflow does not exist.
    ///
    /// EVE-455: API handlers must use this method instead of scanning the
    /// first page of `list_workflows`, which silently returns 404 for any
    /// workflow older than the page once a deployment crosses the page
    /// limit. Implementations must do a direct id lookup.
    async fn get_workflow_extended(
        &self,
        workflow_id: Uuid,
    ) -> Result<Option<WorkflowInfoExtended>, StoreError>;

    /// Get workflow events
    async fn get_workflow_events(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowEventInfo>, StoreError> {
        // Derived from load_events
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

    /// Get system health summary
    async fn get_system_health(&self) -> Result<SystemHealth, StoreError>;
}

/// Umbrella for a complete durable store.
///
/// Combines every focused store trait. It is blanket-implemented for any type
/// that implements all of them, so `Arc<dyn WorkflowEventStore>` and
/// `T: WorkflowEventStore` keep working and the supertrait methods are callable
/// through it. Consumers that need only a slice should bound on the focused
/// trait instead: a worker needs `TaskQueue + SignalStore + WorkerRegistry`.
pub trait WorkflowEventStore:
    EventLog
    + TaskQueue
    + SignalStore
    + WorkerRegistry
    + DeadLetters
    + CircuitBreakers
    + Schedules
    + DurableAdmin
    + Send
    + Sync
    + 'static
{
}

impl<T> WorkflowEventStore for T where
    T: EventLog
        + TaskQueue
        + SignalStore
        + WorkerRegistry
        + DeadLetters
        + CircuitBreakers
        + Schedules
        + DurableAdmin
        + Send
        + Sync
        + 'static
{
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
