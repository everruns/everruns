use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct HealthIssueRow {
    pub id: Uuid,
    pub org_id: i64,
    /// `None` for an organization-level issue (migration 192).
    pub channel_id: Option<Uuid>,
    pub channel_public_id: Option<String>,
    pub agent_public_id: Option<String>,
    pub agent_name: Option<String>,
    pub code: String,
    pub episode_id: Uuid,
    pub status: String,
    pub missing_scopes: Vec<String>,
    pub error_code: Option<String>,
    pub channel_revision: Option<DateTime<Utc>>,
    pub first_detected_at: DateTime<Utc>,
    pub last_checked_at: DateTime<Utc>,
    /// What a background-work issue is about (`sched_…`, `agent_…`,
    /// `session_…`); `None` for channel and organization-level issues.
    #[sqlx(default)]
    pub subject_id: Option<String>,
    /// Session the issue points at, for schedule and session issues.
    #[sqlx(default)]
    pub session_id: Option<Uuid>,
    /// Numbers the issue's copy is built from.
    #[sqlx(default)]
    pub evidence: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct ObserveHealthIssue {
    pub org_id: i64,
    pub channel_id: Uuid,
    pub channel_revision: DateTime<Utc>,
    pub status: String,
    pub missing_scopes: Vec<String>,
    pub error_code: Option<String>,
    pub checked_at: DateTime<Utc>,
}
