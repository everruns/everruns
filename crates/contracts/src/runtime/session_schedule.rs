// Session schedule domain types
//
// Represents scheduled tasks bound to a session.
// When a schedule fires, a message is injected into the session to trigger a turn.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::runtime::principal::PrincipalSummary;
use crate::runtime::typed_id::{PrincipalId, ScheduleId, SessionId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Maximum number of active schedules per session.
pub const MAX_ACTIVE_SCHEDULES_PER_SESSION: u32 = 5;

/// Default minimum seconds between consecutive recurring schedule fires.
pub const DEFAULT_MIN_INTERVAL_SECONDS: i64 = 300;

/// Default maximum number of active (enabled) schedules per org.
pub const DEFAULT_MAX_SCHEDULES_PER_ORG: i64 = 100;

/// Ceiling on how long a schedule an agent creates may run (2026-11-01, after a
/// test agent's poll ran unattended for 13 days). An org may lower it in
/// Settings → Limits; only a person can extend one schedule past it.
pub const AGENT_SCHEDULE_MAX_LIFETIME_DAYS: u32 = 7;

/// Ceiling on runs for a schedule an agent creates. At the 5 minute minimum
/// interval this is 8 hours, so a longer wait needs a longer interval.
pub const AGENT_SCHEDULE_MAX_RUNS: u32 = 96;

/// Default spend cap of one schedule's turns, in USD.
pub const DEFAULT_SCHEDULE_BUDGET_USD: f64 = 2.0;

/// Largest budget an agent may give one schedule, in USD.
pub const DEFAULT_SCHEDULE_MAX_BUDGET_USD: f64 = 10.0;

/// Default daily cap on unattended work in an org (schedule fires, agent
/// trigger sessions, automatic retries), in USD.
pub const DEFAULT_ORG_UNATTENDED_DAILY_USD: f64 = 20.0;

/// Longest `stop_when` an agent may store.
pub const MAX_STOP_WHEN_CHARS: usize = 500;

/// Limits on background work an org can tune in Settings → Limits.
///
/// Defaults are code constants rather than environment variables, so every
/// deployment starts safe. The lifetime and run limits can only be lowered:
/// [`BackgroundLimits::validate`] rejects values above the agent ceilings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(default)]
pub struct BackgroundLimits {
    /// Longest an agent-created recurring schedule may run, in days (1-7).
    pub schedule_max_lifetime_days: u32,
    /// Most runs an agent-created recurring schedule may have (1-96).
    pub schedule_max_runs: u32,
    /// Budget a schedule gets when the agent does not pass one, in USD.
    pub schedule_budget_usd: f64,
    /// Largest budget an agent may give a schedule, in USD.
    pub schedule_max_budget_usd: f64,
    /// Daily cap on unattended work across the org, in USD. `None` is no cap.
    pub org_unattended_daily_usd: Option<f64>,
}

impl Default for BackgroundLimits {
    fn default() -> Self {
        Self {
            schedule_max_lifetime_days: AGENT_SCHEDULE_MAX_LIFETIME_DAYS,
            schedule_max_runs: AGENT_SCHEDULE_MAX_RUNS,
            schedule_budget_usd: DEFAULT_SCHEDULE_BUDGET_USD,
            schedule_max_budget_usd: DEFAULT_SCHEDULE_MAX_BUDGET_USD,
            org_unattended_daily_usd: Some(DEFAULT_ORG_UNATTENDED_DAILY_USD),
        }
    }
}

impl BackgroundLimits {
    /// Reject values outside what an org may set.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if !(1..=AGENT_SCHEDULE_MAX_LIFETIME_DAYS).contains(&self.schedule_max_lifetime_days) {
            return Err(format!(
                "schedule_max_lifetime_days must be between 1 and {AGENT_SCHEDULE_MAX_LIFETIME_DAYS}"
            ));
        }
        if !(1..=AGENT_SCHEDULE_MAX_RUNS).contains(&self.schedule_max_runs) {
            return Err(format!(
                "schedule_max_runs must be between 1 and {AGENT_SCHEDULE_MAX_RUNS}"
            ));
        }
        let positive = |value: f64| value.is_finite() && value > 0.0;
        if !positive(self.schedule_budget_usd) || !positive(self.schedule_max_budget_usd) {
            return Err("Schedule budgets must be positive amounts in USD".to_string());
        }
        if self.schedule_budget_usd > self.schedule_max_budget_usd {
            return Err(
                "schedule_budget_usd cannot be larger than schedule_max_budget_usd".to_string(),
            );
        }
        if self
            .org_unattended_daily_usd
            .is_some_and(|cap| !positive(cap))
        {
            return Err(
                "org_unattended_daily_usd must be a positive amount in USD, or null for no cap"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// How long a schedule may run and what it may spend.
///
/// Passed to [`crate::runtime::session_services::SessionScheduleStore`]: as
/// requested by the agent to `create_schedule_enforcing_limits`, and resolved
/// (every field the schedule needs filled) to `create_schedule`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScheduleBounds {
    /// The schedule stops firing after this time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<DateTime<Utc>>,
    /// The schedule stops after this many fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u32>,
    /// When the work is done, in the agent's words; repeated on every fire.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_when: Option<String>,
    /// Spend cap of this schedule's turns, in USD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
}

/// Resolve what an agent asked for against the org's limits.
///
/// A recurring schedule must say how long it runs (`ends_at` or `max_runs`)
/// and when it is done (`stop_when`); there is no silent default, so the agent
/// states what it needs. The result always carries both an end time and a run
/// count, whichever comes first stops the schedule. A one-shot schedule fires
/// once, so only its budget is resolved. Every rejection names the limit.
pub fn resolve_agent_schedule_bounds(
    requested: &ScheduleBounds,
    limits: &BackgroundLimits,
    recurring: bool,
    now: DateTime<Utc>,
) -> std::result::Result<ScheduleBounds, String> {
    let budget_usd = match requested.budget_usd {
        None => limits.schedule_budget_usd,
        Some(budget) if !budget.is_finite() || budget <= 0.0 => {
            return Err("budget_usd must be a positive amount in USD".to_string());
        }
        Some(budget) if budget > limits.schedule_max_budget_usd => {
            return Err(format!(
                "budget_usd ${budget:.2} exceeds this org's limit of ${:.2} per schedule. Pass a smaller budget, or ask the user to raise it in Settings → Limits.",
                limits.schedule_max_budget_usd
            ));
        }
        Some(budget) => budget,
    };
    let stop_when = requested
        .stop_when
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    if stop_when
        .as_ref()
        .is_some_and(|text| text.chars().count() > MAX_STOP_WHEN_CHARS)
    {
        return Err(format!(
            "stop_when must be at most {MAX_STOP_WHEN_CHARS} characters"
        ));
    }
    if !recurring {
        return Ok(ScheduleBounds {
            ends_at: None,
            max_runs: Some(1),
            stop_when,
            budget_usd: Some(budget_usd),
        });
    }

    let max_days = limits.schedule_max_lifetime_days;
    let max_runs = limits.schedule_max_runs;
    if requested.ends_at.is_none() && requested.max_runs.is_none() {
        return Err(format!(
            "A recurring schedule must say how long it runs: pass ends_at, max_runs, or both (this org allows at most {max_days} days and {max_runs} runs)."
        ));
    }
    let Some(stop_when) = stop_when else {
        return Err(
            "A recurring schedule must pass stop_when: one sentence on when the work is done, so you can cancel the schedule then."
                .to_string(),
        );
    };
    let latest_end = now + chrono::Duration::days(i64::from(max_days));
    let ends_at = match requested.ends_at {
        Some(ends_at) if ends_at <= now => {
            return Err("ends_at must be in the future".to_string());
        }
        Some(ends_at) if ends_at > latest_end => {
            return Err(format!(
                "ends_at {} is past this org's limit of {max_days} days ({}). Pass an earlier end, or ask the user to raise it in Settings → Limits.",
                ends_at.to_rfc3339(),
                latest_end.to_rfc3339()
            ));
        }
        Some(ends_at) => ends_at,
        None => latest_end,
    };
    let runs = match requested.max_runs {
        Some(0) => return Err("max_runs must be at least 1".to_string()),
        Some(runs) if runs > max_runs => {
            return Err(format!(
                "max_runs {runs} exceeds this org's limit of {max_runs} runs. Use a longer interval or fewer runs, or ask the user to raise it in Settings → Limits."
            ));
        }
        Some(runs) => runs,
        None => max_runs,
    };
    Ok(ScheduleBounds {
        ends_at: Some(ends_at),
        max_runs: Some(runs),
        stop_when: Some(stop_when),
        budget_usd: Some(budget_usd),
    })
}

impl ScheduleBounds {
    /// Read `ends_at`, `max_runs`, `stop_when` and `budget_usd` from a tool
    /// call's arguments. Absent fields stay `None`; resolution decides
    /// whether they were required.
    pub fn from_tool_args(
        args: &serde_json::Map<String, serde_json::Value>,
    ) -> std::result::Result<Self, String> {
        let ends_at = match args.get("ends_at") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(value)) if value.trim().is_empty() => None,
            Some(serde_json::Value::String(value)) => Some(
                DateTime::parse_from_rfc3339(value.trim())
                    .map_err(|_| "ends_at must be an RFC3339 datetime".to_string())?
                    .with_timezone(&Utc),
            ),
            Some(_) => return Err("ends_at must be an RFC3339 datetime string".to_string()),
        };
        let max_runs = match args.get("max_runs") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .and_then(|runs| u32::try_from(runs).ok())
                    .ok_or_else(|| "max_runs must be a positive whole number".to_string())?,
            ),
        };
        let stop_when = match args.get("stop_when") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(text)) => Some(text.clone()),
            Some(_) => return Err("stop_when must be a string".to_string()),
        };
        let budget_usd = match args.get("budget_usd") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_f64()
                    .ok_or_else(|| "budget_usd must be a number in USD".to_string())?,
            ),
        };
        Ok(Self {
            ends_at,
            max_runs,
            stop_when,
            budget_usd,
        })
    }

    /// JSON-schema properties for the bounds arguments, with the platform
    /// ceilings as `maximum` so the model sees them before it calls.
    pub fn tool_schema_properties() -> serde_json::Map<String, serde_json::Value> {
        let serde_json::Value::Object(properties) = serde_json::json!({
            "ends_at": {
                "type": "string",
                "description": format!(
                    "RFC3339 time after which a recurring schedule stops. At most {AGENT_SCHEDULE_MAX_LIFETIME_DAYS} days ahead. A recurring schedule needs ends_at, max_runs, or both."
                )
            },
            "max_runs": {
                "type": "integer",
                "minimum": 1,
                "maximum": AGENT_SCHEDULE_MAX_RUNS,
                "description": format!(
                    "Most times a recurring schedule fires, at most {AGENT_SCHEDULE_MAX_RUNS}. Whichever of ends_at and max_runs comes first stops it."
                )
            },
            "stop_when": {
                "type": "string",
                "maxLength": MAX_STOP_WHEN_CHARS,
                "description": "Required for a recurring schedule: one sentence on when the work is done (e.g. 'the claim is approved, or the claim token has expired'). Repeated on every run so you cancel the schedule then."
            },
            "budget_usd": {
                "type": "number",
                "exclusiveMinimum": 0,
                "description": format!(
                    "Spend cap for this schedule's runs in USD. Default ${DEFAULT_SCHEDULE_BUDGET_USD:.2}, at most ${DEFAULT_SCHEDULE_MAX_BUDGET_USD:.2} unless the org changed it. The schedule stops when it is spent."
                )
            }
        }) else {
            unreachable!("literal object")
        };
        properties
    }
}

