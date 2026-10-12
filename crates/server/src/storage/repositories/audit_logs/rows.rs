// Rows the audit logs repository reads and writes.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Audit log row from database
#[derive(Debug, Clone, FromRow)]
pub struct AuditLogRow {
    pub id: Uuid,
    pub org_id: i64,
    pub actor_id: Option<Uuid>,
    pub event_type: String,
    pub ip_address: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    /// Audit domain: "management" or "agent"
    #[sqlx(default)]
    pub domain: String,
    /// Structured action (e.g. "management.member.invited")
    #[sqlx(default)]
    pub action: String,
    /// Target resource type (e.g. "harness", "agent", "member")
    #[sqlx(default)]
    pub target_type: Option<String>,
    /// Target resource ID (public ID or UUID)
    #[sqlx(default)]
    pub target_id: Option<String>,
}

/// Input for creating an audit log entry
#[derive(Debug, Clone)]
pub struct CreateAuditLogRow {
    pub org_id: i64,
    pub actor_id: Option<Uuid>,
    pub event_type: String,
    pub ip_address: Option<String>,
    pub metadata: serde_json::Value,
    pub domain: String,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
}

/// Query parameters for listing audit logs
#[derive(Debug, Clone, Default)]
pub struct AuditLogQuery<'a> {
    pub org_id: i64,
    pub limit: i64,
    pub before: Option<DateTime<Utc>>,
    pub event_type_prefix: Option<&'a str>,
    pub actor_id: Option<Uuid>,
    pub domain: Option<&'a str>,
    pub action: Option<&'a str>,
}
