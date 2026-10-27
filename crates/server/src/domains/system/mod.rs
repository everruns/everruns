// System domain — commands, queries, types.

use everruns_core::{Policy, Rule};

pub mod commands;
pub mod doctor;
pub mod queries;
pub mod types;

pub use commands::*;

/// Policy: read the whole install's health. Platform users only: the report
/// describes every server, worker and datastore, not one organization.
pub const SYSTEM_VIEW: Policy = Policy {
    id: "system.view",
    rules: &[Rule::IsPlatformUser],
};
