// Agent-triggers domain — commands, queries, types.
//
// Agent triggers are agent-owned, cron-driven durable schedules that wake the
// agent on its own harness. Management is gated by the same policies as agent
// CRUD (see `domains::agents`): `AGENT_VIEW` for reads, `AGENT_MANAGE` for
// mutations and manual fires. See specs/domains.md for the command pattern.

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;
