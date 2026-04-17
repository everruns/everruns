// Skills domain — commands, queries, types.
//
// See specs/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;

pub const SKILL_VIEW: Policy = Policy {
    id: "skill.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};
pub const SKILL_MANAGE: Policy = Policy {
    id: "skill.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};
pub const SKILL_DANGEROUS: Policy = Policy {
    id: "skill.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgSkillsDangerous),
    ],
};
