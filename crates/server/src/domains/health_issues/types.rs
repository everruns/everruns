use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

/// An organization-scoped operational issue and its current recovery evidence.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthIssue {
    /// Stable identifier of the canonical issue.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub id: Uuid,
    /// Detector code: `slack.permissions` for a channel, `org.active_turn_limit` for the organization.
    #[schema(example = "slack.permissions")]
    pub code: String,
    /// Public identifier of the owning agent; absent for an organization-level issue.
    #[schema(example = "agent_550e8400e29b41d4a716446655440000")]
    pub agent_id: Option<String>,
    /// Display name of the owning agent; absent for an organization-level issue.
    #[schema(example = "Support assistant")]
    pub agent_name: Option<String>,
    /// Public identifier of the affected channel; absent for an organization-level issue.
    #[schema(example = "appchan_550e8400e29b41d4a716446655440000")]
    pub channel_id: Option<String>,
    /// Current issue state: open, needs_check, resolved, or inapplicable.
    #[schema(example = "open")]
    pub status: String,
    /// Human-readable summary of the required action.
    #[schema(example = "Slack permissions need updating")]
    pub title: String,
    /// Explanation of the impact and recovery action.
    #[schema(example = "Reconnect Slack to grant the required permissions.")]
    pub body: String,
    /// Required Slack scopes absent from the verified grant.
    #[schema(example = json!(["reactions:write"]))]
    pub missing_scopes: Vec<String>,
    /// Sanitized provider or verification error code, when available.
    #[schema(example = "missing_scope")]
    pub error_code: Option<String>,
    /// Time the current issue episode was first detected.
    #[schema(example = "2026-10-03T12:00:00Z")]
    pub first_detected_at: DateTime<Utc>,
    /// Time the latest accepted verification evidence was observed.
    #[schema(example = "2026-10-03T12:05:00Z")]
    pub last_checked_at: DateTime<Utc>,
    /// Whether the evidence is old, unavailable, or for a previous channel revision.
    #[schema(example = false)]
    pub stale: bool,
    /// Current user's announcement identifier, when notifications are enabled.
    #[schema(example = "notification_550e8400e29b41d4a716446655440001")]
    pub notification_id: Option<String>,
    /// Current user's reminder suppression deadline, if snoozed.
    #[schema(example = "2026-10-04T12:00:00Z")]
    pub snoozed_until: Option<DateTime<Utc>>,
    /// Application-relative link to the issue details.
    #[schema(example = "/settings/health?issue=550e8400-e29b-41d4-a716-446655440000")]
    pub href: String,
}

/// A page of pending operational health issues visible to the current caller.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthIssueList {
    /// Issues in this page.
    #[schema(example = json!([]))]
    pub data: Vec<HealthIssue>,
    /// Number of issues matching the current filter.
    #[schema(example = 1)]
    pub total: i64,
    /// Effective pagination offset.
    #[schema(example = 0)]
    pub offset: i64,
    /// Effective page size.
    #[schema(example = 20)]
    pub limit: i64,
}
