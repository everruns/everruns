// Harness domain — commands, queries, types.
//
// See specs/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;

/// Policy: View harnesses (read-only).
pub const HARNESS_VIEW: Policy = Policy {
    id: "harness.view",
    rules: &[Rule::UserHasPermission(Permission::OrgHarnessesView)],
};

/// Policy: CRUD on harnesses (create, update, copy).
pub const HARNESS_MANAGE: Policy = Policy {
    id: "harness.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgHarnessesManage)],
};

/// Policy: Dangerous harness operations (delete).
pub const HARNESS_DANGEROUS: Policy = Policy {
    id: "harness.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgHarnessesManage),
        Rule::UserHasPermission(Permission::OrgHarnessesDangerous),
    ],
};
