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

use everruns_provider::typed_id::{AgentId, MemoryId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Memory lifecycle status.
///
/// Mirrors the building-block lifecycle defined in `knowledge/foundations/models.md`:
/// - `active`: assignable to mounts, editable, listed by default.
/// - `archived`: hidden from default lists, not assignable to new mounts,
///   read-only.
/// - `deleted`: tombstone; detail/list APIs return 404 except for historical
///   references (e.g. existing `session_memory_mounts` snapshots).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Memory {
    /// External identifier (`mem_<32-hex>`). Shown as `id` in API responses.
    #[serde(rename = "id")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, example = "mem_01933b5a000070008000000000000001")
    )]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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

/// Mount access mode. Defaults to `ReadOnly` when omitted from config.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum MemoryMountAccess {
    #[default]
    ReadOnly,
    ReadWrite,
}

impl std::fmt::Display for MemoryMountAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryMountAccess::ReadOnly => write!(f, "readonly"),
            MemoryMountAccess::ReadWrite => write!(f, "readwrite"),
        }
    }
}

impl From<&str> for MemoryMountAccess {
    fn from(s: &str) -> Self {
        match s {
            "readwrite" => MemoryMountAccess::ReadWrite,
            _ => MemoryMountAccess::ReadOnly,
        }
    }
}

/// Capability config entry for `memory`. One entry per mount.
///
/// Wire shape:
///
/// ```json
/// { "memory": "mem_abc...", "path": "/workspace/research", "mode": "readonly" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MemoryMountConfig {
    /// Public Memory ID (`mem_<32-hex>`).
    pub memory: String,
    /// Mount path under `/workspace`.
    pub path: String,
    /// Access mode. Defaults to `readonly` when omitted.
    #[serde(default)]
    pub mode: MemoryMountAccess,
}

/// Top-level config for the `memory` capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MemoryConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<MemoryMountConfig>,
}

/// Validation outcome for a single mount config entry.
///
/// Domain-level cross-validation (cross-org references, archived/deleted
/// memories, capability-mount overlaps) happens at the server layer. This
/// helper covers the structural checks we can perform without DB access so
/// that capability `validate_config` and any clientside form validation
/// share semantics.
pub fn validate_mount_config_shape(mount: &MemoryMountConfig) -> Result<(), String> {
    // Memory reference must be a syntactically valid MemoryId
    // (mem_<32-lowercase-hex>) — match the DB CHECK constraint exactly so
    // structurally invalid IDs cannot pass capability validation and reach
    // domain code or the database.
    if MemoryId::parse(&mount.memory).is_err() {
        return Err(format!(
            "mount.memory must be a valid Memory ID of the form mem_<32-lowercase-hex>, got '{}'",
            mount.memory
        ));
    }

    // Mount path must be either exactly `/workspace` or descend from it via a
    // `/workspace/` boundary (rejects lookalikes such as `/workspacefoo`).
    // It must not contain `..`, null bytes, empty segments, or a trailing slash.
    let path = &mount.path;
    if path != "/workspace" && !path.starts_with("/workspace/") {
        return Err(format!(
            "mount.path must be /workspace or start with /workspace/, got '{path}'"
        ));
    }
    if path.contains("//") {
        return Err(format!("mount.path must not contain '//', got '{path}'"));
    }
    if path.contains('\0') {
        return Err(format!(
            "mount.path must not contain null bytes, got '{path}'"
        ));
    }
    if path.split('/').any(|seg| seg == "..") {
        return Err(format!("mount.path must not contain '..', got '{path}'"));
    }
    if path.len() > 1 && path.ends_with('/') {
        return Err(format!(
            "mount.path must not end with a trailing slash, got '{path}'"
        ));
    }
    Ok(())
}

/// Validate a full `memory` capability config: per-entry shape + duplicate /
/// overlapping path detection.
pub fn validate_memory_config(config: &MemoryConfig) -> Result<(), String> {
    for mount in &config.mounts {
        validate_mount_config_shape(mount)?;
    }
    // Reject duplicate mount paths.
    let mut seen: Vec<&str> = Vec::with_capacity(config.mounts.len());
    for mount in &config.mounts {
        if seen.iter().any(|p| *p == mount.path) {
            return Err(format!(
                "duplicate mount path '{}' in memory config",
                mount.path
            ));
        }
        seen.push(&mount.path);
    }
    // Reject overlapping mount paths (one being a prefix of another).
    for (i, a) in config.mounts.iter().enumerate() {
        for b in &config.mounts[i + 1..] {
            if mount_paths_overlap(&a.path, &b.path) {
                return Err(format!(
                    "overlapping mount paths '{}' and '{}'",
                    a.path, b.path
                ));
            }
        }
    }
    Ok(())
}

