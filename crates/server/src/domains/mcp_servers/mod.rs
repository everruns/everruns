// MCP Server domain — commands, queries, types.
//
// See knowledge/foundations/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod scoped_mcp;
pub mod service;
pub mod types;

pub use commands::*;
pub use service::{
    McpServerOAuthSettings, McpServerResolved, McpServerService, McpServerSettings,
    McpServerWithTools,
};

pub const MCP_SERVER_VIEW: Policy = Policy {
    id: "mcp_server.view",
    rules: &[Rule::UserHasPermission(Permission::OrgMcpServersView)],
};
pub const MCP_SERVER_MANAGE: Policy = Policy {
    id: "mcp_server.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgMcpServersManage)],
};
pub const MCP_SERVER_DANGEROUS: Policy = Policy {
    id: "mcp_server.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgMcpServersManage),
        Rule::UserHasPermission(Permission::OrgMcpServersDangerous),
    ],
};
