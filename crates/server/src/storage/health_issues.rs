use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct HealthIssueRow {
    pub id: Uuid,
    pub org_id: i64,
    pub channel_id: Uuid,
    pub channel_public_id: String,
    pub agent_public_id: String,
    pub agent_name: String,
    pub code: String,
    pub episode_id: Uuid,
    pub status: String,
    pub missing_scopes: Vec<String>,
    pub error_code: Option<String>,
    pub channel_revision: DateTime<Utc>,
    pub first_detected_at: DateTime<Utc>,
    pub last_checked_at: DateTime<Utc>,
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
