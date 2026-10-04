use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthIssue {
    pub id: Uuid,
    pub code: String,
    pub agent_id: String,
    pub agent_name: String,
    pub channel_id: String,
    pub status: String,
    pub title: String,
    pub body: String,
    pub missing_scopes: Vec<String>,
    pub error_code: Option<String>,
    pub first_detected_at: DateTime<Utc>,
    pub last_checked_at: DateTime<Utc>,
    pub stale: bool,
    pub notification_id: Option<String>,
    pub snoozed_until: Option<DateTime<Utc>>,
    pub href: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthIssueList {
    pub data: Vec<HealthIssue>,
    pub total: i64,
    pub offset: i64,
    pub limit: i64,
}
