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

/// Returns the minimum interval (seconds) between the next few consecutive
/// triggers of `cron_expression`, or `None` when the expression cannot be parsed
/// or fires fewer than twice.
///
/// Accepts 5-field (`min hour dom mon dow`) and 6/7-field cron forms; 5-field is
/// normalized to the seconds-aware form the `cron` crate expects (sec=0, year=*).
/// This is more permissive than the app schedule channel's
/// `normalize_cron_expression` (which accepts only 5 or 7 fields): here we also
/// accept the 6-field seconds form the agent may already pass through to the store.
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
    validate_cron_min_interval_with(cron_expression, DEFAULT_MIN_INTERVAL_SECONDS)
}

/// Validate a cron against a host-selected minimum interval.
pub fn validate_cron_min_interval_with(
    cron_expression: &str,
    min_limit: i64,
) -> Result<(), String> {
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

/// Outcome of a failed session-schedule limit check.
///
/// Distinguishes a store/count failure (surface as an internal error) from a
/// limit rejection (surface as a user-facing tool error) so callers preserve the
/// same behavior they had with the inline checks.
pub enum ScheduleLimitError {
    /// Counting active schedules failed.
    Store(crate::error::AgentLoopError),
    /// A limit was exceeded; carries the user-facing message.
    Rejected(String),
}

/// Enforce the create-time session-schedule limits shared by every agent entry
/// point (`create_schedule` and `spawn_background` with a `schedule` arg):
/// per-session cap, per-org cap, and minimum recurring cron interval. Pass the
/// recurring `cron_expression` (None for one-shot schedules, which skip the
/// interval gate). Each fire dispatches a real worker turn, so these bound
/// operator compute on open-signup deployments (see `knowledge/security/threat-model.md`
/// TM-SCHED-001).
pub async fn validate_schedule_create_limits<
    T: crate::session_services::SessionScheduleStore + ?Sized,
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
    use crate::error::{AgentLoopError, Result};
    use crate::session_services::SessionScheduleStore;

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
                Some(seconds),
                "{expression}"
            );
        }
    }

    #[test]
    fn unrecognized_or_exhausted_cron_has_no_interval_and_defers_validation() {
        for expression in [
            "not a cron",
            "* * *",
            "* * * * * * * *",
            "99 * * * *",
            "0 0 0 1 1 * 2000",
        ] {
            assert_eq!(cron_min_interval_seconds(expression), None, "{expression}");
            assert_eq!(validate_cron_min_interval(expression), Ok(()));
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
