// Memory domain.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod entries;
pub mod files;
pub mod record;
pub mod types;

pub use commands::*;
pub use entries::*;

pub const MEMORY_VIEW: Policy = Policy {
    id: "memory.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};

pub const MEMORY_MANAGE: Policy = Policy {
    id: "memory.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};
