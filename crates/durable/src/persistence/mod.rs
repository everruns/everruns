//! Persistence layer for durable execution
//!
//! This module provides:
//! - Focused store traits ([`EventLog`], [`TaskQueue`], [`SignalStore`], [`WorkerRegistry`],
//!   [`DeadLetters`], [`CircuitBreakers`], [`Schedules`], [`DurableAdmin`]) and the
//!   [`WorkflowEventStore`] umbrella that combines them
//! - [`InMemoryWorkflowEventStore`] for testing
//! - [`PostgresWorkflowEventStore`] for production

mod db_failure;
mod memory;
mod postgres;
mod store;

pub(crate) use db_failure::log_database_failure;

pub use memory::{InMemoryWorkflowEventStore, WorkflowEndSubscription};
pub use postgres::PostgresWorkflowEventStore;
pub use store::{
    CapacitySnapshot, CircuitBreakerState, CircuitBreakers, ClaimedTask, CreateScheduleRow,
    DEFAULT_MAX_PENDING_TASKS_PER_WORKFLOW, DEFAULT_NO_PROGRESS_SEAL_THRESHOLD,
    DEFAULT_SNAPSHOT_INTERVAL, DeadLetters, DeadTaskInfo, DlqEntry, DlqFilter, DurableAdmin,
    EventLog, HeartbeatResponse, Pagination, ReclaimResult, RunStart, ScheduleExecutionFilter,
    ScheduleExecutionRow, ScheduleExecutionStatus, ScheduleFilter, ScheduleRow, ScheduleStats,
    ScheduleTargetType, SchedulerInstanceInfo, Schedules, SealedTaskInfo, SignalStore, StoreError,
    SystemHealth, TaskDefinition, TaskFailureOutcome, TaskFilter, TaskInfo, TaskQueue, TaskStatus,
    TraceContext, UpdateSchedule, WORKER_HEARTBEAT_TIMEOUT_SECS, WorkerFilter, WorkerInfo,
    WorkerRegistry, WorkflowEventInfo, WorkflowEventStore, WorkflowFilter, WorkflowInfo,
    WorkflowInfoExtended, WorkflowSnapshot, WorkflowStatus, event_type_name,
    no_progress_seal_threshold_from_env, snapshot_interval_from_env,
};
