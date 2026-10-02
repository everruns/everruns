#![doc = include_str!("../README.md")]

pub mod activity;
pub mod engine;
pub mod execution;
pub mod persistence;
pub mod reliability;
pub mod scheduler;
pub mod sysstat;
pub mod task_events;
pub mod update_field;
pub mod worker;
pub mod workflow;
// pub mod observability; // Phase 5
// pub mod admin;       // Phase 5

/// Benchmark support utilities
///
/// This module provides metrics collection and HTML report generation
/// for load testing the durable execution engine.
#[doc(hidden)]
pub mod bench;

/// Prelude for common imports
pub mod prelude {
    pub use crate::activity::{Activity, ActivityContext, ActivityError};
    pub use crate::engine::{ExecutorConfig, ExecutorError, WorkflowExecutor, WorkflowRegistry};
    pub use crate::persistence::{
        ClaimedTask, InMemoryWorkflowEventStore, PostgresWorkflowEventStore, StoreError,
        TaskDefinition, TraceContext, WorkerInfo, WorkflowEventStore, WorkflowStatus,
    };
    pub use crate::reliability::{CircuitBreakerConfig, RetryPolicy};
    pub use crate::scheduler::{DurableScheduler, SchedulerConfig, SchedulerError};
    pub use crate::worker::{WorkerPool, WorkerPoolConfig, WorkerPoolError};
    pub use crate::workflow::{
        ActivityOptions, Workflow, WorkflowAction, WorkflowError, WorkflowEvent, WorkflowSignal,
    };
}

// Re-export key types at crate root
pub use activity::{Activity, ActivityContext, ActivityError};
pub use engine::{
    ExecutorConfig, ExecutorError, SYSTEM_ACTIVITY_TYPES, WorkflowExecutor, WorkflowRegistry,
};
pub use execution::DurableExecution;
pub use persistence::{
    CircuitBreakerState, ClaimedTask, CreateScheduleRow, DeadTaskInfo, DlqEntry, DlqFilter,
    HeartbeatResponse, InMemoryWorkflowEventStore, Pagination, PostgresWorkflowEventStore,
    ReclaimResult, ScheduleExecutionFilter, ScheduleExecutionRow, ScheduleExecutionStatus,
    ScheduleFilter, ScheduleRow, ScheduleStats, ScheduleTargetType, SchedulerInstanceInfo,
    SealedTaskInfo, StoreError, SystemHealth, TaskDefinition, TaskFailureOutcome, TaskFilter,
    TaskInfo, TaskStatus, TraceContext, UpdateSchedule, WorkerFilter, WorkerInfo,
    WorkflowEventInfo, WorkflowEventStore, WorkflowFilter, WorkflowInfo, WorkflowInfoExtended,
    WorkflowStatus, no_progress_seal_threshold_from_env,
};
pub use reliability::{
    CircuitBreakerConfig, CircuitBreakerError, CircuitState, DistributedCircuitBreaker, RetryPolicy,
};
pub use scheduler::{DurableScheduler, SchedulerConfig, SchedulerError};
pub use update_field::UpdateField;
pub use worker::{WorkerPool, WorkerPoolConfig, WorkerPoolError};
pub use workflow::{
    ActivityOptions, Workflow, WorkflowAction, WorkflowError, WorkflowEvent, WorkflowSignal,
    signal_types,
};

// Re-export task event recording functions
pub use task_events::{
    append_event, record_activity_completed, record_activity_failed, record_activity_started,
    record_workflow_cancelled, record_workflow_completed, record_workflow_failed,
};
