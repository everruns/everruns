// Agent-triggers domain — commands, queries, types.
//
// Agent triggers are agent-owned, cron-driven durable schedules that wake the
// agent on its own harness. Management is gated by the same policies as agent
// CRUD (see `domains::agents`): `AGENT_VIEW` for reads, `AGENT_MANAGE` for
// mutations and manual fires. See knowledge/foundations/domains.md for the command pattern.

pub mod commands;
pub mod deliveries;
pub mod events;
pub mod github;
pub mod mcp_event;
pub mod queries;
pub mod record;
mod script_target;
pub mod types;
pub mod webhook;
pub mod webhook_invocation;

pub use commands::*;
pub use deliveries::*;
pub use mcp_event::McpEventTriggers;
pub use webhook_invocation::{WebhookTriggerInvocationRequest, invoke_webhook_agent_trigger};
