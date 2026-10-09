//! The domain-command catalog behind the scripted command surface: MCP
//! `discover`/`query`/`execute`, the Platform capability and the worker shell.
//!
//! Decision: transport-neutral, so it lives under `services/` rather than in
//! the HTTP layer. `api::mcp_endpoint` re-exports these modules.

pub(crate) mod catalog;
pub(crate) mod cli_tree;
mod command_line;
pub(crate) mod positional;
