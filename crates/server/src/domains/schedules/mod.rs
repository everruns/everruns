// Durable schedules domain — commands, queries, and types.

use everruns_core::{Policy, Rule};

pub mod commands;
pub mod queries;
pub mod types;

pub use commands::*;

/// Policy: Manage durable schedules. Gated on platform-user auth — matches
/// the axum `PlatformUser` extractor on these routes. Defense in depth so
/// MCP/gRPC paths also enforce.
pub const SCHEDULE_MANAGE: Policy = Policy {
    id: "schedule.manage",
    rules: &[Rule::IsPlatformUser],
};

/// Policy: View durable schedules.
pub const SCHEDULE_VIEW: Policy = Policy {
    id: "schedule.view",
    rules: &[Rule::IsPlatformUser],
};
