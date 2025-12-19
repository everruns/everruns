// V2 Agent Workflow - Session-based infinite loop workflow
//
// Decision: Workflow represents a session (not a single turn)
// Decision: Infinite loop structure: input -> (agent -> tools) -> output -> wait
// Decision: LLM Call is an activity
// Decision: Tool calls can run in parallel
// Decision: When new message arrives: error if running, add message if waiting
//
// This module provides an isolated, runnable, and testable workflow implementation
// that does not depend on external infrastructure (Temporal, database, etc.)

pub mod activities;
pub mod executor;
pub mod types;
pub mod workflow;

pub use activities::*;
pub use executor::*;
pub use types::*;
pub use workflow::*;
