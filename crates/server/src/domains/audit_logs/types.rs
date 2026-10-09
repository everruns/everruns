// Audit log domain types.

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

pub use crate::storage::{AuditLogQuery, AuditLogRow};

/// Domain-level audit log view. Mirrors `AuditLogRow` but omits `org_id`
/// (derived from the caller) and formats IDs as strings, matching the
/// shape returned by the HTTP and MCP adapters.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditLogEntry {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    /// Audit domain: `management` for org administration, `agent` for agent activity.
    #[schema(example = "management")]
    pub domain: String,
    /// What happened, as `<domain>.<resource>.<verb>`.
    #[schema(example = "management.member.invited")]
    pub action: String,
    /// UUID of the user who acted, when a user did.
    #[schema(example = "01933b5a-0000-7000-8000-000000000001")]
    pub actor_id: Option<String>,
    /// Legacy event type, kept for older filters.
    #[schema(example = "auth.login")]
    pub event_type: String,
    /// Kind of resource the entry is about.
    #[schema(example = "member")]
    pub target_type: Option<String>,
    /// Identifier of the resource the entry is about.
    pub target_id: Option<String>,
    /// Client IP address of the request, when known.
    pub ip_address: Option<String>,
    /// Free-form metadata attached to this resource.
    pub metadata: serde_json::Value,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
}

impl From<AuditLogRow> for AuditLogEntry {
    fn from(r: AuditLogRow) -> Self {
        Self {
            id: r.id.to_string(),
            domain: r.domain,
            action: r.action,
            actor_id: r.actor_id.map(stringify_uuid),
            event_type: r.event_type,
            target_type: r.target_type,
            target_id: r.target_id,
            ip_address: r.ip_address,
            metadata: r.metadata,
            created_at: r.created_at,
        }
    }
}

fn stringify_uuid(u: Uuid) -> String {
    u.to_string()
}
