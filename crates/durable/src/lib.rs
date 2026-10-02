//! PostgreSQL-backed durable execution for the [Everruns](https://everruns.com)
//! ecosystem: event-sourced workflows, a claimable task queue, retries,
//! circuit breakers and schedules.
//!
//! State lives in PostgreSQL. Workers claim tasks with
//! `SELECT ... FOR UPDATE SKIP LOCKED`, and anything a dead worker held is
//! reclaimed and retried, so work survives crashes and restarts with no
//! infrastructure beyond the database.
//!
//! - [`Workflow`] is a deterministic state machine whose handlers return
//!   [`WorkflowAction`]s; [`WorkflowExecutor`] starts workflows, appends
//!   [`WorkflowEvent`]s and replays them after a crash.
//! - [`WorkflowEventStore`] is the storage contract, split into focused traits
//!   such as [`EventLog`] and [`TaskQueue`]. [`PostgresWorkflowEventStore`] is
//!   the production store and [`InMemoryWorkflowEventStore`] the test double.
//! - [`WorkerPool`] runs activity handlers with bounded concurrency,
//!   heartbeats and backpressure; [`DurableScheduler`] fires cron and interval
//!   schedules.
//! - [`RetryPolicy`], [`ActivityOptions`] and [`DistributedCircuitBreaker`]
//!   control retries, timeouts and failure isolation.
//!
//! The crate is a generic durable-execution engine: it knows workflows,
//! activities, tasks and signals, and nothing about the domain built on top of
//! it. Everruns' agent-turn semantics live in the worker and server.
//!
//! # Example
//!
//! The crate ships its own PostgreSQL schema. Apply it with
//! [`PostgresWorkflowEventStore::migrate`], which is idempotent and safe to call
//! on every start-up, then build the executor over the store.
//!
//! ```no_run
//! use everruns_durable::prelude::*;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let pool = sqlx::PgPool::connect("postgres://localhost/my_app").await?;
//! PostgresWorkflowEventStore::migrate(&pool).await?;
//!
//! let mut executor = WorkflowExecutor::new(PostgresWorkflowEventStore::new(pool));
//! // executor.register::<MyWorkflow>();
//! # let _ = &mut executor;
//! # Ok(()) }
//! ```
//!
//! The crate README walks through a complete workflow, workers, timers, child
//! workflows and the reliability toolkit.

// The README's examples are compiled and run as doctests without rendering
// the README twice in the API docs.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub mod activity;
pub mod engine;
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
/// for load testing the durable execution engine. Behind the `bench` feature,
/// which only the crate's own bench binaries enable; not a supported API.
#[cfg(feature = "bench")]
#[doc(hidden)]
pub mod bench;

/// Prelude for common imports
pub mod prelude {
    pub use crate::activity::{Activity, ActivityContext, ActivityError};
    pub use crate::engine::{ExecutorConfig, ExecutorError, WorkflowExecutor, WorkflowRegistry};
    pub use crate::persistence::{
        CircuitBreakers, ClaimedTask, DeadLetters, DurableAdmin, EventLog,
        InMemoryWorkflowEventStore, PostgresWorkflowEventStore, Schedules, SignalStore, StoreError,
        TaskDefinition, TaskQueue, TraceContext, WorkerInfo, WorkerRegistry, WorkflowEventStore,
        WorkflowStatus,
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
pub use persistence::{
    CircuitBreakerState, CircuitBreakers, ClaimedTask, CreateScheduleRow, DeadLetters,
    DeadTaskInfo, DlqEntry, DlqFilter, DurableAdmin, EventLog, HeartbeatResponse,
    InMemoryWorkflowEventStore, Pagination, PostgresWorkflowEventStore, ReclaimResult,
    ScheduleExecutionFilter, ScheduleExecutionRow, ScheduleExecutionStatus, ScheduleFilter,
    ScheduleRow, ScheduleStats, ScheduleTargetType, SchedulerInstanceInfo, Schedules,
    SealedTaskInfo, SignalStore, StoreError, SystemHealth, TaskDefinition, TaskFailureOutcome,
    TaskFilter, TaskInfo, TaskQueue, TaskStatus, TraceContext, UpdateSchedule, WorkerFilter,
    WorkerInfo, WorkerRegistry, WorkflowEventInfo, WorkflowEventStore, WorkflowFilter,
    WorkflowInfo, WorkflowInfoExtended, WorkflowStatus, no_progress_seal_threshold_from_env,
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
