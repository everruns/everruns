//! Persistence layer for durable execution
//!
//! This module provides:
//! - [`WorkflowEventStore`] trait for workflow and event persistence
//! - [`InMemoryWorkflowEventStore`] for testing
//! - [`PostgresWorkflowEventStore`] for production

mod memory;
mod postgres;
mod store;

pub use memory::InMemoryWorkflowEventStore;
pub use postgres::PostgresWorkflowEventStore;
pub use store::{
    CapacitySnapshot, CircuitBreakerState, ClaimedTask, CreateScheduleRow,
    DEFAULT_MAX_PENDING_TASKS_PER_WORKFLOW, DEFAULT_NO_PROGRESS_SEAL_THRESHOLD,
    DEFAULT_SNAPSHOT_INTERVAL, DeadTaskInfo, DlqEntry, DlqFilter, HeartbeatResponse, Pagination,
    ReclaimResult, ScheduleExecutionFilter, ScheduleExecutionRow, ScheduleExecutionStatus,
    ScheduleFilter, ScheduleRow, ScheduleStats, ScheduleTargetType, SchedulerInstanceInfo,
    SealedTaskInfo, StoreError, SystemHealth, TaskDefinition, TaskFailureOutcome, TaskFilter,
    TaskInfo, TaskStatus, TraceContext, UpdateSchedule, WORKER_HEARTBEAT_TIMEOUT_SECS,
    WorkerFilter, WorkerInfo, WorkflowEventInfo, WorkflowEventStore, WorkflowFilter, WorkflowInfo,
    WorkflowInfoExtended, WorkflowSnapshot, WorkflowStatus, event_type_name,
    no_progress_seal_threshold_from_env, snapshot_interval_from_env,
};
