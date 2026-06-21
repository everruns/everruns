// Session schedule domain types
//
// Represents scheduled tasks bound to a session.
// When a schedule fires, a message is injected into the session to trigger a turn.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::principal::PrincipalSummary;
use crate::typed_id::{PrincipalId, ScheduleId, SessionId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Maximum number of active schedules per session.
pub const MAX_ACTIVE_SCHEDULES_PER_SESSION: u32 = 5;

/// Default minimum seconds between consecutive recurring schedule fires.
pub const DEFAULT_MIN_INTERVAL_SECONDS: i64 = 300;

/// Default maximum number of active (enabled) schedules per org.
pub const DEFAULT_MAX_SCHEDULES_PER_ORG: i64 = 100;

/// Minimum seconds between consecutive recurring session-schedule fires.
///
/// Configurable via `SESSION_SCHEDULE_MIN_INTERVAL_SECONDS`; default 300 (5 min).
/// Each fire dispatches a real worker turn, so on open-signup deployments a
/// `* * * * *` (every-minute) cron is a sustained amplifier of operator compute.
/// This is the session-schedule sibling of the app `schedule` channel's
/// `SCHEDULE_CHANNEL_MIN_INTERVAL_SECONDS` (see `specs/app-invocation-channels.md`).
pub fn min_interval_seconds() -> i64 {
    std::env::var("SESSION_SCHEDULE_MIN_INTERVAL_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_MIN_INTERVAL_SECONDS)
}

/// Maximum number of active (enabled) schedules per org.
///
/// Configurable via `RESOURCE_LIMIT_MAX_SESSION_SCHEDULES_PER_ORG`; default 100.
/// `MAX_ACTIVE_SCHEDULES_PER_SESSION` only bounds a single session, so without an
/// org-wide cap unlimited sessions imply unlimited active schedules per org. Uses
/// the `RESOURCE_LIMIT_*` env family so the SaaS wrapper sets it per plan via
/// `PlanResourceLimits::apply_to_env`.
pub fn max_active_schedules_per_org() -> i64 {
    std::env::var("RESOURCE_LIMIT_MAX_SESSION_SCHEDULES_PER_ORG")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_MAX_SCHEDULES_PER_ORG)
}

/// Returns the minimum interval (seconds) between the next few consecutive
/// triggers of `cron_expression`, or `None` when the expression cannot be parsed
/// or fires fewer than twice.
///
/// Accepts 5-field (`min hour dom mon dow`) and 6/7-field cron forms; 5-field is
/// normalized to the seconds-aware form the `cron` crate expects (sec=0, year=*),
/// mirroring `normalize_cron_expression` in the app schedule channel.
pub fn cron_min_interval_seconds(cron_expression: &str) -> Option<i64> {
    use std::str::FromStr;
    let fields: Vec<&str> = cron_expression.split_whitespace().collect();
    let normalized = match fields.len() {
        5 => format!("0 {} *", fields.join(" ")),
        6 | 7 => cron_expression.to_string(),
        _ => return None,
    };
    let schedule = cron::Schedule::from_str(&normalized).ok()?;
    let upcoming: Vec<_> = schedule.upcoming(chrono::Utc).take(3).collect();
    if upcoming.len() < 2 {
        return None;
    }
    upcoming
        .windows(2)
        .map(|w| (w[1] - w[0]).num_seconds())
        .min()
}

/// Validate that a recurring cron does not fire more often than the configured
/// minimum interval. Returns a user-facing error string when it fires too often.
///
/// Unparseable expressions pass here (return `Ok`) and are rejected later at
/// next-trigger computation, so this gate never false-rejects a valid cron form
/// it does not recognize.
pub fn validate_cron_min_interval(cron_expression: &str) -> Result<(), String> {
    let min_limit = min_interval_seconds();
    if let Some(interval) = cron_min_interval_seconds(cron_expression)
        && interval < min_limit
    {
        return Err(format!(
            "Schedule cron must fire no more than once every {min_limit} seconds (≥ {} min); expression fires every {interval} seconds",
            min_limit / 60
        ));
    }
    Ok(())
}

/// Type of schedule: one-shot or recurring.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ScheduleType {
    /// Fires once at `scheduled_at` then auto-disables.
    OneShot,
    /// Fires on a cron schedule.
    Recurring,
}

