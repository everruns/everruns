// Temporal workflow implementations
// Decision: Workflows are state machines that produce commands in response to activations
// Decision: Use trait-based abstraction for pluggable workflow types
// Decision: WorkflowRegistry maps workflow_type strings to factory functions

mod agent_run;

pub use agent_run::{TemporalSessionWorkflow, TemporalSessionWorkflowState};
// Legacy alias for backwards compatibility
pub type AgentRunWorkflow = TemporalSessionWorkflow;
pub type AgentRunWorkflowState = TemporalSessionWorkflowState;

// Re-export from root modules
pub use crate::workflow_registry::{WorkflowFactory, WorkflowRegistry, WorkflowRegistryBuilder};
pub use crate::workflow_traits::{Workflow, WorkflowInput};

// Re-export WorkflowAction from types
pub use super::types::WorkflowAction;
