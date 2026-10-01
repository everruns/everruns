// Virtual user domain types
//
// Design Decision:
// - VirtualUser is a first-class virtual principal, separate from Agent behavior.
// - It carries durable presentation + preference defaults and can be bound to Apps
//   and Sessions without implying every interactive turn acts as the identity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::principal::PrincipalSummary;
use crate::typed_id::VirtualUserId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Virtual user lifecycle status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum VirtualUserStatus {
    /// Available for runtime execution.
    Active,
    /// Retained but unavailable for execution.
    Archived,
    /// Soft-deleted runtime account.
    Deleted,
}

impl std::fmt::Display for VirtualUserStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VirtualUserStatus::Active => write!(f, "active"),
            VirtualUserStatus::Archived => write!(f, "archived"),
            VirtualUserStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for VirtualUserStatus {
    fn from(value: &str) -> Self {
        match value {
            "archived" => Self::Archived,
            "deleted" => Self::Deleted,
            _ => Self::Active,
        }
    }
}

/// VirtualUser is a durable virtual principal.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum VirtualUserUsage {
    /// A person using agents within an organization.
    EndUser,
    #[default]
    /// An agent service account for unattended execution.
    Service,
}

impl std::fmt::Display for VirtualUserUsage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::EndUser => "end_user",
            Self::Service => "service",
        })
    }
}

impl TryFrom<&str> for VirtualUserUsage {
    type Error = &'static str;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "end_user" => Ok(Self::EndUser),
            "service" => Ok(Self::Service),
            _ => Err("Invalid virtual user usage"),
        }
    }
}

/// Organization-scoped runtime account with its own profile and connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VirtualUser {
    /// External identifier (identity_<32-hex>). Shown as `id` in API.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "identity_01933b5a000070008000000000000001"))]
    pub id: VirtualUserId,
    /// Organization that owns the runtime account.
    #[cfg_attr(feature = "openapi", schema(example = "org_example"))]
    pub organization_id: String,
    /// Immutable runtime account purpose.
    pub usage: VirtualUserUsage,
    /// Display name used when the identity acts autonomously.
    pub name: String,
    /// Optional description shown in management UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Principal row representing this identity as a durable owner/executor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal: Option<PrincipalSummary>,
    /// Effective human owner summary derived from the principal lineage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_owner: Option<PrincipalSummary>,
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
    pub status: VirtualUserStatus,
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
