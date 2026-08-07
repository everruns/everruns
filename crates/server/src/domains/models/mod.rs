// LLM models domain — commands, queries, types.
//
// See knowledge/foundations/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod service;
pub mod types;

pub use commands::*;
pub use service::*;

pub const LLM_MODEL_VIEW: Policy = Policy {
    id: "model.view",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersView)],
};
pub const LLM_MODEL_MANAGE: Policy = Policy {
    id: "model.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersManage)],
};