/// A session-scoped schedule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionSchedule {
    /// Unique identifier (format: sched_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "sched_01933b5a00007000800000000000001"))]
    pub id: ScheduleId,
    /// Session this schedule belongs to.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "session_01933b5a00007000800000000000001"))]
    pub session_id: SessionId,
    /// Owning principal for this schedule.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "principal_01933b5a000070008000000000000001"))]
    pub owner_principal_id: PrincipalId,
    /// Denormalized effective human owner of the owning principal lineage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_owner_user_id: Option<Uuid>,
    /// Owning principal summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<PrincipalSummary>,
    /// Effective human owner summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_owner: Option<PrincipalSummary>,
    /// What the agent should do when the schedule fires.
    pub description: String,
    /// Cron expression for recurring schedules (None for one-shot).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    /// One-shot trigger time (None for recurring).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_at: Option<DateTime<Utc>>,
    /// IANA timezone for cron interpretation.
    pub timezone: String,
    /// Whether the schedule is active.
    pub enabled: bool,
    /// Computed type based on cron_expression vs scheduled_at.
    pub schedule_type: ScheduleType,
    /// Next computed trigger time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_trigger_at: Option<DateTime<Utc>>,
    /// Last time this schedule fired.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_triggered_at: Option<DateTime<Utc>>,
    /// Total number of times this schedule has fired.
    pub trigger_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SessionSchedule {
    /// Derive schedule type from fields.
    pub fn derive_type(cron_expression: &Option<String>) -> ScheduleType {
        if cron_expression.is_some() {
            ScheduleType::Recurring
        } else {
            ScheduleType::OneShot
        }
    }
}

#[cfg(test)]
mod limit_tests {
    use super::*;

    // Env-backed getters touch process-global state; keep them in one test each
    // and always restore so parallel tests in the crate are not affected.
    #[test]
    fn min_interval_default_is_300() {
        unsafe { std::env::remove_var("SESSION_SCHEDULE_MIN_INTERVAL_SECONDS") };
        assert_eq!(min_interval_seconds(), DEFAULT_MIN_INTERVAL_SECONDS);
    }

    // NOTE: the per-org cap env var (`RESOURCE_LIMIT_MAX_SESSION_SCHEDULES_PER_ORG`)
    // and its enforcement are exercised by the single `create_schedule_enforces_per_org_cap`
    // tool test in `capabilities/session_schedule.rs`. Process env is global and core
    // tests run in parallel, so we deliberately keep exactly one test mutating that
    // var to avoid a set/remove race with a default-reading test.

    #[test]
    fn cron_interval_every_minute_is_60() {
        // 5-field every-minute cron.
        assert_eq!(cron_min_interval_seconds("* * * * *"), Some(60));
    }

    #[test]
    fn cron_interval_every_5_min_is_300() {
        assert_eq!(cron_min_interval_seconds("*/5 * * * *"), Some(300));
    }

    #[test]
    fn cron_interval_six_field_every_30s() {
        // 6-field (seconds) form, every 30 seconds.
        assert_eq!(cron_min_interval_seconds("*/30 * * * * *"), Some(30));
    }

    #[test]
    fn cron_interval_unparseable_is_none() {
        assert_eq!(cron_min_interval_seconds("not a cron"), None);
        assert_eq!(cron_min_interval_seconds("* * *"), None);
    }

    #[test]
    fn validate_rejects_every_minute_at_default() {
        unsafe { std::env::remove_var("SESSION_SCHEDULE_MIN_INTERVAL_SECONDS") };
        assert!(validate_cron_min_interval("* * * * *").is_err());
    }

    #[test]
    fn validate_accepts_daily() {
        unsafe { std::env::remove_var("SESSION_SCHEDULE_MIN_INTERVAL_SECONDS") };
        assert!(validate_cron_min_interval("0 3 * * *").is_ok());
    }

    #[test]
    fn validate_accepts_unparseable() {
        // Unrecognized forms pass the gate; the create path rejects them later.
        assert!(validate_cron_min_interval("garbage").is_ok());
    }
}
