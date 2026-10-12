//! PostgreSQL-backed durable execution for the [Everruns](https://everruns.com)
//! ecosystem: a claimable task queue, signals, an event log, schedules, a
//! worker pool, and retries and circuit breakers.
//!
//! State lives in PostgreSQL. Workers claim tasks with
//! `SELECT ... FOR UPDATE SKIP LOCKED`, and anything a dead worker held is
//! reclaimed and retried, so work survives crashes and restarts with no
//! infrastructure beyond the database.
//!
//! - [`WorkflowEventStore`] is the storage contract, split into focused traits
//!   such as [`EventLog`], [`TaskQueue`] and [`SignalStore`].
//!   [`PostgresWorkflowEventStore`] is the production store and
//!   [`InMemoryWorkflowEventStore`] the test double.
//! - [`WorkerPool`] claims and runs tasks with bounded concurrency, heartbeats
//!   and backpressure; [`StaleTaskReaper`] reclaims work from dead workers.
//! - [`DurableScheduler`] fires cron and interval schedules.
//! - [`RetryPolicy`], [`ActivityOptions`] and [`DistributedCircuitBreaker`]
//!   control retries, timeouts and failure isolation.
//!
//! The crate knows workflows, tasks and signals as durable records, and nothing
//! about the domain built on top of it. Everruns' turn driver and the server's
//! cluster jobs drive the task queue directly and own their own semantics.
//!
//! # Example
//!
//! The crate ships its own PostgreSQL schema. Apply it with
//! [`PostgresWorkflowEventStore::migrate`], which is idempotent and safe to call
//! on every start-up, then enqueue and claim work.
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use everruns_durable::prelude::*;
//! use serde_json::json;
//!
//! let pool = sqlx::PgPool::connect("postgres://localhost/my_app").await?;
//! PostgresWorkflowEventStore::migrate(&pool).await?;
//! let store = PostgresWorkflowEventStore::new(pool);
//!
//! let workflow_id = uuid::Uuid::now_v7();
//! store.create_workflow(workflow_id, "order", json!({}), None).await?;
//! store
//!     .enqueue_task(TaskDefinition {
//!         workflow_id: Some(workflow_id),
//!         activity_id: "charge".into(),
//!         activity_type: "charge_card".into(),
//!         input: json!({ "amount": 42 }),
//!         options: ActivityOptions::default(),
//!     })
//!     .await?;
//!
//! let claimed = store.claim_task("worker-1", &["charge_card".into()], 1).await?;
//! # let _ = claimed;
//! # Ok(()) }
//! ```

pub mod activity;
pub mod maintenance;
pub mod persistence;
pub mod reliability;
pub mod scheduler;
mod update_field;
// `/proc` readings for the worker pool's backpressure and the bench reports.
// Not part of the API.
pub(crate) mod sysstat;
pub mod task_events;
pub mod worker;
pub mod workflow;

// The README's examples are compiled and run as doctests without rendering
// the README twice in the API docs.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;

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
}

// Root re-exports: the storage contract and its record types, the task event
// recorders the Everruns worker and server call, and the scheduler, worker
// pool and reaper entry points. Narrow helpers (env readers, `/proc` readings)
// stay at their module path or crate-private.
pub use activity::ActivityError;
pub use maintenance::{
    NoopReapHandler, ReapHandler, ReaperConfig, StaleTaskReaper, reap_stale_tasks,
    requeue_stranded_workflows,
};
pub use persistence::{
    CircuitBreakerState, CircuitBreakers, ClaimedTask, CreateScheduleRow, DeadLetters,
    DeadTaskInfo, DlqEntry, DlqFilter, DurableAdmin, Enqueued, EventLog, HandOff, HandedOff,
    HeartbeatResponse, InMemoryWorkflowEventStore, NextStep, Pagination,
    PostgresWorkflowEventStore, ReclaimResult, RequeuedWorkflow, RunStart, RunSteering,
    ScheduleExecutionFilter, ScheduleExecutionRow, ScheduleExecutionStatus, ScheduleFilter,
    ScheduleRow, ScheduleStats, ScheduleTargetType, SchedulerInstanceInfo, Schedules,
    SealedTaskInfo, SignalDrain, SignalStore, StoreError, SystemHealth, TaskDefinition,
    TaskFailureOutcome, TaskFilter, TaskInfo, TaskQueue, TaskStatus, TraceContext, UpdateSchedule,
    WorkerFilter, WorkerHeartbeat, WorkerInfo, WorkerRegistry, WorkflowEndSubscription,
    WorkflowEventInfo, WorkflowEventStore, WorkflowFilter, WorkflowInfo, WorkflowInfoExtended,
    WorkflowStatus,
};
pub use reliability::{
    CircuitBreakerConfig, CircuitBreakerError, CircuitState, DistributedCircuitBreaker, RetryPolicy,
};
pub use scheduler::{
    Cadence, DurableScheduler, EnsureOutcome, ScheduleSpec, SchedulerConfig, SchedulerError,
    disable_schedule, ensure_schedule, find_schedule,
};
// `ScheduleUpdate` fields are `UpdateField`s, so the type stays nameable here.
pub use update_field::UpdateField;
pub use worker::{WorkerPool, WorkerPoolConfig, WorkerPoolError};
pub use workflow::{ActivityOptions, WorkflowError, WorkflowEvent, WorkflowSignal, signal_types};

// Task event recording, used by the Everruns turn driver and server sweeps.
pub use task_events::{
    append_event, record_activity_completed, record_activity_failed, record_activity_started,
    record_workflow_cancelled, record_workflow_completed, record_workflow_failed,
};
