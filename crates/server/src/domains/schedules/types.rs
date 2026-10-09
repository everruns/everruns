// Schedules domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use chrono::{DateTime, Utc};
use everruns_durable::{
    ScheduleExecutionRow, ScheduleExecutionStatus, ScheduleRow, ScheduleStats, ScheduleTargetType,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Create schedule request
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateScheduleRequest {
    /// Unique name for the schedule
    #[schema(example = "nightly-triage")]
    pub name: String,
    /// Optional description
    #[schema(example = "Fires the support-triage agent every night at 02:00 UTC")]
    pub description: Option<String>,
    /// Cron expression (5-field or 7-field). Standard `min hour day-of-month month day-of-week` form.
    #[schema(example = "0 2 * * *")]
    pub cron_expression: String,
    /// Timezone (default: UTC). IANA name (e.g. `UTC`, `America/New_York`).
    #[serde(default = "default_timezone")]
    #[schema(example = "UTC")]
    pub timezone: String,
    /// Target to trigger. Variant shape is defined on `ScheduleTarget`.
    pub target: ScheduleTarget,
    /// Whether schedule is enabled (default: true)
    #[serde(default = "default_enabled")]
    #[schema(example = true)]
    pub enabled: bool,
    /// Max concurrent executions. Omit for no limit beyond the worker pool's concurrency.
    #[schema(example = 1)]
    pub max_concurrent: Option<u32>,
    /// Whether to catch up missed triggers (default: false)
    #[serde(default)]
    #[schema(example = false)]
    pub catch_up_missed: bool,
    /// Max catch-up executions when `catch_up_missed` is true. Older missed fires are dropped.
    #[schema(example = 3)]
    pub max_catch_up: Option<u32>,
    /// Retry policy for failed executions (provider-specific JSON; see the durable engine's `RetryPolicy`).
    /// Example: `{"max_attempts": 3, "initial_backoff_secs": 30, "backoff_multiplier": 2.0}`.
    pub retry_policy: Option<serde_json::Value>,
}

/// Query parameters for listing executions of a schedule — optional status
/// filter plus offset/limit paging.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListExecutionsQuery {
    /// Filter by execution status
    #[schema(example = "completed")]
    pub status: Option<String>,
    /// Pagination offset
    #[schema(example = 0)]
    pub offset: Option<u32>,
    /// Pagination limit (default: 20, max: 100)
    #[schema(example = 20)]
    pub limit: Option<u32>,
}

/// Query parameters for `GET /v1/schedules` — optional enabled/target-type
/// filters plus standard offset/limit paging.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSchedulesQuery {
    /// Filter by enabled status
    #[schema(example = true)]
    pub enabled: Option<bool>,
    /// Filter by target type ("workflow" or "activity")
    #[schema(example = "workflow")]
    pub target_type: Option<String>,
    /// Pagination offset
    #[schema(example = 0)]
    pub offset: Option<u32>,
    /// Pagination limit (default: 20, max: 100)
    #[schema(example = 20)]
    pub limit: Option<u32>,
}

/// Schedule execution response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleExecutionResponse {
    /// UUID of the schedule execution.
    #[schema(example = "01933b5b-0000-7000-8000-000000000001")]
    pub id: Uuid,
    /// UUID of the owning schedule.
    #[schema(example = "01933b5a-0000-7000-8000-000000000001")]
    pub schedule_id: Uuid,
    /// Timestamp when this resource is scheduled to run (RFC 3339).
    #[schema(example = "2026-05-25T02:00:00Z")]
    pub scheduled_at: DateTime<Utc>,
    /// Timestamp when this resource started, if any (RFC 3339).
    #[schema(example = "2026-05-25T02:00:01Z")]
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Timestamp when this resource completed, if any (RFC 3339).
    #[schema(example = "2026-05-25T02:00:12Z")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Current lifecycle status (`pending`, `running`, `completed`, `failed`, `skipped`).
    #[schema(example = "completed")]
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Durable workflow's identifier.
    #[schema(example = "01933b5c-0000-7000-8000-000000000001")]
    pub workflow_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Durable task's identifier.
    #[schema(example = "01933b5d-0000-7000-8000-000000000001")]
    pub task_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable error message, populated when this resource is in a failed state.
    #[schema(example = "workflow.exec.timeout: activity exceeded 30s budget")]
    pub error: Option<String>,
    /// Total execution duration in milliseconds (`completed_at - started_at`). `None` while still running.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 11000)]
    pub duration_ms: Option<i32>,
    /// Timestamp when this resource was created (RFC 3339).
    #[schema(example = "2026-05-25T02:00:00Z")]
    pub created_at: DateTime<Utc>,
}

