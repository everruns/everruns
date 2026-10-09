// Rows the schedules repository reads and writes.

use crate::kernel_imports::contracts::typed_id::{
    LeasedResourceId, PrincipalId, ScheduleId, SessionId,
};
use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// How long a session schedule claim is honoured before another instance may
/// take it over.
///
/// Short on purpose: the claim only has to survive until the poller advances
/// `next_trigger_at`, which is the first thing it does with a claimed row. A
/// long lease would instead mean an instance that died mid-fire stranded its
/// schedules for that long.
pub const SESSION_SCHEDULE_CLAIM_LEASE_SECONDS: i32 = 30;

#[derive(Debug, Clone, FromRow, serde::Serialize)]
pub struct SessionScheduleRow {
    pub id: ScheduleId,
    pub public_id: String,
    pub org_id: i64,
    pub session_id: SessionId,
    pub owner_principal_id: PrincipalId,
    #[sqlx(default)]
    pub resolved_owner_user_id: Option<Uuid>,
    pub description: String,
    pub cron_expression: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub enabled: bool,
    pub next_trigger_at: Option<DateTime<Utc>>,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub trigger_count: i32,
    /// Scheduler instance holding the current claim lease, if any.
    #[sqlx(default)]
    pub claimed_by: Option<String>,
    /// When the current claim was taken. A claim older than the claim lease
    /// (see `SESSION_SCHEDULE_CLAIM_LEASE_SECONDS`) is reclaimable by any
    /// instance, so an instance that dies mid-fire does not strand a schedule.
    #[sqlx(default)]
    pub claimed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateSessionScheduleRow {
    pub org_id: i64,
    pub session_id: SessionId,
    pub owner_principal_id: PrincipalId,
    pub resolved_owner_user_id: Option<Uuid>,
    pub description: String,
    pub cron_expression: Option<String>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub timezone: String,
    pub next_trigger_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateSessionScheduleRow {
    pub enabled: Option<bool>,
    pub next_trigger_at: UpdateField<DateTime<Utc>>,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub trigger_count_increment: bool,
}

#[derive(Debug, Clone, FromRow)]
pub struct LeasedResourceRow {
    pub id: LeasedResourceId,
    pub public_id: String,
    pub org_id: i64,
    pub session_id: Option<SessionId>,
    pub provider: String,
    pub resource_type: String,
    pub external_id: String,
    pub display_name: Option<String>,
    pub status: String,
    pub owner_user_id: Option<Uuid>,
    pub connection_id: Option<Uuid>,
    pub lease_duration_seconds: i32,
    pub last_touched_at: DateTime<Utc>,
    pub lease_expires_at: DateTime<Utc>,
    pub cleanup_started_at: Option<DateTime<Utc>>,
    pub cleanup_completed_at: Option<DateTime<Utc>>,
    pub cleanup_attempts: i32,
    pub last_cleanup_error: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertLeasedResourceRow {
    pub org_id: i64,
    pub session_id: SessionId,
    pub provider: String,
    pub resource_type: String,
    pub external_id: String,
    pub display_name: Option<String>,
    pub owner_user_id: Option<Uuid>,
    pub connection_id: Option<Uuid>,
    pub lease_duration_seconds: i32,
    pub lease_expires_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ReleaseLeasedResourceRow {
    pub org_id: i64,
    pub session_id: SessionId,
    pub provider: String,
    pub resource_type: String,
    pub external_id: String,
}