fn mount_paths_overlap(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (shorter, longer) = if a.len() < b.len() { (a, b) } else { (b, a) };
    longer.starts_with(shorter) && longer.as_bytes().get(shorter.len()) == Some(&b'/')
}

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

    #[test]
    fn access_default_is_readonly() {
        let cfg: MemoryMountConfig = serde_json::from_str(
            r#"{ "memory": "mem_00000000000000000000000000000001", "path": "/workspace/r" }"#,
        )
        .unwrap();
        assert_eq!(cfg.mode, MemoryMountAccess::ReadOnly);
    }

    #[test]
    fn validate_rejects_non_mem_prefix() {
        let cfg = MemoryMountConfig {
            memory: "agent_x".into(),
            path: "/workspace/r".into(),
            mode: MemoryMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_path_outside_workspace() {
        let cfg = MemoryMountConfig {
            memory: "mem_00000000000000000000000000000001".into(),
            path: "/etc/passwd".into(),
            mode: MemoryMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_workspace_prefix_lookalike() {
        // /workspacefoo must NOT pass the /workspace boundary check.
        let cfg = MemoryMountConfig {
            memory: "mem_00000000000000000000000000000001".into(),
            path: "/workspacefoo".into(),
            mode: MemoryMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_accepts_workspace_root() {
        let cfg = MemoryMountConfig {
            memory: "mem_00000000000000000000000000000001".into(),
            path: "/workspace".into(),
            mode: MemoryMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_invalid_hex_in_memory_id() {
        // mem_-prefixed but not 32 lowercase hex chars must be rejected so
        // structurally invalid IDs cannot reach the database.
        let cfg = MemoryMountConfig {
            memory: "mem_not-hex".into(),
            path: "/workspace/r".into(),
            mode: MemoryMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_dotdot() {
        let cfg = MemoryMountConfig {
            memory: "mem_00000000000000000000000000000001".into(),
            path: "/workspace/../etc".into(),
            mode: MemoryMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_double_slash() {
        let cfg = MemoryMountConfig {
            memory: "mem_00000000000000000000000000000001".into(),
            path: "/workspace//data".into(),
            mode: MemoryMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_trailing_slash() {
        let cfg = MemoryMountConfig {
            memory: "mem_00000000000000000000000000000001".into(),
            path: "/workspace/data/".into(),
            mode: MemoryMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_accepts_valid_mount() {
        let cfg = MemoryMountConfig {
            memory: "mem_00000000000000000000000000000001".into(),
            path: "/workspace/research".into(),
            mode: MemoryMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_ok());
    }

    #[test]
    fn config_validate_rejects_duplicate_paths() {
        let cfg = MemoryConfig {
            mounts: vec![
                MemoryMountConfig {
                    memory: "mem_00000000000000000000000000000001".into(),
                    path: "/workspace/data".into(),
                    mode: MemoryMountAccess::ReadOnly,
                },
                MemoryMountConfig {
                    memory: "mem_00000000000000000000000000000002".into(),
                    path: "/workspace/data".into(),
                    mode: MemoryMountAccess::ReadWrite,
                },
            ],
        };
        let err = validate_memory_config(&cfg).unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn config_validate_rejects_overlapping_paths() {
        let cfg = MemoryConfig {
            mounts: vec![
                MemoryMountConfig {
                    memory: "mem_00000000000000000000000000000001".into(),
                    path: "/workspace/data".into(),
                    mode: MemoryMountAccess::ReadOnly,
                },
                MemoryMountConfig {
                    memory: "mem_00000000000000000000000000000002".into(),
                    path: "/workspace/data/sub".into(),
                    mode: MemoryMountAccess::ReadWrite,
                },
            ],
        };
        let err = validate_memory_config(&cfg).unwrap_err();
        assert!(err.contains("overlapping"));
    }

    #[test]
    fn config_validate_accepts_distinct_paths() {
        let cfg = MemoryConfig {
            mounts: vec![
                MemoryMountConfig {
                    memory: "mem_00000000000000000000000000000001".into(),
                    path: "/workspace/data".into(),
                    mode: MemoryMountAccess::ReadOnly,
                },
                MemoryMountConfig {
                    memory: "mem_00000000000000000000000000000002".into(),
                    path: "/workspace/notes".into(),
                    mode: MemoryMountAccess::ReadWrite,
                },
            ],
        };
        assert!(validate_memory_config(&cfg).is_ok());
    }

    #[test]
    fn overlap_helper_does_not_match_unrelated_prefix() {
        // /workspace/data and /workspace/datasets must NOT overlap.
        assert!(!mount_paths_overlap(
            "/workspace/data",
            "/workspace/datasets"
        ));
    }
}
