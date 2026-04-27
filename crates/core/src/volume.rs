// Workspace Volume domain types
//
// Design intent lives in `specs/volumes.md`.
//
// A Volume is an org-scoped, named filesystem tree that users can mount into
// session workspaces through the `workspace_volumes` capability. This module
// defines the Volume entity, lifecycle status, file entries, and the
// capability mount config shape. CRUD APIs, filesystem APIs, mount resolution,
// and UI ship as follow-up vertical slices.
//
// The dual-ID pattern matches every other building-block entity: external
// `public_id: VolumeId` (vol_<32-hex>) is the API-facing identifier, internal
// UUID `internal_id` is the FK target and is never exposed in API responses.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::typed_id::VolumeId;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Volume lifecycle status.
///
/// Mirrors the building-block lifecycle defined in `specs/models.md`:
/// - `active`: assignable to mounts, editable, listed by default.
/// - `archived`: hidden from default lists, not assignable to new mounts,
///   read-only.
/// - `deleted`: tombstone; detail/list APIs return 404 except for historical
///   references (e.g. existing `session_volume_mounts` snapshots).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum VolumeStatus {
    Active,
    Archived,
    Deleted,
}

impl std::fmt::Display for VolumeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VolumeStatus::Active => write!(f, "active"),
            VolumeStatus::Archived => write!(f, "archived"),
            VolumeStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for VolumeStatus {
    fn from(s: &str) -> Self {
        match s {
            "archived" => VolumeStatus::Archived,
            "deleted" => VolumeStatus::Deleted,
            _ => VolumeStatus::Active,
        }
    }
}

/// A workspace Volume — org-scoped named filesystem tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Volume {
    /// External identifier (`vol_<32-hex>`). Shown as `id` in API responses.
    #[serde(rename = "id")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, example = "vol_01933b5a000070008000000000000001")
    )]
    pub public_id: VolumeId,
    /// Internal UUID primary key. Used for FK references. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Human-readable name, unique per org while not deleted.
    pub name: String,
    /// Optional human-readable description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Principal that created the volume (free-form; resolved at the domain layer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_principal_id: Option<String>,
    /// Resolved owner user, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_owner_user_id: Option<Uuid>,
    /// Lifecycle status.
    pub status: VolumeStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// A file or directory inside a Volume.
///
/// Mirrors `SessionFile` shape; path validation is intentionally identical to
/// `session_files` so existing client code can reuse path normalization
/// helpers without bifurcating semantics.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VolumeFile {
    pub id: Uuid,
    /// Internal UUID of the parent volume.
    pub volume_id: Uuid,
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
pub enum VolumeMountAccess {
    #[default]
    ReadOnly,
    ReadWrite,
}

impl std::fmt::Display for VolumeMountAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VolumeMountAccess::ReadOnly => write!(f, "readonly"),
            VolumeMountAccess::ReadWrite => write!(f, "readwrite"),
        }
    }
}

impl From<&str> for VolumeMountAccess {
    fn from(s: &str) -> Self {
        match s {
            "readwrite" => VolumeMountAccess::ReadWrite,
            _ => VolumeMountAccess::ReadOnly,
        }
    }
}

/// Capability config entry for `workspace_volumes`. One entry per mount.
///
/// Wire shape:
///
/// ```json
/// { "volume": "vol_abc...", "path": "/workspace/research", "mode": "readonly" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VolumeMountConfig {
    /// Public Volume ID (`vol_<32-hex>`).
    pub volume: String,
    /// Mount path under `/workspace`.
    pub path: String,
    /// Access mode. Defaults to `readonly` when omitted.
    #[serde(default)]
    pub mode: VolumeMountAccess,
}

/// Top-level config for the `workspace_volumes` capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct WorkspaceVolumesConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<VolumeMountConfig>,
}

