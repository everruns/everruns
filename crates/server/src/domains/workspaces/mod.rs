// Workspace domain.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod types;

pub use commands::*;

pub const WORKSPACE_VIEW: Policy = Policy {
    id: "workspace.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};

pub const WORKSPACE_MANAGE: Policy = Policy {
    id: "workspace.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};
