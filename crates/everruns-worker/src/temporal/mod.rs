// Temporal integration module
// Decision: Keep all Temporal-specific code in a dedicated module for clarity
//
// This module contains:
// - types.rs: Workflow/activity input/output types
// - activities.rs: Activity implementations (database, LLM, tools)
// - workflows.rs: Workflow state machines
// - client.rs: Temporal client for starting workflows
// - worker.rs: Worker for polling and executing tasks
// - runner.rs: AgentRunner implementation using Temporal

mod activities;
mod client;
mod runner;
mod types;
mod worker;
mod workflows;

// Re-export main types for external use
pub use client::TemporalClient;
pub use runner::{run_temporal_worker, TemporalRunner};
pub use types::*;
pub use worker::TemporalWorker;
pub use workflows::{AgentRunWorkflow, WorkflowAction};
