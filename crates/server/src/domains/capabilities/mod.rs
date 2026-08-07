// Capability domain — registry reads plus persisted declarative capabilities.
// See knowledge/foundations/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod types;
pub mod validation;

pub use commands::*;

pub const CAPABILITY_VIEW: Policy = Policy {
    id: "capability.view",
    rules: &[Rule::UserHasPermission(Permission::OrgCapabilitiesView)],
};

pub const CAPABILITY_MANAGE: Policy = Policy {
    id: "capability.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgCapabilitiesManage)],
};

// Dangerous capability ops reuse OrgSkillsDangerous (Owner-only) as the danger
// tier — a pre-existing choice kept here to preserve the Owner-only gate.
pub const CAPABILITY_DANGEROUS: Policy = Policy {
    id: "capability.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgCapabilitiesManage),
        Rule::UserHasPermission(Permission::OrgSkillsDangerous),
    ],
};
