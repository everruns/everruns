// Org-scoped memory stores domain.
// See specs/memory.md.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod types;

pub use commands::*;

pub const MEMORY_STORE_VIEW: Policy = Policy {
    id: "memory_store.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};

pub const MEMORY_STORE_MANAGE: Policy = Policy {
    id: "memory_store.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};
