// V2 Session Workflow - Temporal-based infinite loop workflow
//
// Decision: Workflow represents a session (not a single turn)
// Decision: Infinite loop structure: input -> (agent -> tools) -> output -> wait for signal
// Decision: LLM Call is a Temporal activity
// Decision: Tool calls can run in parallel as separate activities
// Decision: Uses Temporal signals to receive new messages
// Decision: Error if message arrives while running, accept if waiting

pub mod workflow;

pub use workflow::*;
