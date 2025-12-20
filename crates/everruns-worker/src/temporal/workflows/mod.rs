// Temporal workflow implementations
// Decision: Workflows are state machines that produce commands in response to activations
// Decision: Use trait-based abstraction for pluggable workflow types
// Decision: WorkflowRegistry maps workflow_type strings to factory functions

mod agent_run;
mod registry;
mod traits;

pub use agent_run::{TemporalSessionWorkflow, TemporalSessionWorkflowState};
// Legacy alias for backwards compatibility
pub type AgentRunWorkflow = TemporalSessionWorkflow;
pub type AgentRunWorkflowState = TemporalSessionWorkflowState;
pub use registry::{WorkflowRegistry, WorkflowRegistryBuilder};
pub use traits::{Workflow, WorkflowInput};

// Re-export WorkflowAction from types (it's already there)
pub use super::types::WorkflowAction;
