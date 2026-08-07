// Agent identity domain — commands, queries, types.
//
// See knowledge/foundations/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;

pub const AGENT_IDENTITY_VIEW: Policy = Policy {
    id: "agent_identity.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentIdentitiesView)],
};

pub const AGENT_IDENTITY_MANAGE: Policy = Policy {
    id: "agent_identity.manage",
    rules: &[Rule::UserHasPermission(
        Permission::OrgAgentIdentitiesManage,
    )],
};

// Dangerous agent-identity ops reuse OrgAppsDangerous (Owner-only) as the danger
// tier — a pre-existing choice kept here to preserve the Owner-only gate.
pub const AGENT_IDENTITY_DANGEROUS: Policy = Policy {
    id: "agent_identity.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentIdentitiesManage),
        Rule::UserHasPermission(Permission::OrgAppsDangerous),
    ],
};
