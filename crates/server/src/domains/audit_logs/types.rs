// Audit log domain types.

use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

pub use crate::storage::models::{AuditLogQuery, AuditLogRow};

/// Domain-level audit log view. Mirrors `AuditLogRow` but omits `org_id`
/// (derived from the caller) and formats IDs as strings, matching the
/// shape returned by the HTTP and MCP adapters.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AuditLogEntry {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
    pub domain: String,
    pub action: String,
    pub actor_id: Option<String>,
    pub event_type: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
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
