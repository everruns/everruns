// Temporal integration module
// Decision: Keep all Temporal-specific code in a dedicated module for clarity
//
// This module contains:
// - types.rs: Workflow/activity input/output types
// - activities.rs: Activity implementations (database, LLM, tools)
// - workflows/: Workflow state machines with trait-based abstraction
// - client.rs: Temporal client for starting workflows
// - runner.rs: AgentRunner implementation using Temporal
//
// Note: TemporalWorker is in the root module (crate::temporal_worker)

pub(crate) mod activities;
pub(crate) mod client;
mod runner;
pub mod types;
pub mod workflows;

// Re-export main types for external use
pub use client::TemporalClient;
pub use runner::{run_temporal_worker, TemporalRunner};
pub use types::*;
pub use workflows::{
    // Legacy aliases for backwards compatibility
    AgentRunWorkflow,
    AgentRunWorkflowState,
    // Primary exports
    TemporalSessionWorkflow,
    TemporalSessionWorkflowState,
    Workflow,
    WorkflowAction,
    WorkflowInput,
    WorkflowRegistry,
    WorkflowRegistryBuilder,
};
