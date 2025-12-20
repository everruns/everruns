// In-process execution module
// Decision: Non-durable workflow execution using tokio tasks
// Decision: For durable execution, use temporal/ module instead
//
// This module contains:
// - runner.rs: InProcessRunner implementation of AgentRunner trait
// - workflow.rs: InProcessWorkflow for session execution

mod runner;
mod workflow;

pub use runner::InProcessRunner;
pub use workflow::InProcessWorkflow;

// Legacy alias for backwards compatibility
pub use workflow::SessionWorkflow;
