//! # everruns-durable
//!
//! Generic PostgreSQL-backed workflow orchestration for reliable, distributed
//! task execution. The crate owns no Everruns server DTOs or agent policy;
//! [`everruns-scale`](https://docs.rs/everruns-scale) composes it behind the
//! public distributed Engine.
//!
//! Part of the [Everruns](https://everruns.com) ecosystem.
//!
//! ## Features
//!
//! - **Event-sourced workflows**: All state changes are persisted as events, enabling replay and recovery
//! - **Automatic retries**: Configurable retry policies with exponential backoff and jitter
//! - **Circuit breakers**: Protect external services from cascading failures
//! - **Distributed task queue**: Scalable task distribution with backpressure support
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      WorkflowExecutor                        │
//! │  (drives workflow state machines, handles event replay)     │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                   WorkflowEventStore                         │
//! │  (PostgreSQL: durable_workflow_instances, events, tasks)    │
//! └─────────────────────────────────────────────────────────────┘
//!                              │
//!                              ▼
//! ┌─────────────────────────────────────────────────────────────┐
//! │                      WorkerPool                              │
//! │  (claims tasks, executes activities, sends heartbeats)      │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Example
//!
//! ```rust
//! use everruns_durable::{InMemoryWorkflowEventStore, WorkflowEventStore};
//!
//! let store: std::sync::Arc<dyn WorkflowEventStore> =
//!     std::sync::Arc::new(InMemoryWorkflowEventStore::new());
//! assert_eq!(std::sync::Arc::strong_count(&store), 1);
//! ```

pub mod activity;
pub mod engine;
pub mod persistence;
pub mod reliability;
pub mod scheduler;
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
#[cfg(feature = "benchmarks")]
pub mod bench;

/// Prelude for common imports
pub mod prelude {
    pub use crate::activity::{Activity, ActivityContext, ActivityError};
    pub use crate::engine::{ExecutorConfig, ExecutorError, WorkflowExecutor, WorkflowRegistry};
    pub use crate::persistence::{
        ClaimedTask, InMemoryWorkflowEventStore, PostgresWorkflowEventStore, StoreError,
        TaskDefinition, TraceContext, WorkflowEventStore, WorkflowStatus,
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
pub use engine::{ExecutorConfig, ExecutorError, WorkflowExecutor, WorkflowRegistry};
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