/// Validation outcome for a single mount config entry.
///
/// Domain-level cross-validation (cross-org references, archived/deleted
/// volumes, capability-mount overlaps) happens at the server layer. This
/// helper covers the structural checks we can perform without DB access so
/// that capability `validate_config` and any clientside form validation
/// share semantics.
pub fn validate_mount_config_shape(mount: &VolumeMountConfig) -> Result<(), String> {
    // Volume reference must be a syntactically valid VolumeId.
    if !mount.volume.starts_with("vol_") {
        return Err(format!(
            "mount.volume must be a vol_-prefixed Volume ID, got '{}'",
            mount.volume
        ));
    }

    // Mount path must be under /workspace, must not contain `..` or null bytes,
    // must not have empty segments, must not have a trailing slash.
    let path = &mount.path;
    if !path.starts_with("/workspace") {
        return Err(format!(
            "mount.path must start with /workspace, got '{path}'"
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

/// Validate a full workspace_volumes config: per-entry shape + duplicate /
/// overlapping path detection.
pub fn validate_workspace_volumes_config(config: &WorkspaceVolumesConfig) -> Result<(), String> {
    for mount in &config.mounts {
        validate_mount_config_shape(mount)?;
    }
    // Reject duplicate mount paths.
    let mut seen: Vec<&str> = Vec::with_capacity(config.mounts.len());
    for mount in &config.mounts {
        if seen.iter().any(|p| *p == mount.path) {
            return Err(format!(
                "duplicate mount path '{}' in workspace_volumes config",
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
        assert_eq!(VolumeStatus::from("active").to_string(), "active");
        assert_eq!(VolumeStatus::from("archived").to_string(), "archived");
        assert_eq!(VolumeStatus::from("deleted").to_string(), "deleted");
        assert_eq!(VolumeStatus::from("unknown").to_string(), "active");
    }

    #[test]
    fn access_default_is_readonly() {
        let cfg: VolumeMountConfig = serde_json::from_str(
            r#"{ "volume": "vol_00000000000000000000000000000001", "path": "/workspace/r" }"#,
        )
        .unwrap();
        assert_eq!(cfg.mode, VolumeMountAccess::ReadOnly);
    }

    #[test]
    fn validate_rejects_non_vol_prefix() {
        let cfg = VolumeMountConfig {
            volume: "agent_x".into(),
            path: "/workspace/r".into(),
            mode: VolumeMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_path_outside_workspace() {
        let cfg = VolumeMountConfig {
            volume: "vol_00000000000000000000000000000001".into(),
            path: "/etc/passwd".into(),
            mode: VolumeMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_dotdot() {
        let cfg = VolumeMountConfig {
            volume: "vol_00000000000000000000000000000001".into(),
            path: "/workspace/../etc".into(),
            mode: VolumeMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_double_slash() {
        let cfg = VolumeMountConfig {
            volume: "vol_00000000000000000000000000000001".into(),
            path: "/workspace//data".into(),
            mode: VolumeMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_rejects_trailing_slash() {
        let cfg = VolumeMountConfig {
            volume: "vol_00000000000000000000000000000001".into(),
            path: "/workspace/data/".into(),
            mode: VolumeMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_err());
    }

    #[test]
    fn validate_accepts_valid_mount() {
        let cfg = VolumeMountConfig {
            volume: "vol_00000000000000000000000000000001".into(),
            path: "/workspace/research".into(),
            mode: VolumeMountAccess::ReadOnly,
        };
        assert!(validate_mount_config_shape(&cfg).is_ok());
    }

    #[test]
    fn config_validate_rejects_duplicate_paths() {
        let cfg = WorkspaceVolumesConfig {
            mounts: vec![
                VolumeMountConfig {
                    volume: "vol_00000000000000000000000000000001".into(),
                    path: "/workspace/data".into(),
                    mode: VolumeMountAccess::ReadOnly,
                },
                VolumeMountConfig {
                    volume: "vol_00000000000000000000000000000002".into(),
                    path: "/workspace/data".into(),
                    mode: VolumeMountAccess::ReadWrite,
                },
            ],
        };
        let err = validate_workspace_volumes_config(&cfg).unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn config_validate_rejects_overlapping_paths() {
        let cfg = WorkspaceVolumesConfig {
            mounts: vec![
                VolumeMountConfig {
                    volume: "vol_00000000000000000000000000000001".into(),
                    path: "/workspace/data".into(),
                    mode: VolumeMountAccess::ReadOnly,
                },
                VolumeMountConfig {
                    volume: "vol_00000000000000000000000000000002".into(),
                    path: "/workspace/data/sub".into(),
                    mode: VolumeMountAccess::ReadWrite,
                },
            ],
        };
        let err = validate_workspace_volumes_config(&cfg).unwrap_err();
        assert!(err.contains("overlapping"));
    }

    #[test]
    fn config_validate_accepts_distinct_paths() {
        let cfg = WorkspaceVolumesConfig {
            mounts: vec![
                VolumeMountConfig {
                    volume: "vol_00000000000000000000000000000001".into(),
                    path: "/workspace/data".into(),
                    mode: VolumeMountAccess::ReadOnly,
                },
                VolumeMountConfig {
                    volume: "vol_00000000000000000000000000000002".into(),
                    path: "/workspace/notes".into(),
                    mode: VolumeMountAccess::ReadWrite,
                },
            ],
        };
        assert!(validate_workspace_volumes_config(&cfg).is_ok());
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
