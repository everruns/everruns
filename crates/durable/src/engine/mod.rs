//! Workflow execution engine
//!
//! The engine module provides the `WorkflowExecutor` which drives workflow
//! state machines through event replay and action processing.
//!
//! **Experimental** (`workflows` feature): the API may change in any release.
//! Everruns' own services drive the task queue directly and do not run it.

mod executor;
mod registry;
mod replay;

pub use executor::{ExecutorConfig, ExecutorError, SYSTEM_ACTIVITY_TYPES, WorkflowExecutor};
pub use registry::{WorkflowFactory, WorkflowRegistry};
