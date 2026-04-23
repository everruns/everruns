// Budget domain — commands, queries, types.
//
// See specs/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod service;
pub mod types;

pub use commands::*;
pub use service::*;

/// Policy: Manage budgets. Budgets control spending limits; treated as a
/// settings-level concern so only Owner/Admin can modify.
pub const BUDGET_MANAGE: Policy = Policy {
    id: "budget.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

/// Policy: View budgets. Any caller with settings view, which every role
/// receives today.
pub const BUDGET_VIEW: Policy = Policy {
    id: "budget.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};
