//! Workflow abstractions and types
//!
//! This module contains the core workflow primitives:
//! - [`WorkflowEvent`] enum for persisted events
//! - [`WorkflowSignal`] for external communication
//! - [`ActivityOptions`] and [`WorkflowError`], shared by the task queue
//!
//! A workflow here is a durable record (instance row, event log, signals and
//! tasks) driven by the caller, not a replayed state machine.

mod error;
mod event;
mod options;
mod signal;

pub use error::WorkflowError;
pub use event::{ParentWorkflow, TimeoutType, WorkflowEvent};
pub use options::ActivityOptions;
pub use signal::{WorkflowSignal, signal_types};
