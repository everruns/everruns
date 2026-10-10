//! Payloads for timers kept in the session log.
//!
//! A timer is a journal entry: `timer.set` records when the session wants to
//! wake, `timer.fired` each occurrence that was delivered, and
//! `timer.cancelled` its end. Anything that indexes due work (a schedule
//! table, a runnable queue) is rebuilt from these entries, so an index lost or
//! left behind by a crash is repaired by reading the log again.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// What a timer wakes the session for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub enum TimerPurpose {
    /// A session schedule: each occurrence sends its description to the session.
    Schedule,
}

/// Data for `timer.set`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TimerSetData {
    /// Timer identity. For a schedule, the schedule id.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "sched_01933b5a00007000800000000000001")
    )]
    pub timer_id: String,
    /// What the timer wakes the session for.
    pub purpose: TimerPurpose,
    /// First time the timer is due.
    pub fire_at: DateTime<Utc>,
    /// Cron expression for a recurring timer; absent for a one-shot timer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cron_expression: Option<String>,
    /// IANA timezone the cron expression is read in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "UTC"))]
    pub timezone: Option<String>,
    /// Text delivered to the session when the timer fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Host-defined fields kept with the timer (for example a schedule's name).
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub metadata: serde_json::Value,
}

/// Data for `timer.fired`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TimerFiredData {
    /// Timer that fired.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "sched_01933b5a00007000800000000000001")
    )]
    pub timer_id: String,
    /// When the occurrence was delivered.
    pub fired_at: DateTime<Utc>,
    /// Next time the timer is due; absent when this was its last occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_fire_at: Option<DateTime<Utc>>,
}

/// Data for `timer.cancelled`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TimerCancelledData {
    /// Timer that will not fire again.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "sched_01933b5a00007000800000000000001")
    )]
    pub timer_id: String,
}
