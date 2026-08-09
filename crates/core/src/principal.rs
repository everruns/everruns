// Principal domain value types.
//
// Design Decision:
// - Principals are org-scoped durable owners/executors with structured metadata.
// - Ownership resolves through principal lineage to exactly one human user.
// - Execution provenance remains separate from ownership; principals only model
//   who can own durable entities and who an unattended flow can act as.
//
// EVE-837: the `Principal` aggregate entity moved to the `everruns-platform`
// crate. The value types below stay in core because they are embedded by core
// domain models: `PrincipalSummary` is a field of `Session`/`SessionSchedule`/
// `AgentIdentity`, and `PrincipalKind` backs that summary. EVE-845: the
// `PrincipalStatus` lifecycle enum, which no core type embeds, moved to
// `everruns-platform` alongside the `Principal` aggregate it describes.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::typed_id::PrincipalId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Class of principal that can hold permissions or own resources. `system`
/// is reserved for platform-internal callers and is never minted via the
/// public API.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    User,
    AgentIdentity,
    System,
}

impl std::fmt::Display for PrincipalKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::AgentIdentity => write!(f, "agent_identity"),
            Self::System => write!(f, "system"),
        }
    }
}

impl From<&str> for PrincipalKind {
    fn from(value: &str) -> Self {
        match value {
            "agent_identity" => Self::AgentIdentity,
            "system" => Self::System,
            _ => Self::User,
        }
    }
}

/// Compact view of a principal — id + kind + the subject-id pointer back
/// into the user/agent-identity row. Used wherever a full `Principal`
/// would be redundant (e.g. as a sub-field of a session or audit record).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PrincipalSummary {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "principal_01933b5a000070008000000000000001"))]
    pub id: PrincipalId,
    pub kind: PrincipalKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject_id: Option<Uuid>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub metadata: serde_json::Value,
}
