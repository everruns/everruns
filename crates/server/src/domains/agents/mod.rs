// Agent domain — commands, queries, types.
//
// See specs/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;

/// Policy: View agents (read-only).
pub const AGENT_VIEW: Policy = Policy {
    id: "agent.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Manage agents (create, update, copy, delete).
pub const AGENT_MANAGE: Policy = Policy {
    id: "agent.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

pub const AGENT_DANGEROUS: Policy = Policy {
    id: "agent.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgAgentsDangerous),
    ],
};
