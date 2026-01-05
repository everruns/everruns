//! WorkflowEventStore trait definition

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::workflow::{ActivityOptions, WorkflowEvent, WorkflowSignal};

/// Error type for store operations
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Workflow not found
    #[error("workflow not found: {0}")]
    WorkflowNotFound(Uuid),

    /// Task not found
    #[error("task not found: {0}")]
    TaskNotFound(Uuid),

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
}

impl std::fmt::Display for WorkflowStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
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

/// Definition of a task to be enqueued
#[derive(Debug, Clone)]
pub struct TaskDefinition {
    pub workflow_id: Uuid,
    pub activity_id: String,
    pub activity_type: String,
    pub input: serde_json::Value,
    pub options: ActivityOptions,
}

/// A task that has been claimed by a worker
#[derive(Debug, Clone)]
pub struct ClaimedTask {
    pub id: Uuid,
    pub workflow_id: Uuid,
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

/// Worker information
#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub id: String,
    pub worker_group: String,
    pub activity_types: Vec<String>,
    pub max_concurrency: u32,
    pub current_load: u32,
    pub status: String,
    pub accepting_tasks: bool,
    pub started_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
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
    pub workflow_id: Uuid,
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
    async fn complete_task(
        &self,
        task_id: Uuid,
        result: serde_json::Value,
    ) -> Result<(), StoreError>;

    /// Fail a task (may requeue or send to DLQ)
    async fn fail_task(&self, task_id: Uuid, error: &str)
        -> Result<TaskFailureOutcome, StoreError>;

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

    /// Deregister a worker
    async fn deregister_worker(&self, _worker_id: &str) -> Result<(), StoreError> {
        Ok(())
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
    // Circuit Breaker Operations (optional, default no-op)
    // =========================================================================

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
