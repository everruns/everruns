//! Reporting domain.
//!
//! Queries are semantic and org-scoped by construction. Storage backends compile
//! this constrained model to SQL; user input is never treated as SQL.

use everruns_core::{Permission, Policy, Rule};

pub mod catalog;
pub mod commands;
pub mod service;
pub mod types;

pub use commands::*;
pub use service::*;

pub const REPORT_VIEW: Policy = Policy {
    id: "report.view",
    rules: &[Rule::UserHasPermission(Permission::OrgReportsView)],
};

pub const REPORT_MANAGE: Policy = Policy {
    id: "report.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgReportsManage)],
};

pub const REPORT_ADMIN: Policy = Policy {
    id: "report.admin",
    rules: &[Rule::UserHasPermission(Permission::OrgReportsAdmin)],
};
