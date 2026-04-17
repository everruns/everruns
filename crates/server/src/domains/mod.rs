// Feature-oriented domain modules.
//
// Each domain owns its commands (user-facing operations), queries (shared
// read/write helpers), and types. See specs/domains.md for the full pattern.

pub mod common;

pub mod agent_identities;
pub mod agents;
pub mod apps;
pub mod capabilities;
pub mod harnesses;
pub mod mcp_servers;
pub mod skills;
