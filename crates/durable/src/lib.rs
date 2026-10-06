//! PostgreSQL-backed durable execution for the [Everruns](https://everruns.com)
//! ecosystem: event-sourced workflows, a claimable task queue, retries,
//! circuit breakers and schedules.
//!
//! State lives in PostgreSQL. Workers claim tasks with
//! `SELECT ... FOR UPDATE SKIP LOCKED`, and anything a dead worker held is
//! reclaimed and retried, so work survives crashes and restarts with no
//! infrastructure beyond the database.
//!
//! - [`WorkflowEventStore`] is the storage contract, split into focused traits
//!   such as [`EventLog`] and [`TaskQueue`]. [`PostgresWorkflowEventStore`] is
//!   the production store and [`InMemoryWorkflowEventStore`] the test double.
//! - [`DurableScheduler`] fires cron and interval schedules.
//! - [`RetryPolicy`], [`ActivityOptions`] and [`DistributedCircuitBreaker`]
//!   control retries, timeouts and failure isolation.
//!
//! The crate is a generic durable-execution engine: it knows workflows,
//! activities, tasks and signals, and nothing about the domain built on top of
//! it. Everruns' agent-turn semantics live in the worker and server, which
//! drive the task queue directly.
//!
//! # Features
//!
//! - `workflows` (default, **experimental**): the general-purpose workflow
//!   engine. `Workflow` is a deterministic state machine whose handlers
//!   return `WorkflowAction`s; `WorkflowExecutor` starts workflows, appends
//!   [`WorkflowEvent`]s, replays them after a crash, and runs timers and child
//!   workflows as system tasks. The API may change in
//!   any release; Everruns itself does not run it in production, so it is
//!   proven by this crate's tests, benches and `examples/order_pipeline.rs`.
//!   Turn it off with `default-features = false` to compile only the store,
//!   queue, reliability, scheduler and worker pool core.
//! - `sqlite`: the `sqlite` module, a small rusqlite wrapper for local hosts.
//!
//! # Example
//!
//! The crate ships its own PostgreSQL schema. Apply it with
//! [`PostgresWorkflowEventStore::migrate`], which is idempotent and safe to call
//! on every start-up, then build the executor over the store.
//!
//! ```no_run
//! # #[cfg(feature = "workflows")]
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns_durable::prelude::*;
//!
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
//! workflows and the reliability toolkit; `examples/order_pipeline.rs` is a
//! runnable one on the in-memory store.

// The README's examples are compiled and run as doctests without rendering
// the README twice in the API docs. They drive the workflow executor.
#[cfg(all(doctest, feature = "workflows"))]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

pub mod activity;
#[cfg(feature = "workflows")]
pub mod engine;
pub mod maintenance;
pub mod persistence;
pub mod reliability;
pub mod scheduler;
#[cfg(feature = "sqlite")]
pub mod sqlite;
// `/proc` readings for the worker pool's backpressure and the bench reports.
// Not part of the API.
pub(crate) mod sysstat;
pub mod task_events;
pub mod update_field;
pub mod worker;
pub mod workflow;

/// PostgreSQL pool passed by an owning host to the durable store API.
pub type PostgresPool = sqlx::PgPool;

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
    pub use crate::activity::ActivityError;
    #[cfg(feature = "workflows")]
    pub use crate::activity::{Activity, ActivityContext};
    #[cfg(feature = "workflows")]
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
    pub use crate::workflow::{ActivityOptions, WorkflowError, WorkflowEvent, WorkflowSignal};
    #[cfg(feature = "workflows")]
    pub use crate::workflow::{Workflow, WorkflowAction};
}

// Root re-exports: the storage contract and its record types, the task event
// recorders the Everruns worker and server call, and the generic engine entry
// points. Narrow helpers (env readers, `/proc` readings, timeout internals)
// stay at their module path or crate-private.
pub use activity::ActivityError;
#[cfg(feature = "workflows")]
pub use activity::{Activity, ActivityContext};
#[cfg(feature = "workflows")]
pub use engine::{
    ExecutorConfig, ExecutorError, SYSTEM_ACTIVITY_TYPES, WorkflowExecutor, WorkflowRegistry,
};
pub use maintenance::{
    NoopReapHandler, ReapHandler, ReaperConfig, StaleTaskReaper, reap_stale_tasks,
};
pub use persistence::{
    CircuitBreakerState, CircuitBreakers, ClaimedTask, CreateScheduleRow, DeadLetters,
    DeadTaskInfo, DlqEntry, DlqFilter, DurableAdmin, Enqueued, EventLog, HeartbeatResponse,
    InMemoryWorkflowEventStore, Pagination, PostgresWorkflowEventStore, ReclaimResult, RunStart,
    ScheduleExecutionFilter, ScheduleExecutionRow, ScheduleExecutionStatus, ScheduleFilter,
    ScheduleRow, ScheduleStats, ScheduleTargetType, SchedulerInstanceInfo, Schedules,
    SealedTaskInfo, SignalStore, StoreError, SystemHealth, TaskDefinition, TaskFailureOutcome,
    TaskFilter, TaskInfo, TaskQueue, TaskStatus, TraceContext, UpdateSchedule, WorkerFilter,
    WorkerHeartbeat, WorkerInfo, WorkerRegistry, WorkflowEndSubscription, WorkflowEventInfo,
    WorkflowEventStore, WorkflowFilter, WorkflowInfo, WorkflowInfoExtended, WorkflowStatus,
};
pub use reliability::{
    CircuitBreakerConfig, CircuitBreakerError, CircuitState, DistributedCircuitBreaker, RetryPolicy,
};
pub use scheduler::{
    Cadence, DurableScheduler, EnsureOutcome, ScheduleSpec, SchedulerConfig, SchedulerError,
    disable_schedule, ensure_schedule, find_schedule,
};
pub use update_field::UpdateField;
pub use worker::{WorkerPool, WorkerPoolConfig, WorkerPoolError};
pub use workflow::{ActivityOptions, WorkflowError, WorkflowEvent, WorkflowSignal, signal_types};
#[cfg(feature = "workflows")]
pub use workflow::{Workflow, WorkflowAction};

// Task event recording, used by the Everruns turn driver and server sweeps.
pub use task_events::{
    append_event, record_activity_completed, record_activity_failed, record_activity_started,
    record_workflow_cancelled, record_workflow_completed, record_workflow_failed,
};
