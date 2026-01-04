//! Workflow abstractions and types
//!
//! This module contains the core workflow primitives:
//! - [`Workflow`] trait for defining workflow state machines
//! - [`WorkflowAction`] enum for workflow commands
//! - [`WorkflowEvent`] enum for persisted events
//! - [`WorkflowSignal`] for external communication

mod action;
mod definition;
mod event;
mod signal;

pub use action::{ActivityOptions, WorkflowAction};
pub use definition::{Workflow, WorkflowError};
pub use event::{TimeoutType, WorkflowEvent};
pub use signal::{signal_types, WorkflowSignal};
