// Workspace persistence record (EVE-880).
//
// The stored, org-scoped working area. Execution never loads this row: a turn
// consumes only `WorkspaceId` plus the resolved roots/policy that stay in
// `everruns-core`, so the record and its lifecycle live with the other
// control-plane aggregates.
//
// Design intent lives in `knowledge/runtime-resources/workspace.md`.
//
// A Workspace is an org-scoped, named working area that holds the files an
// agent reads and writes during execution. Sessions attach to a Workspace;
// multiple sessions may share one Workspace, or each session may own its
// own (the default, per session creation).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use everruns_provider::typed_id::WorkspaceId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Workspace lifecycle status.
///
/// Mirrors the standard building-block lifecycle from `knowledge/foundations/models.md`:
/// - `active`: assignable to sessions, editable, listed by default.
/// - `archived`: hidden from default lists, not assignable to new sessions,
///   files become read-only.
/// - `deleted`: tombstone; detail/list APIs return 404 except for historical
///   references (existing session links).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceStatus {
    Active,
    Archived,
    Deleted,
}

impl std::fmt::Display for WorkspaceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceStatus::Active => write!(f, "active"),
            WorkspaceStatus::Archived => write!(f, "archived"),
            WorkspaceStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for WorkspaceStatus {
    fn from(s: &str) -> Self {
        match s {
            "archived" => WorkspaceStatus::Archived,
            "deleted" => WorkspaceStatus::Deleted,
            _ => WorkspaceStatus::Active,
        }
    }
}

/// A Workspace — org-scoped named working area.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Workspace {
    /// External identifier (`wsp_<32-hex>`). Shown as `id` in API responses.
    #[serde(rename = "id")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, example = "wsp_01933b5a000070008000000000000001")
    )]
    pub public_id: WorkspaceId,
    /// Internal UUID primary key. Used for FK references. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Human-readable name, unique per org while not deleted.
    pub name: String,
    /// Optional human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Principal that created the workspace (free-form; resolved at the domain layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_principal_id: Option<String>,
    /// Resolved owner user, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_owner_user_id: Option<Uuid>,
    /// Lifecycle status.
    pub status: WorkspaceStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}