/// One sentence stating the default limits, for tool descriptions.
pub fn schedule_limits_summary() -> String {
    format!(
        "Limits: a recurring schedule must pass stop_when and ends_at or max_runs, up to {AGENT_SCHEDULE_MAX_LIFETIME_DAYS} days and {AGENT_SCHEDULE_MAX_RUNS} runs (at the 5 minute minimum interval that is 8 hours, so wait longer with a longer interval). Each schedule's runs may spend ${DEFAULT_SCHEDULE_BUDGET_USD:.2} unless you pass budget_usd (up to ${DEFAULT_SCHEDULE_MAX_BUDGET_USD:.2}). An org may set lower limits in Settings → Limits; an error names the limit that was hit."
    )
}

/// Why a schedule stopped on its own.
pub mod stop_reason {
    /// It reached `ends_at` or `max_runs`.
    pub const ENDED: &str = "ended";
    /// Its budget ran out.
    pub const BUDGET_EXHAUSTED: &str = "budget_exhausted";
    /// Its runs kept ending the same way, so it was achieving nothing.
    pub const RUNAWAY: &str = "runaway";
}

/// Normalize a session-schedule cron expression to the seconds-aware form the
/// `cron` crate parses, or `None` when it has the wrong number of fields.
///
/// Accepts 5-field (`min hour dom mon dow`) and 6/7-field cron forms; 5-field
/// gains sec=0 and year=*. This is more permissive than the app schedule
/// channel's `normalize_cron_expression` (which accepts only 5 or 7 fields):
/// here we also accept the 6-field seconds form the agent may already pass.
///
/// THREAT[TM-SCHED-001]: one normalizer for the interval gate and every next-trigger
/// computation. When they disagreed, a 5-field Sunday cron failed to parse at
/// the gate (and passed as "unrecognized") while the trigger computation
/// accepted it, so `* * * * 0` slipped under the minimum interval.
pub fn normalize_session_cron(cron_expression: &str) -> Option<String> {
    let fields: Vec<&str> = cron_expression.split_whitespace().collect();
    match fields.len() {
        5 => standard_cron_to_crate_form(cron_expression),
        6 | 7 => Some(fields.join(" ")),
        _ => None,
    }
}

