// Principal entity (platform domain aggregate).
//
// Moved out of `everruns-core` in EVE-837. The compact embedded value types
// remain in `everruns-core`: `PrincipalSummary` is a field of `Session`/
// `SessionSchedule`/`AgentIdentity`, and `PrincipalKind` backs that summary.
// EVE-845 moved the `PrincipalStatus` lifecycle enum here — no core type embeds
// it; it describes this durable aggregate. This crate depends on the core value
// types (direction: platform -> core) and re-exports the full principal surface
// from the crate root.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use everruns_core::{PrincipalKind, PrincipalSummary};
use everruns_provider::typed_id::PrincipalId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Principal lifecycle status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum PrincipalStatus {
    Active,
    Archived,
    Deleted,
}

impl std::fmt::Display for PrincipalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Archived => write!(f, "archived"),
            Self::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for PrincipalStatus {
    fn from(value: &str) -> Self {
        match value {
            "archived" => Self::Archived,
            "deleted" => Self::Deleted,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Principal {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "principal_01933b5a000070008000000000000001"))]
    pub id: PrincipalId,
    pub organization_id: String,
    pub kind: PrincipalKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_principal_id: Option<PrincipalId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_user_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
    pub status: PrincipalStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

impl Principal {
    pub fn summary(&self) -> PrincipalSummary {
        PrincipalSummary {
            id: self.id,
            kind: self.kind.clone(),
            subject_id: self.subject_id,
            metadata: self.metadata.clone(),
        }
    }
}