impl From<ScheduleExecutionRow> for ScheduleExecutionResponse {
    fn from(e: ScheduleExecutionRow) -> Self {
        let status = match e.status {
            ScheduleExecutionStatus::Pending => "pending",
            ScheduleExecutionStatus::Running => "running",
            ScheduleExecutionStatus::Completed => "completed",
            ScheduleExecutionStatus::Failed => "failed",
            ScheduleExecutionStatus::Skipped => "skipped",
        };

        Self {
            id: e.id,
            schedule_id: e.schedule_id,
            scheduled_at: e.scheduled_at,
            started_at: e.started_at,
            completed_at: e.completed_at,
            status: status.to_string(),
            workflow_id: e.workflow_id,
            task_id: e.task_id,
            error: e.error,
            duration_ms: e.duration_ms,
            created_at: e.created_at,
        }
    }
}

/// Schedule executions list response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleExecutionsListResponse {
    /// Page of items returned by this query.
    pub data: Vec<ScheduleExecutionResponse>,
    /// Total number of items matching the query, across all pages.
    #[schema(example = 142)]
    pub total: usize,
}

/// Schedule response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleResponse {
    /// UUID of the schedule.
    #[schema(example = "01933b5a-0000-7000-8000-000000000001")]
    pub id: Uuid,
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "nightly-triage")]
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    #[schema(example = "Fires the support-triage agent every night at 02:00 UTC")]
    pub description: Option<String>,
    /// Cron expression in the standard `min hour day-of-month month day-of-week` form (6 fields with seconds optional). Evaluated in `timezone`.
    #[schema(example = "0 2 * * *")]
    pub cron_expression: String,
    /// IANA timezone name used to interpret `cron_expression` (e.g. `UTC`, `America/New_York`).
    #[schema(example = "UTC")]
    pub timezone: String,
    /// What the schedule invokes when it fires (a session, an agent, an app channel, etc.).
    pub target: ScheduleTargetResponse,
    /// When `false`, the schedule is paused — kept in storage but never fires until re-enabled.
    #[schema(example = true)]
    pub enabled: bool,
    /// Maximum number of overlapping executions allowed. `None` means no limit beyond the worker pool's concurrency.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 1)]
    pub max_concurrent: Option<u32>,
    /// When `true`, missed fires (while the scheduler was down, paused, or unreachable) are queued and run after recovery, subject to `max_catch_up`.
    #[schema(example = false)]
    pub catch_up_missed: bool,
    /// Maximum number of missed fires to replay when `catch_up_missed` is `true`. Older missed fires are dropped. `None` means no cap.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 3)]
    pub max_catch_up: Option<u32>,
    /// Optional retry policy for failed runs (provider-specific JSON; see the durable engine's `RetryPolicy`).
    /// Example: `{"max_attempts": 3, "initial_backoff_secs": 30, "backoff_multiplier": 2.0}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_policy: Option<serde_json::Value>,
    /// Timestamp of the most recent fire (RFC 3339). `None` if never triggered.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-05-25T02:00:00Z")]
    pub last_triggered_at: Option<DateTime<Utc>>,
    /// Timestamp of the next scheduled fire (RFC 3339). `None` when the schedule is disabled or the cron expression has no upcoming match.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "2026-05-26T02:00:00Z")]
    pub next_trigger_at: Option<DateTime<Utc>>,
    /// Timestamp when this resource was created (RFC 3339).
    #[schema(example = "2026-05-01T12:00:00Z")]
    pub created_at: DateTime<Utc>,
    /// Timestamp when this resource was last updated (RFC 3339).
    #[schema(example = "2026-05-20T12:00:00Z")]
    pub updated_at: DateTime<Utc>,
}

