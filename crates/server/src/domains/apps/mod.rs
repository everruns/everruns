// App domain — commands, queries, types.
//
// See specs/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;

/// Policy: View apps (read-only).
pub const APP_VIEW: Policy = Policy {
    id: "app.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Manage apps (create, update).
pub const APP_MANAGE: Policy = Policy {
    id: "app.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Dangerous app operations (delete, publish, unpublish).
pub const APP_DANGEROUS: Policy = Policy {
    id: "app.dangerous",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgAppsDangerous),
    ],
};
