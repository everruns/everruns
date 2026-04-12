// Session resource registry — generic, session-scoped registry of active resources.
//
// Decision: SessionResourceRegistry is the core trait. Any capability registers
// resources here (sandboxes, subagents, browser sessions, future background work).
// Agents query it ("what's running?"), infra scans it for cleanup.
// Decision: resource_id is caller-provided and unique per session. Repeated
// register calls with the same resource_id update rather than duplicate.
// Decision: kind is a free-form string for extensibility — no enum.
// Decision: LeasedResourceStore auto-registers into the registry so existing
// tool code doesn't change.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::SessionId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Status of a resource in the session resource registry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionResourceStatus {
    /// Resource is alive and doing work.
    Active,
    /// Work finished successfully.
    Completed,
    /// Work finished with an error.
    Failed,
    /// Resource was explicitly released / cleaned up.
    Released,
}

impl std::fmt::Display for SessionResourceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Released => write!(f, "released"),
        }
    }
}

impl From<&str> for SessionResourceStatus {
    fn from(s: &str) -> Self {
        match s {
            "active" => Self::Active,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "released" => Self::Released,
            _ => Self::Active,
        }
    }
}

/// A resource registered in the session resource registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SessionResourceEntry {
    /// Caller-provided stable ID (unique per session).
    pub resource_id: String,
    /// Parent session.
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub session_id: SessionId,
    /// Resource kind: "sandbox", "subagent", "browser_session", etc.
    pub kind: String,
    /// Human-readable label.
    pub display_name: String,
    /// Lifecycle status.
    pub status: SessionResourceStatus,
    /// Kind-specific non-secret metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for registering a resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterSessionResource {
    pub session_id: SessionId,
    /// Caller-provided stable ID (leased resource ID, session ID, etc.).
    pub resource_id: String,
    /// Resource kind: "sandbox", "subagent", "browser_session", etc.
    pub kind: String,
    /// Human-readable label.
    pub display_name: String,
    /// Initial status (defaults to Active).
    #[serde(default = "default_active")]
    pub status: SessionResourceStatus,
    /// Kind-specific non-secret metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

fn default_active() -> SessionResourceStatus {
    SessionResourceStatus::Active
}

/// Optional filter for listing resources.
#[derive(Debug, Clone, Default)]
pub struct SessionResourceFilter {
    /// Filter by kind (exact match).
    pub kind: Option<String>,
    /// Filter by status.
    pub status: Option<SessionResourceStatus>,
}
