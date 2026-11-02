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
    /// Current user's announcement identifier, when one was issued.
    #[schema(example = "notification_550e8400e29b41d4a716446655440001")]
    pub notification_id: Option<String>,
    /// Current user's reminder suppression deadline, if snoozed.
    #[schema(example = "2026-10-04T12:00:00Z")]
    pub snoozed_until: Option<DateTime<Utc>>,
    /// Application-relative link to the issue details.
    #[schema(example = "/settings/health?issue=550e8400-e29b-41d4-a716-446655440000")]
    pub href: String,
    /// What a background-work issue is about: a schedule (`sched_…`), an agent
    /// (`agent_…`), a session (`session_…`) or the organization (`org_…`).
    #[schema(example = "sched_550e8400e29b41d4a716446655440000")]
    pub subject_id: Option<String>,
    /// Public identifier of the session the issue points at, when it has one.
    #[schema(example = "session_550e8400e29b41d4a716446655440000")]
    pub session_id: Option<String>,
    /// The numbers a detector or classifier recorded for a finding: counts,
    /// costs, probabilities. Never session content.
    #[schema(value_type = Option<Object>, example = json!({"probability": 0.97, "calibrated": true}))]
    pub evidence: Option<serde_json::Value>,
    /// The Inspector's write-up of a finding's current episode, when it has
    /// one.
    pub investigation: Option<FindingInvestigation>,
}

/// The Inspector's write-up of a finding: what happened, how serious it is,
/// what to do, and the cluster of similar findings it was filed under.
#[derive(Debug, Serialize, ToSchema)]
pub struct FindingInvestigation {
    /// `running`, `done`, `failed`, or `unavailable` (no model to write it).
    #[schema(example = "done")]
    pub status: String,
    /// What happened, in two to four plain sentences.
    #[schema(
        example = "The schedule polls a sign-in claim that expired; every run since reports the same error."
    )]
    pub what_happened: Option<String>,
    /// `high`, `medium` or `low`, as the Inspector judged it.
    #[schema(example = "high")]
    pub severity: Option<String>,
    /// One concrete next step.
    #[schema(example = "Add a stop rule to the schedule's instructions.")]
    pub suggested_action: Option<String>,
    /// The cluster of similar findings in this organization.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub cluster_id: Option<Uuid>,
    /// Title of that cluster, naming the problem the findings share.
    #[schema(example = "Schedules polling an expired claim")]
    pub cluster_title: Option<String>,
    /// Open findings in the cluster, this one included.
    #[schema(example = 3)]
    pub cluster_open: Option<i64>,
    /// The model that wrote it.
    #[schema(example = "gpt-6.1-sol")]
    pub model: Option<String>,
    /// When the write-up was finished.
    #[schema(example = "2026-10-18T21:40:00Z")]
    pub completed_at: Option<DateTime<Utc>>,
    /// When a person dismissed the finding, and why.
    pub dismissed_at: Option<DateTime<Utc>>,
    /// The note the person gave when dismissing.
    #[schema(example = "Expected: load test")]
    pub dismiss_reason: Option<String>,
}

/// An open finding of the platform operator's concern, in any organization.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlatformDetection {
    /// Health issue identifier of the finding.
    #[schema(example = "550e8400-e29b-41d4-a716-446655440000")]
    pub issue_id: Uuid,
    /// Public identifier of the organization.
    #[schema(example = "org_550e8400e29b41d4a716446655440000")]
    pub org_id: String,
    /// Display name of the organization.
    #[schema(example = "Acme")]
    pub org_name: String,
    /// Detector code of the finding.
    #[schema(example = "session.suspected_abuse")]
    pub code: String,
    /// Public identifier of the session the finding points at.
    #[schema(example = "session_550e8400e29b41d4a716446655440000")]
    pub session_id: Option<String>,
    /// Display name of the session's agent.
    #[schema(example = "Outreach bot")]
    pub agent_name: Option<String>,
    /// The numbers the classifier recorded, never session content.
    #[schema(value_type = Option<Object>, example = json!({"probability": 0.93}))]
    pub evidence: Option<serde_json::Value>,
    /// When the current episode was first detected.
    #[schema(example = "2026-10-18T21:40:00Z")]
    pub first_detected_at: DateTime<Utc>,
    /// The Inspector's account of what happened, once written.
    pub what_happened: Option<String>,
    /// `high`, `medium` or `low`, as the Inspector judged it.
    #[schema(example = "high")]
    pub severity: Option<String>,
    /// The Inspector's suggested next step.
    pub suggested_action: Option<String>,
    /// Title of the cluster of similar findings in that organization.
    pub cluster_title: Option<String>,
}

/// A page of open platform detections.
#[derive(Debug, Serialize, ToSchema)]
pub struct PlatformDetectionList {
    /// Detections in this page.
    pub data: Vec<PlatformDetection>,
    /// Number of open detections.
    #[schema(example = 1)]
    pub total: i64,
    /// Effective pagination offset.
    #[schema(example = 0)]
    pub offset: i64,
    /// Effective page size.
    #[schema(example = 20)]
    pub limit: i64,
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
