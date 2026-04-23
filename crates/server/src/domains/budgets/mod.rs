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

/// Policy: Manage budgets (create, update, top-up, delete, resume).
pub const BUDGET_MANAGE: Policy = Policy {
    id: "budget.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};
