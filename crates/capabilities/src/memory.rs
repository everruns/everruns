//! Runtime mount configuration for the hosted memory capability.
use everruns_contracts::typed_id::MemoryId;
use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

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
