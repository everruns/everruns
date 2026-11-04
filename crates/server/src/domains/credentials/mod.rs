//! Scoped write-only credentials; the shared access domain owns authorization.
pub mod commands;
pub mod management;
pub mod service;
#[cfg(test)]
mod tests;

pub const CREDENTIAL_VIEW: everruns_core::Policy = everruns_core::Policy {
    id: "credential.view",
    rules: &[everruns_core::Rule::UserHasPermission(
        everruns_core::Permission::OrgCredentialsView,
    )],
};
pub const CREDENTIAL_MANAGE: everruns_core::Policy = everruns_core::Policy {
    id: "credential.manage",
    rules: &[everruns_core::Rule::UserHasPermission(
        everruns_core::Permission::OrgCredentialsManage,
    )],
};
