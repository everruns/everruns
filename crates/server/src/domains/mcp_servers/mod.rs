// MCP Server domain — commands, queries, types.
//
// See specs/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;

pub const MCP_SERVER_VIEW: Policy = Policy {
    id: "mcp_server.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};
pub const MCP_SERVER_MANAGE: Policy = Policy {
    id: "mcp_server.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};
pub const MCP_SERVER_DANGEROUS: Policy = Policy {
    id: "mcp_server.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgMcpServersDangerous),
    ],
};
