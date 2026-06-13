// LLM models domain — commands, queries, types.
//
// See specs/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod service;
pub mod types;

pub use commands::*;
pub use service::*;

pub const LLM_MODEL_VIEW: Policy = Policy {
    id: "llm_model.view",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersView)],
};
pub const LLM_MODEL_MANAGE: Policy = Policy {
    id: "llm_model.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersManage)],
};
