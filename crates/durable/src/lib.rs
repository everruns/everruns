//! # Durable Execution Engine
//!
//! A PostgreSQL-backed workflow orchestration engine for reliable, distributed task execution.
//!
//! ## Features
//!
//! - **Event-sourced workflows**: All state changes are persisted as events, enabling replay and recovery
//! - **Automatic retries**: Configurable retry policies with exponential backoff and jitter
//! - **Circuit breakers**: Protect external services from cascading failures
//! - **Distributed task queue**: Scalable task distribution with backpressure support
//! - **OpenTelemetry integration**: Full observability with traces and metrics
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
//! ```ignore
//! use everruns_durable::prelude::*;
//!
//! #[derive(Debug, Serialize, Deserialize)]
//! struct MyWorkflow {
//!     state: MyState,
//! }
//!
//! impl Workflow for MyWorkflow {
//!     const TYPE: &'static str = "my_workflow";
//!     type Input = MyInput;
//!     type Output = MyOutput;
//!
//!     fn new(input: Self::Input) -> Self {
//!         Self { state: MyState::Init }
//!     }
//!
//!     fn on_start(&mut self) -> Vec<WorkflowAction> {
//!         vec![WorkflowAction::ScheduleActivity {
//!             activity_id: "step-1".into(),
//!             activity_type: "my_activity".into(),
//!             input: json!({}),
//!             options: ActivityOptions::default(),
//!         }]
//!     }
//!
//!     // ... implement other trait methods
//! }
//! ```

pub mod activity;
pub mod engine;
pub mod persistence;
pub mod reliability;
pub mod worker;
pub mod workflow;
// pub mod observability; // Phase 5
// pub mod admin;       // Phase 5

/// Prelude for common imports
pub mod prelude {
    pub use crate::activity::{Activity, ActivityContext, ActivityError};
    pub use crate::engine::{ExecutorConfig, ExecutorError, WorkflowExecutor, WorkflowRegistry};
    pub use crate::persistence::{
        ClaimedTask, InMemoryWorkflowEventStore, PostgresWorkflowEventStore, StoreError,
        TaskDefinition, TraceContext, WorkflowEventStore, WorkflowStatus,
    };
    pub use crate::reliability::{CircuitBreakerConfig, RetryPolicy};
    pub use crate::worker::{WorkerPool, WorkerPoolConfig, WorkerPoolError};
    pub use crate::workflow::{
        ActivityOptions, Workflow, WorkflowAction, WorkflowError, WorkflowEvent, WorkflowSignal,
    };
}

// Re-export key types at crate root
pub use activity::{Activity, ActivityContext, ActivityError};
pub use engine::{ExecutorConfig, ExecutorError, WorkflowExecutor, WorkflowRegistry};
pub use persistence::{
    InMemoryWorkflowEventStore, PostgresWorkflowEventStore, StoreError, TraceContext,
    WorkflowEventStore, WorkflowStatus,
};
pub use reliability::{CircuitBreakerConfig, RetryPolicy};
pub use worker::{WorkerPool, WorkerPoolConfig, WorkerPoolError};
pub use workflow::{
    ActivityOptions, Workflow, WorkflowAction, WorkflowError, WorkflowEvent, WorkflowSignal,
};
