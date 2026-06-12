// Memory domain.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod files;
pub mod source_sync;
pub mod types;

pub use commands::*;

pub const MEMORY_VIEW: Policy = Policy {
    id: "memory.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};

pub const MEMORY_MANAGE: Policy = Policy {
    id: "memory.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};