impl From<ScheduleRow> for ScheduleResponse {
    fn from(s: ScheduleRow) -> Self {
        let target_type = match s.target_type {
            ScheduleTargetType::Workflow => "workflow",
            ScheduleTargetType::Activity => "activity",
        };

        Self {
            id: s.id,
            name: s.name,
            description: s.description,
            cron_expression: s.cron_expression,
            timezone: s.timezone,
            target: ScheduleTargetResponse {
                target_type: target_type.to_string(),
                name: s.target_name,
                input: s.target_input,
            },
            enabled: s.enabled,
            max_concurrent: s.max_concurrent,
            catch_up_missed: s.catch_up_missed,
            max_catch_up: s.max_catch_up,
            retry_policy: s.retry_policy,
            last_triggered_at: s.last_triggered_at,
            next_trigger_at: s.next_trigger_at,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

/// Schedule stats response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleStatsResponse {
    /// Total number of executions recorded for this schedule (sum of successful + failed + skipped).
    #[schema(example = 142)]
    pub total_executions: u64,
    /// Count of executions that completed successfully.
    #[schema(example = 137)]
    pub successful_executions: u64,
    /// Count of executions that ended in failure (after exhausting retries).
    #[schema(example = 3)]
    pub failed_executions: u64,
    /// Count of fires that were intentionally skipped (e.g. blocked by `max_concurrent` or disabled mid-fire).
    #[schema(example = 2)]
    pub skipped_executions: u64,
    /// Average execution duration in milliseconds across `total_executions`. `None` when no executions exist yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 8421)]
    pub avg_duration_ms: Option<u64>,
    /// Status of the most recent execution (one of the `ScheduleExecutionResponse.status` values:
    /// `pending`, `running`, `completed`, `failed`, `skipped`). `None` if the schedule has never fired.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "completed")]
    pub last_execution_status: Option<String>,
}

impl From<ScheduleStats> for ScheduleStatsResponse {
    fn from(s: ScheduleStats) -> Self {
        let last_status = s.last_execution_status.map(|st| {
            match st {
                ScheduleExecutionStatus::Pending => "pending",
                ScheduleExecutionStatus::Running => "running",
                ScheduleExecutionStatus::Completed => "completed",
                ScheduleExecutionStatus::Failed => "failed",
                ScheduleExecutionStatus::Skipped => "skipped",
            }
            .to_string()
        });

        Self {
            total_executions: s.total_executions,
            successful_executions: s.successful_executions,
            failed_executions: s.failed_executions,
            skipped_executions: s.skipped_executions,
            avg_duration_ms: s.avg_duration_ms,
            last_execution_status: last_status,
        }
    }
}

/// Target for a schedule - either a workflow or activity
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
#[schema(example = json!({"type": "workflow", "name": "session.run", "input": {"session_id": "session_01933b5a00007000800000000000001"}}))]
pub struct ScheduleTarget {
    /// Target type: "workflow" or "activity"
    #[serde(rename = "type")]
    #[schema(example = "workflow")]
    pub target_type: String,
    /// Workflow type name or activity type name
    #[schema(example = "session.run")]
    pub name: String,
    /// Input JSON for the workflow/activity
    #[serde(default)]
    #[schema(example = json!({"session_id": "session_01933b5a00007000800000000000001"}))]
    pub input: serde_json::Value,
}

/// Schedule target response
#[derive(Debug, Serialize, ToSchema)]
pub struct ScheduleTargetResponse {
    /// Target type discriminator (`workflow` or `activity`).
    #[serde(rename = "type")]
    #[schema(example = "workflow")]
    pub target_type: String,
    /// Human-readable name. Safe to render in user-facing messages.
    #[schema(example = "session.run")]
    pub name: String,
    /// Input JSON payload passed to the workflow/activity on each fire.
    /// Example: `{"session_id": "session_01933b5a000070008000000000000001"}`.
    #[schema(value_type = Object)]
    pub input: serde_json::Value,
}

/// Schedules list response
#[derive(Debug, Serialize, ToSchema)]
pub struct SchedulesListResponse {
    /// Page of items returned by this query.
    pub data: Vec<ScheduleResponse>,
    /// Total number of items matching the query, across all pages.
    #[schema(example = 17)]
    pub total: u64,
}

/// Manual trigger response
#[derive(Debug, Serialize, ToSchema)]
pub struct TriggerResponse {
    /// Schedule execution's identifier.
    pub execution_id: Uuid,
}

/// Update schedule request
#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateScheduleRequest {
    /// New description
    #[schema(example = "Fires the support-triage agent every weekday at 08:00 UTC")]
    pub description: Option<String>,
    /// New cron expression. Standard `min hour day-of-month month day-of-week` form.
    #[schema(example = "0 8 * * 1-5")]
    pub cron_expression: Option<String>,
    /// New timezone (IANA name).
    #[schema(example = "America/New_York")]
    pub timezone: Option<String>,
    /// New target. Variant shape is defined on `ScheduleTarget`.
    pub target: Option<ScheduleTarget>,
    /// Enable/disable
    #[schema(example = true)]
    pub enabled: Option<bool>,
    /// Max concurrent executions
    #[schema(example = 1)]
    pub max_concurrent: Option<u32>,
    /// Catch up missed triggers
    #[schema(example = true)]
    pub catch_up_missed: Option<bool>,
    /// Max catch-up executions
    #[schema(example = 3)]
    pub max_catch_up: Option<u32>,
    /// Retry policy (provider-specific JSON; see the durable engine's `RetryPolicy`).
    /// Example: `{"max_attempts": 3, "initial_backoff_secs": 30, "backoff_multiplier": 2.0}`.
    pub retry_policy: Option<serde_json::Value>,
}

fn default_timezone() -> String {
    "UTC".to_string()
}

fn default_enabled() -> bool {
    true
}
