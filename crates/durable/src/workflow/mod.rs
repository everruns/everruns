//! Workflow abstractions and types
//!
//! This module contains the core workflow primitives:
//! - [`WorkflowEvent`] enum for persisted events
//! - [`WorkflowSignal`] for external communication
//! - [`ActivityOptions`] and [`WorkflowError`], shared by the task queue
//!
//! With the experimental `workflows` feature it also holds the `Workflow`
//! trait for defining workflow state machines and the `WorkflowAction` enum
//! for workflow commands.

#[cfg(feature = "workflows")]
mod action;
#[cfg(feature = "workflows")]
mod definition;
mod error;
mod event;
mod options;
mod signal;

#[cfg(feature = "workflows")]
pub use action::WorkflowAction;
#[cfg(feature = "workflows")]
pub use definition::Workflow;
pub use error::WorkflowError;
pub use event::{ParentWorkflow, TimeoutType, WorkflowEvent};
pub use options::ActivityOptions;
pub use signal::{WorkflowSignal, signal_types};
