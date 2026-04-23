// Notification domain — commands, queries, types.
//
// See specs/domains.md for the pattern.

use everruns_core::{OrgRole, Policy, Rule};

pub mod commands;
pub mod queries;
pub mod service;
pub mod types;

pub use commands::*;
pub use service::*;

/// Policy: Access the authenticated caller's own notifications. Requires
/// any org role (i.e., the caller must be a member of the org). The
/// notification handler further scopes by `caller.user_id`.
pub const NOTIFICATION_ACCESS: Policy = Policy {
    id: "notification.access",
    rules: &[Rule::UserHasRole(OrgRole::Member)],
};
