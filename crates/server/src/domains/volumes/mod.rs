// Workspace Volumes domain.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod source_sync;
pub mod types;

pub use commands::*;

pub const VOLUME_VIEW: Policy = Policy {
    id: "volume.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};

pub const VOLUME_MANAGE: Policy = Policy {
    id: "volume.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};