/// Rewrite a standard 5-field cron (`min hour dom mon dow`) as the 7-field
/// form the `cron` crate parses, or `None` when it does not have 5 fields.
///
/// Standard cron counts weekdays 0-6 from Sunday (7 is Sunday too); the
/// `cron` crate counts 1-7 from Sunday, so the weekday numbers become day
/// names, which mean the same day to both. Every surface that accepts 5-field
/// cron (session schedules, agent triggers, org schedules) reads it through
/// here so `1-5` is Monday-Friday everywhere.
pub fn standard_cron_to_crate_form(cron_expression: &str) -> Option<String> {
    let fields: Vec<&str> = cron_expression.split_whitespace().collect();
    let [minute, hour, day, month, weekday] = fields[..] else {
        return None;
    };
    Some(format!(
        "0 {minute} {hour} {day} {month} {} *",
        unix_weekdays_to_names(weekday)
    ))
}

/// Rewrite a standard-cron weekday field's numbers (0-7, Sunday first) as
/// day names, keeping lists, ranges, steps, and existing names intact.
fn unix_weekdays_to_names(field: &str) -> String {
    const NAMES: [&str; 8] = ["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];
    let day = |value: &str| match value.parse::<usize>() {
        Ok(index) if index < NAMES.len() => NAMES[index].to_string(),
        _ => value.to_string(),
    };
    field
        .split(',')
        .map(|part| {
            let (range, step) = match part.split_once('/') {
                Some((range, step)) => (range, Some(step)),
                None => (part, None),
            };
            // 7 is Sunday at the end of the week, but as a name it starts
            // the week, so `FRI-SUN` runs backwards and `SUN-SUN` keeps only
            // Sunday. A numeric range ending at 7 is spelled out day by day.
            if let Some((start, "7")) = range.split_once('-')
                && let Ok(start) = start.parse::<usize>()
                && start <= 7
            {
                let step = match step {
                    None => 1,
                    Some(step) => match step.parse::<usize>() {
                        Ok(step) if step > 0 => step,
                        // Leave a malformed step for the parser to reject.
                        _ => return part.to_string(),
                    },
                };
                return (start..=7)
                    .step_by(step)
                    .map(|index| NAMES[index])
                    .collect::<Vec<_>>()
                    .join(",");
            }
            let range = range.split('-').map(day).collect::<Vec<_>>().join("-");
            match step {
                Some(step) => format!("{range}/{step}"),
                None => range,
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Parse the cron contract shared by recurring admission and trigger calculation.
pub fn parse_session_cron(cron_expression: &str) -> Result<cron::Schedule, String> {
    use std::str::FromStr;
    let normalized = normalize_session_cron(cron_expression).ok_or_else(|| {
        format!("Invalid cron expression '{cron_expression}': use 5 fields (minute hour day month weekday), or 6/7 fields with seconds")
    })?;
    cron::Schedule::from_str(&normalized)
        .map_err(|error| format!("Invalid cron expression '{cron_expression}': {error}"))
}

/// Return the minimum interval between the next three occurrences.
/// Invalid cron or fewer than two future occurrences provides no interval evidence.
pub fn cron_min_interval_seconds(cron_expression: &str) -> Result<i64, String> {
    cron_min_interval_seconds_after(cron_expression, Utc::now())
}

fn cron_min_interval_seconds_after(
    cron_expression: &str,
    after: DateTime<Utc>,
) -> Result<i64, String> {
    let schedule = parse_session_cron(cron_expression)?;
    let upcoming: Vec<_> = schedule.after(&after).take(3).collect();
    upcoming
        .windows(2)
        .map(|w| (w[1] - w[0]).num_seconds())
        .min()
        .ok_or_else(|| {
            "Schedule cron must have at least two future occurrences to verify its minimum interval"
                .to_string()
        })
}

/// Validate that a recurring cron does not fire more often than the configured
/// minimum interval. Reject invalid expressions and unavailable interval evidence.
pub fn validate_cron_min_interval(cron_expression: &str) -> Result<(), String> {
    validate_cron_min_interval_with(cron_expression, DEFAULT_MIN_INTERVAL_SECONDS)
}

/// Validate a cron against a host-selected minimum interval.
pub fn validate_cron_min_interval_with(
    cron_expression: &str,
    min_limit: i64,
) -> Result<(), String> {
    // THREAT[TM-SCHED-001]: unknown interval evidence cannot authorize recurrence.
    let interval = cron_min_interval_seconds(cron_expression)?;
    if interval < min_limit {
        return Err(format!(
            "Schedule cron must fire no more than once every {min_limit} seconds (≥ {} min); expression fires every {interval} seconds",
            min_limit / 60
        ));
    }
    Ok(())
}

/// Outcome of a failed session-schedule limit check.
///
/// Distinguishes a store/count failure (surface as an internal error) from a
/// limit rejection (surface as a user-facing tool error) so callers preserve the
/// same behavior they had with the inline checks.
pub enum ScheduleLimitError {
    /// Counting active schedules failed.
    Store(crate::runtime::error::AgentLoopError),
    /// A limit was exceeded; carries the user-facing message.
    Rejected(String),
}

/// Enforce the create-time session-schedule limits shared by every agent entry
/// point (`create_schedule` and `spawn_background` with a `schedule` arg):
/// per-session cap, per-org cap, and minimum recurring cron interval. Pass the
/// recurring `cron_expression` (None for one-shot schedules, which skip the
/// interval gate). Each fire dispatches a real worker turn, so these bound
/// operator compute on open-signup deployments.
pub async fn validate_schedule_create_limits<
    T: crate::runtime::session_services::SessionScheduleStore + ?Sized,
>(
    store: &T,
    session_id: SessionId,
    cron_expression: Option<&str>,
) -> std::result::Result<(), ScheduleLimitError> {
    let per_session = store
        .count_active_schedules(session_id)
        .await
        .map_err(ScheduleLimitError::Store)?;
    if per_session >= MAX_ACTIVE_SCHEDULES_PER_SESSION {
        return Err(ScheduleLimitError::Rejected(format!(
            "Maximum {MAX_ACTIVE_SCHEDULES_PER_SESSION} active schedules per session. Cancel an existing schedule first."
        )));
    }

    let max_per_org = DEFAULT_MAX_SCHEDULES_PER_ORG;
    let per_org = store
        .count_active_org_schedules()
        .await
        .map_err(ScheduleLimitError::Store)?;
    if i64::from(per_org) >= max_per_org {
        return Err(ScheduleLimitError::Rejected(format!(
            "Maximum {max_per_org} active schedules per org reached. Cancel an existing schedule first."
        )));
    }

    if let Some(cron) = cron_expression {
        validate_cron_min_interval(cron).map_err(ScheduleLimitError::Rejected)?;
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
    /// When the schedule was deleted. The row stays so a session recording can
    /// show it; a deleted schedule does not fire and is hidden from the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
    /// The schedule stops firing after this time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at: Option<DateTime<Utc>>,
    /// The schedule stops after this many fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u32>,
    /// When the work is done, in the agent's words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_when: Option<String>,
    /// Why the schedule stopped on its own (`ended`, `budget_exhausted`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// This schedule's spend cap and what its turns spent so far.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<ScheduleBudget>,
}

/// A schedule's spend cap, read from its `session_schedule` budget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScheduleBudget {
    /// Cap in USD.
    pub limit_usd: f64,
    /// Spent so far in USD.
    pub spent_usd: f64,
    /// `active`, `paused`, `exhausted` or `disabled`.
    pub status: String,
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
    use crate::runtime::error::{AgentLoopError, Result};
    use crate::runtime::session_services::SessionScheduleStore;

    #[test]
    fn cron_forms_preserve_literal_intervals() {
        for (expression, seconds) in [
            ("* * * * *", 60),
            ("*/5 * * * *", 300),
            ("*/30 * * * * *", 30),
            ("0 */5 * * * * *", 300),
            ("  */5   * * * *  ", 300),
            ("0 3 * * *", 86_400),
        ] {
            assert_eq!(
                cron_min_interval_seconds(expression),
                Ok(seconds),
                "{expression}"
            );
        }
    }

    /// Standard cron counts weekdays 0-6 from Sunday; the `cron` crate counts
    /// 1-7. A Sunday-only every-minute cron must still hit the interval gate
    /// rather than fail to parse here and slip through as "unrecognized".
    #[test]
    fn unix_weekday_crons_are_interval_checked() {
        assert_eq!(cron_min_interval_seconds("* * * * 0"), Ok(60));
        assert!(validate_cron_min_interval("* * * * 0").is_err());
        assert!(validate_cron_min_interval("*/2 * * * 0,6").is_err());
        assert_eq!(cron_min_interval_seconds("0 18 * * 0"), Ok(7 * 86_400));
        assert_eq!(validate_cron_min_interval("0 18 * * 0"), Ok(()));
    }

    #[test]
    fn five_field_weekdays_become_names() {
        assert_eq!(
            normalize_session_cron("0 18 * * 0").as_deref(),
            Some("0 0 18 * * SUN *")
        );
        assert_eq!(unix_weekdays_to_names("0"), "SUN");
        assert_eq!(unix_weekdays_to_names("7"), "SUN");
        assert_eq!(unix_weekdays_to_names("1-5"), "MON-FRI");
        assert_eq!(unix_weekdays_to_names("0,6"), "SUN,SAT");
        assert_eq!(unix_weekdays_to_names("*/2"), "*/2");
        assert_eq!(unix_weekdays_to_names("MON"), "MON");
        assert_eq!(unix_weekdays_to_names("5-7"), "FRI,SAT,SUN");
        assert_eq!(unix_weekdays_to_names("1-7/2"), "MON,WED,FRI,SUN");
        assert_eq!(unix_weekdays_to_names("7-7"), "SUN");
        assert_eq!(unix_weekdays_to_names("5-7/0"), "5-7/0");
        assert_eq!(
            normalize_session_cron(" 0 0 7 * * * ").as_deref(),
            Some("0 0 7 * * *")
        );
        assert_eq!(normalize_session_cron("* * *"), None);
    }

    /// Standard cron allows a weekday range to end at 7 (Sunday as the last
    /// day of the week). As names that range runs backwards (`FRI-SUN`) or
    /// collapses (`SUN-SUN`), so `5-7` was rejected and `0-7`, which is every
    /// day, fired only on Sundays.
    #[test]
    fn weekday_ranges_ending_at_seven_keep_their_days() {
        use chrono::{Datelike, TimeZone, Weekday};
        use std::str::FromStr;
        let days = |cron: &str| {
            let normalized = normalize_session_cron(cron).unwrap();
            let schedule = cron::Schedule::from_str(&normalized)
                .unwrap_or_else(|error| panic!("{cron} as {normalized}: {error}"));
            // A Monday; the next seven runs of a daily-or-sparser cron.
            let after = Utc.with_ymd_and_hms(2026, 10, 12, 12, 0, 0).unwrap();
            let mut days: Vec<_> = schedule
                .after(&after)
                .take(7)
                .map(|t| t.weekday())
                .collect();
            days.sort_by_key(|day| day.num_days_from_monday());
            days.dedup();
            days
        };
        use Weekday::*;
        assert_eq!(
            days("0 10 * * 0-7"),
            vec![Mon, Tue, Wed, Thu, Fri, Sat, Sun]
        );
        assert_eq!(days("0 10 * * 5-7"), vec![Fri, Sat, Sun]);
        assert_eq!(days("0 10 * * 6-7"), vec![Sat, Sun]);
        assert_eq!(days("0 10 * * 1-3,6-7"), vec![Mon, Tue, Wed, Sat, Sun]);
        assert_eq!(days("0 10 * * 7-7"), vec![Sun]);
        assert_eq!(days("0 10 * * 1-7/2"), vec![Mon, Wed, Fri, Sun]);
        assert_eq!(days("0 10 * * 1-5"), vec![Mon, Tue, Wed, Thu, Fri]);
    }

    #[test]
    fn invalid_or_exhausted_cron_is_rejected_at_admission() {
        for expression in [
            "not a cron",
            "* * *",
            "* * * * * * * *",
            "99 * * * *",
            "0 0 0 1 1 * 2000",
        ] {
            assert!(
                cron_min_interval_seconds(expression).is_err(),
                "{expression}"
            );
            assert!(
                validate_cron_min_interval(expression).is_err(),
                "{expression}"
            );
        }
    }

    #[test]
    fn canonical_parser_preserves_compliant_weekdays_and_cron_forms() {
        use chrono::TimeZone;
        let after = Utc.with_ymd_and_hms(2026, 10, 12, 0, 0, 0).unwrap();
        for expression in [
            "0 9 * * 1-5",
            "0 9 * * MON-FRI",
            "0 0 9 * * MON-FRI",
            "0 0 9 * * MON-FRI *",
        ] {
            let parsed = parse_session_cron(expression).unwrap();
            assert_eq!(
                parsed.after(&after).next(),
                Some(Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap())
            );
            assert_eq!(
                cron_min_interval_seconds_after(expression, after),
                Ok(86_400)
            );
        }
    }

    #[test]
    fn interval_evidence_requires_two_future_calendar_occurrences() {
        use chrono::TimeZone;
        let expression = "0 0 9 1,2 1 * 2027";
        let before = Utc.with_ymd_and_hms(2026, 12, 31, 0, 0, 0).unwrap();
        assert_eq!(
            cron_min_interval_seconds_after(expression, before),
            Ok(86_400)
        );
        let between = Utc.with_ymd_and_hms(2027, 1, 1, 10, 0, 0).unwrap();
        let exhausted = Utc.with_ymd_and_hms(2027, 1, 3, 0, 0, 0).unwrap();
        for after in [between, exhausted] {
            assert_eq!(
                cron_min_interval_seconds_after(expression, after).unwrap_err(),
                "Schedule cron must have at least two future occurrences to verify its minimum interval"
            );
        }
    }

    #[test]
    fn interval_gate_enforces_default_and_custom_inclusive_boundaries() {
        assert_eq!(validate_cron_min_interval("* * * * *"), Err("Schedule cron must fire no more than once every 300 seconds (≥ 5 min); expression fires every 60 seconds".into()));
        assert_eq!(validate_cron_min_interval("*/5 * * * *"), Ok(()));
        assert_eq!(validate_cron_min_interval("0 3 * * *"), Ok(()));
        assert_eq!(validate_cron_min_interval_with("* * * * *", 59), Ok(()));
        assert_eq!(validate_cron_min_interval_with("* * * * *", 60), Ok(()));
        assert_eq!(validate_cron_min_interval_with("* * * * *", 61), Err("Schedule cron must fire no more than once every 61 seconds (≥ 1 min); expression fires every 60 seconds".into()));
    }

    struct Counts {
        session: std::result::Result<u32, &'static str>,
        org: std::result::Result<u32, &'static str>,
    }

    #[async_trait::async_trait]
    impl SessionScheduleStore for Counts {
        async fn create_schedule(
            &self,
            _: SessionId,
            _: String,
            _: Option<String>,
            _: Option<DateTime<Utc>>,
            _: String,
        ) -> Result<SessionSchedule> {
            panic!("validation must not create schedules")
        }
        async fn cancel_schedule(&self, _: SessionId, _: ScheduleId) -> Result<SessionSchedule> {
            panic!("validation must not cancel schedules")
        }
        async fn list_schedules(&self, _: SessionId) -> Result<Vec<SessionSchedule>> {
            panic!("validation must use counts rather than fetching schedules")
        }
        async fn count_active_schedules(&self, session_id: SessionId) -> Result<u32> {
            assert_eq!(session_id, SessionId::from_seed(9));
            self.session.map_err(AgentLoopError::store)
        }
        async fn count_active_org_schedules(&self) -> Result<u32> {
            self.org.map_err(AgentLoopError::store)
        }
    }

    #[tokio::test]
    async fn create_limits_enforce_independent_session_org_and_cron_caps() {
        let session = SessionId::from_seed(9);
        for (per_session, per_org, cron, expected) in [
            (4, 99, None, None),
            (4, 99, Some("*/5 * * * *"), None),
            (
                5,
                0,
                None,
                Some("Maximum 5 active schedules per session. Cancel an existing schedule first."),
            ),
            (
                6,
                100,
                None,
                Some("Maximum 5 active schedules per session. Cancel an existing schedule first."),
            ),
            (
                0,
                100,
                None,
                Some(
                    "Maximum 100 active schedules per org reached. Cancel an existing schedule first.",
                ),
            ),
            (
                0,
                101,
                None,
                Some(
                    "Maximum 100 active schedules per org reached. Cancel an existing schedule first.",
                ),
            ),
            (
                0,
                0,
                Some("* * * * *"),
                Some(
                    "Schedule cron must fire no more than once every 300 seconds (≥ 5 min); expression fires every 60 seconds",
                ),
            ),
        ] {
            let store = Counts {
                session: Ok(per_session),
                org: Ok(per_org),
            };
            match (
                validate_schedule_create_limits(&store, session, cron).await,
                expected,
            ) {
                (Ok(()), None) => {}
                (Err(ScheduleLimitError::Rejected(message)), Some(expected)) => {
                    assert_eq!(message, expected)
                }
                _ => panic!("unexpected result for {per_session}/{per_org}/{cron:?}"),
            }
        }
    }

    #[tokio::test]
    async fn create_limits_reject_unknown_interval_without_insertion() {
        let store = Counts {
            session: Ok(0),
            org: Ok(0),
        };
        for expression in ["not a cron", "0 0 0 1 1 * 2000"] {
            assert!(matches!(
                validate_schedule_create_limits(&store, SessionId::from_seed(9), Some(expression))
                    .await,
                Err(ScheduleLimitError::Rejected(_))
            ));
        }
    }

    #[tokio::test]
    async fn count_failures_remain_store_errors_and_session_rejection_wins() {
        let session = SessionId::from_seed(9);
        for store in [
            Counts {
                session: Err("session unavailable"),
                org: Ok(0),
            },
            Counts {
                session: Ok(0),
                org: Err("org unavailable"),
            },
        ] {
            let expected = store.session.err().or(store.org.err()).unwrap();
            match validate_schedule_create_limits(&store, session, None).await {
                Err(ScheduleLimitError::Store(error)) => {
                    assert_eq!(
                        error.to_string(),
                        format!("Message store error: {expected}")
                    )
                }
                _ => panic!("count failure must remain a store error"),
            }
        }
        let store = Counts {
            session: Ok(5),
            org: Err("must not mask session limit"),
        };
        assert!(
            matches!(validate_schedule_create_limits(&store, session, None).await, Err(ScheduleLimitError::Rejected(message)) if message == "Maximum 5 active schedules per session. Cancel an existing schedule first.")
        );
    }
}

#[cfg(test)]
mod bounds_tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap()
    }

    fn recurring(bounds: ScheduleBounds) -> std::result::Result<ScheduleBounds, String> {
        resolve_agent_schedule_bounds(&bounds, &BackgroundLimits::default(), true, now())
    }

    #[test]
    fn recurring_schedule_must_state_its_end_and_stop_condition() {
        let error = recurring(ScheduleBounds {
            stop_when: Some("done".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(error.contains("7 days and 96 runs"), "{error}");

        let error = recurring(ScheduleBounds {
            max_runs: Some(10),
            stop_when: Some("   ".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(error.contains("stop_when"), "{error}");
    }

    #[test]
    fn recurring_schedule_gets_both_ends_and_the_default_budget() {
        let bounds = recurring(ScheduleBounds {
            max_runs: Some(24),
            stop_when: Some(" the claim is approved ".into()),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(bounds.max_runs, Some(24));
        assert_eq!(bounds.ends_at, Some(now() + chrono::Duration::days(7)));
        assert_eq!(bounds.stop_when.as_deref(), Some("the claim is approved"));
        assert_eq!(bounds.budget_usd, Some(DEFAULT_SCHEDULE_BUDGET_USD));

        let end = now() + chrono::Duration::hours(3);
        let bounds = recurring(ScheduleBounds {
            ends_at: Some(end),
            stop_when: Some("done".into()),
            budget_usd: Some(5.0),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(bounds.ends_at, Some(end));
        assert_eq!(bounds.max_runs, Some(AGENT_SCHEDULE_MAX_RUNS));
        assert_eq!(bounds.budget_usd, Some(5.0));
    }

    #[test]
    fn requests_over_the_limits_name_the_limit() {
        let error = recurring(ScheduleBounds {
            max_runs: Some(500),
            stop_when: Some("done".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(error.contains("limit of 96 runs"), "{error}");
        assert!(error.contains("Settings → Limits"), "{error}");

        let error = recurring(ScheduleBounds {
            ends_at: Some(now() + chrono::Duration::days(8)),
            stop_when: Some("done".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(error.contains("7 days"), "{error}");

        let error = recurring(ScheduleBounds {
            max_runs: Some(3),
            stop_when: Some("done".into()),
            budget_usd: Some(11.0),
            ..Default::default()
        })
        .unwrap_err();
        assert!(error.contains("$10.00"), "{error}");

        assert!(
            recurring(ScheduleBounds {
                ends_at: Some(now() - chrono::Duration::minutes(1)),
                stop_when: Some("done".into()),
                ..Default::default()
            })
            .is_err()
        );
    }

    #[test]
    fn org_limits_apply_instead_of_the_defaults() {
        let limits = BackgroundLimits {
            schedule_max_runs: 10,
            schedule_budget_usd: 1.0,
            ..Default::default()
        };
        let request = ScheduleBounds {
            max_runs: Some(20),
            stop_when: Some("done".into()),
            ..Default::default()
        };
        let error = resolve_agent_schedule_bounds(&request, &limits, true, now()).unwrap_err();
        assert!(error.contains("limit of 10 runs"), "{error}");
        let bounds = resolve_agent_schedule_bounds(
            &ScheduleBounds {
                max_runs: Some(5),
                ..request
            },
            &limits,
            true,
            now(),
        )
        .unwrap();
        assert_eq!(bounds.budget_usd, Some(1.0));
    }

    #[test]
    fn one_shot_fires_once_without_a_stop_condition() {
        let bounds = resolve_agent_schedule_bounds(
            &ScheduleBounds::default(),
            &BackgroundLimits::default(),
            false,
            now(),
        )
        .unwrap();
        assert_eq!(bounds.max_runs, Some(1));
        assert_eq!(bounds.budget_usd, Some(DEFAULT_SCHEDULE_BUDGET_USD));
    }

    #[test]
    fn org_limits_cannot_exceed_the_ceilings() {
        assert!(BackgroundLimits::default().validate().is_ok());
        for limits in [
            BackgroundLimits {
                schedule_max_lifetime_days: 8,
                ..Default::default()
            },
            BackgroundLimits {
                schedule_max_runs: 97,
                ..Default::default()
            },
            BackgroundLimits {
                schedule_budget_usd: 20.0,
                ..Default::default()
            },
            BackgroundLimits {
                org_unattended_daily_usd: Some(0.0),
                ..Default::default()
            },
        ] {
            assert!(limits.validate().is_err(), "{limits:?}");
        }
        assert!(
            BackgroundLimits {
                org_unattended_daily_usd: None,
                ..Default::default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn tool_args_parse_and_reject_bad_types() {
        let args = serde_json::json!({
            "ends_at": "2026-11-02T00:00:00Z",
            "max_runs": 12,
            "stop_when": "done",
            "budget_usd": 3.5
        });
        let bounds = ScheduleBounds::from_tool_args(args.as_object().unwrap()).unwrap();
        assert_eq!(bounds.max_runs, Some(12));
        assert_eq!(bounds.budget_usd, Some(3.5));
        assert!(bounds.ends_at.is_some());
        let bad = serde_json::json!({"max_runs": -1});
        assert!(ScheduleBounds::from_tool_args(bad.as_object().unwrap()).is_err());
        let bad = serde_json::json!({"ends_at": "tomorrow"});
        assert!(ScheduleBounds::from_tool_args(bad.as_object().unwrap()).is_err());
    }
}
