//! Workflow execution engine
//!
//! The engine module provides the `WorkflowExecutor` which drives workflow
//! state machines through event replay and action processing.

mod executor;
mod registry;
mod replay;

pub use executor::{ExecutorConfig, ExecutorError, WorkflowExecutor};
pub use registry::{WorkflowFactory, WorkflowRegistry};
