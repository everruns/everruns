// Agent-scripts domain: saved shell scripts an agent owns.
//
// A much smaller sibling of `agent_triggers`: name, description, optional
// input schema, body, and nothing that runs on its own. Gated by the same
// policies as agent CRUD (`AGENT_VIEW` for reads, `AGENT_MANAGE` for writes).
// The worker's internal caller passes those policies like it does for
// triggers: `Caller::internal` bypasses policy evaluation.
// Design: knowledge/runtime-resources/agent-scripts.md.

pub mod commands;
pub mod queries;
pub mod record;
pub mod types;
pub mod validation;

#[cfg(test)]
mod tests;

pub use commands::*;
