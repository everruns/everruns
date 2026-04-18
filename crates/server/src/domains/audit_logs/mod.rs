// Audit log domain — commands, queries, types.
//
// Read-only; audit writes stay on the AuditLogger trait (fire-and-forget
// side-effect of other operations, not a user-facing command).
//
// See specs/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;
pub use types::AuditLogEntry;

/// View audit logs (read-only, admin+owner).
pub const AUDIT_LOG_VIEW: Policy = Policy {
    id: "audit_log.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAuditLogsView)],
};
