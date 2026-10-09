// Memory domain types
//
// Design intent lives in `knowledge/runtime-resources/memory.md`.
//
// A Memory is an org-scoped, named store that users can mount into session
// workspaces through the `memory` capability. This module defines the Memory
// entity, lifecycle status, file entries, and the capability mount config
// shape.
//
// The dual-ID pattern matches every other building-block entity: external
// `public_id: MemoryId` (mem_<32-hex>) is the API-facing identifier, internal
// UUID `internal_id` is the FK target and is never exposed in API responses.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use everruns_contracts::typed_id::{AgentId, MemoryId};

use utoipa::ToSchema;

/// Memory lifecycle status.
///
/// Resource lifecycle:
/// - `active`: assignable to mounts, editable, listed by default.
/// - `archived`: hidden from default lists, not assignable to new mounts,
///   read-only.
/// - `deleted`: tombstone; detail/list APIs return 404 except for historical
///   references (e.g. existing `session_memory_mounts` snapshots).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MemoryStatus {
    Active,
    Archived,
    Deleted,
}

impl std::fmt::Display for MemoryStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryStatus::Active => write!(f, "active"),
            MemoryStatus::Archived => write!(f, "archived"),
            MemoryStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for MemoryStatus {
    fn from(s: &str) -> Self {
        match s {
            "archived" => MemoryStatus::Archived,
            "deleted" => MemoryStatus::Deleted,
            _ => MemoryStatus::Active,
        }
    }
}

/// Ownership scope for a Memory.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    /// Organization-managed Memory selected through explicit `memory.mounts[]`.
    #[default]
    Org,
    /// Agent-owned Memory that follows the agent across sessions.
    Agent,
    /// User-owned Memory that follows the user across sessions.
    User,
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryScope::Org => write!(f, "org"),
            MemoryScope::Agent => write!(f, "agent"),
            MemoryScope::User => write!(f, "user"),
        }
    }
}

impl From<&str> for MemoryScope {
    fn from(s: &str) -> Self {
        match s {
            "agent" => MemoryScope::Agent,
            "user" => MemoryScope::User,
            _ => MemoryScope::Org,
        }
    }
}

/// A Memory — org-scoped named store.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Memory {
    /// External identifier (`mem_<32-hex>`). Shown as `id` in API responses.
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "mem_01933b5a000070008000000000000001")]
    pub public_id: MemoryId,
    /// Internal UUID primary key. Used for FK references. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Human-readable name, unique per org while not deleted.
    pub name: String,
    /// Optional human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Memory ownership scope.
    #[serde(default)]
    pub scope: MemoryScope,
    /// Owning agent for agent-scoped memories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_agent_id: Option<AgentId>,
    /// Owning user for user-scoped memories.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: Option<Uuid>,
    /// Principal that created the memory (free-form; resolved at the domain layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_principal_id: Option<String>,
    /// Resolved owner user, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_owner_user_id: Option<Uuid>,
    /// Lifecycle status.
    pub status: MemoryStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A file or directory inside a Memory.
///
/// Mirrors `SessionFile` shape; path validation is intentionally identical to
/// `session_files` so existing client code can reuse path normalization
/// helpers without bifurcating semantics.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MemoryFile {
    pub id: Uuid,
    /// Internal UUID of the parent memory.
    pub memory_id: Uuid,
    /// Absolute, normalized path starting with `/`.
    pub path: String,
    /// File content. None for directories. Encoded the same way as
    /// `SessionFile::content` (text or base64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Encoding marker: "text" or "base64". Defaults to "text".
    #[serde(default = "default_encoding")]
    pub encoding: String,
    pub is_directory: bool,
    pub size_bytes: i64,
    /// Optional `sha256:...` hash for stale-edit protection on read-write
    /// mounts. Mirrors `session_files` freshness semantics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn default_encoding() -> String {
    "text".to_string()
}

pub use everruns_capabilities::memory::{
    MemoryConfig, MemoryMountAccess, MemoryMountConfig, validate_memory_config,
    validate_mount_config_shape,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_round_trip() {
        assert_eq!(MemoryStatus::from("active").to_string(), "active");
        assert_eq!(MemoryStatus::from("archived").to_string(), "archived");
        assert_eq!(MemoryStatus::from("deleted").to_string(), "deleted");
        assert_eq!(MemoryStatus::from("unknown").to_string(), "active");
    }
}
