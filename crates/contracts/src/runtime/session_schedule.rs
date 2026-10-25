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
