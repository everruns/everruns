// Principal entity (platform domain aggregate).
//
// Moved out of `everruns-core` in EVE-837. The principal value types embedded by
// core domain models remain in `everruns-core`: `PrincipalSummary` is a field of
// `Session`/`App`/`SessionSchedule`, `PrincipalKind` backs that summary, and
// `PrincipalStatus` is shared with the agent-identity lifecycle. This aggregate
// depends on them (direction: platform -> core) and re-exports them from the
// crate root.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use everruns_core::typed_id::PrincipalId;
use everruns_core::{PrincipalKind, PrincipalStatus, PrincipalSummary};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

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
