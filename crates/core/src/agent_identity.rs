// Agent identity domain types
//
// Design Decision:
// - AgentIdentity is a first-class virtual principal, separate from Agent behavior.
// - It carries durable presentation + preference defaults and can be bound to Apps
//   and Sessions without implying every interactive turn acts as the identity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::AgentIdentityId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Agent identity lifecycle status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AgentIdentityStatus {
    Active,
    Archived,
    Deleted,
}

impl std::fmt::Display for AgentIdentityStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AgentIdentityStatus::Active => write!(f, "active"),
            AgentIdentityStatus::Archived => write!(f, "archived"),
            AgentIdentityStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for AgentIdentityStatus {
    fn from(value: &str) -> Self {
        match value {
            "archived" => Self::Archived,
            "deleted" => Self::Deleted,
            _ => Self::Active,
        }
    }
}

/// AgentIdentity is a durable virtual principal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AgentIdentity {
    /// External identifier (identity_<32-hex>). Shown as `id` in API.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "identity_01933b5a000070008000000000000001"))]
    pub id: AgentIdentityId,
    /// Display name used when the identity acts autonomously.
    pub name: String,
    /// Optional description shown in management UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional avatar URL for UI surfaces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// Default locale for unattended runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Default timezone for unattended runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// Lifecycle status.
    pub status: AgentIdentityStatus,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
    /// Archive timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Delete timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}
