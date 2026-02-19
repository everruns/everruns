// Session schedule domain types
//
// Represents scheduled tasks bound to a session.
// When a schedule fires, a message is injected into the session to trigger a turn.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::{ScheduleId, SessionId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Maximum number of active schedules per session.
pub const MAX_ACTIVE_SCHEDULES_PER_SESSION: u32 = 5;

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
