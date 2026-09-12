// LLM providers domain — commands, queries, types.
//
// See knowledge/foundations/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod credential_check;
pub mod queries;
pub mod service;
pub mod types;

pub use commands::*;
pub use credential_check::{CredentialCheckResult, check_credentials};
pub use service::*;

pub const LLM_PROVIDER_VIEW: Policy = Policy {
    id: "provider.view",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersView)],
};
pub const LLM_PROVIDER_MANAGE: Policy = Policy {
    id: "provider.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersManage)],
};
