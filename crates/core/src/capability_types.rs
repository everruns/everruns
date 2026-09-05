// Capability type definitions
//
// Design Decision (EVE-873): capability identity and per-agent configuration
// are the neutral `everruns-capability` contract — `CapabilityId`,
// `CapabilityRef`, validation, and registry index bookkeeping live there and
// are imported here for the execution-facing vocabulary. Core owns only the
// runtime-facing vocabulary that needs engine types (capability status,
// mount points, virtual file trees).
//
// Design Decision: Capability IDs are String-based to allow adding new capabilities
// without requiring database migrations or code changes. Each capability defines its
// own ID via the Capability trait's id() method - no central registry of IDs needed.
// Validation happens at the registry level (capability must be registered).
//
// Design Decision: Capabilities can declare mount points to populate files in the session
// filesystem. Mount points specify a path, access mode (readonly/readwrite), and content
// source. This allows capabilities to provide sample data, documentation, or configuration
// files that agents can access during execution.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
use everruns_capability::CapabilityId;

/// Per-agent capability configuration — the persisted attachment row shape.
///
/// This is the neutral [`CapabilityRef`](everruns_capability::CapabilityRef)
/// under its historical product name:
/// one semantic model for "capability id + per-agent JSON config" shared by
/// the Framework, persisted attachments, and worker resolution. It serializes
/// as `{"ref": "<id>", "config": {…}}`.
pub(crate) use everruns_capability::CapabilityRef as AgentCapabilityConfig;

// OpenAPI schema surrogate for `AgentCapabilityConfig`.
//
// The neutral contract crate carries no OpenAPI dependency, so this doc-only
// shadow struct reproduces the wire shape (`ref` + optional `config`) for
// `utoipa` derives. Fields embedding `AgentCapabilityConfig` reference it via
// `#[schema(value_type = AgentCapabilityConfigSchema)]`; the emitted component
// keeps the public name `AgentCapabilityConfig`. The doc comment below is the
// published schema description — keep it byte-stable.
#[cfg(feature = "openapi")]
#[derive(serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[schema(as = AgentCapabilityConfig)]
#[allow(dead_code)]
/// Per-agent capability configuration
///
/// Associates a capability with an agent, including optional per-agent configuration.
/// The config field allows the same capability to behave differently per-agent.
pub(crate) struct AgentCapabilityConfigSchema {
    /// Reference to the capability ID
    #[serde(rename = "ref")]
    #[schema(value_type = String)]
    pub capability_ref: String,
    /// Per-agent configuration for this capability (capability-specific)
    #[serde(default)]
    pub config: serde_json::Value,
}

/// Capability status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityStatus {
    /// Capability is available for use
    Available,
    /// Capability is coming soon (not yet implemented)
    ComingSoon,
    /// Capability is deprecated
    Deprecated,
}

impl std::fmt::Display for CapabilityStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilityStatus::Available => write!(f, "available"),
            CapabilityStatus::ComingSoon => write!(f, "coming_soon"),
            CapabilityStatus::Deprecated => write!(f, "deprecated"),
        }
    }
}

// ============================================================================
// Mount Point Types
// ============================================================================

// ============================================================================
// Virtual File Tree
// ============================================================================

/// A read-only, path-indexed tree of file content. Built once at startup,
/// shared across all sessions via `Arc`. Used by `MountSource::Virtual` to
/// serve files from memory without writing rows to `session_files`.
#[derive(Debug, Clone)]
pub struct VirtualFileTree {
    files: HashMap<String, VirtualFile>,
}

/// A single file in a virtual file tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualFile {
    pub content: Vec<u8>,
    pub is_directory: bool,
}

impl VirtualFileTree {
    pub fn new() -> Self {
        Self {
            files: HashMap::new(),
        }
    }

    /// Insert a text file at the given path (must start with "/").
    pub fn insert_text(&mut self, path: impl Into<String>, content: impl Into<String>) {
        let path = path.into();
        self.ensure_parent_dirs(&path);
        self.files.insert(
            path,
            VirtualFile {
                content: content.into().into_bytes(),
                is_directory: false,
            },
        );
    }

    /// Insert a directory entry at the given path.
    pub fn insert_directory(&mut self, path: impl Into<String>) {
        self.files.insert(
            path.into(),
            VirtualFile {
                content: Vec::new(),
                is_directory: true,
            },
        );
    }

    /// Get a file by path.
    pub fn get(&self, path: &str) -> Option<&VirtualFile> {
        self.files.get(path)
    }

    /// List entries directly under the given directory path.
    pub fn list_directory(&self, dir_path: &str) -> Vec<(String, &VirtualFile)> {
        let prefix = if dir_path == "/" {
            "/".to_string()
        } else {
            format!("{dir_path}/")
        };
        self.files
            .iter()
            .filter(|(p, _)| {
                if let Some(rest) = p.strip_prefix(&prefix) {
                    !rest.is_empty() && !rest.contains('/')
                } else {
                    false
                }
            })
            .map(|(p, f)| (p.clone(), f))
            .collect()
    }

    /// Iterate all files (non-directory entries) for grep.
    pub fn all_files(&self) -> impl Iterator<Item = (&str, &VirtualFile)> {
        self.files
            .iter()
            .filter(|(_, f)| !f.is_directory)
            .map(|(p, f)| (p.as_str(), f))
    }

    /// Number of entries in the tree.
    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    fn ensure_parent_dirs(&mut self, path: &str) {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let mut current = String::new();
        for part in &parts[..parts.len().saturating_sub(1)] {
            current = format!("{current}/{part}");
            self.files.entry(current.clone()).or_insert(VirtualFile {
                content: Vec::new(),
                is_directory: true,
            });
        }
    }
}

impl Default for VirtualFileTree {
    fn default() -> Self {
        Self::new()
    }
}

impl PartialEq for VirtualFileTree {
    fn eq(&self, other: &Self) -> bool {
        self.files == other.files
    }
}

impl Eq for VirtualFileTree {}

/// Access mode for mounted files/directories
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MountAccess {
    /// Files are read-only (cannot be modified or deleted by agents)
    #[default]
    ReadOnly,
    /// Files can be read and written by agents
    ReadWrite,
}

impl std::fmt::Display for MountAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MountAccess::ReadOnly => write!(f, "readonly"),
            MountAccess::ReadWrite => write!(f, "readwrite"),
        }
    }
}

/// Source content for a mount entry
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountSource {
    /// A single file with inline content
    InlineFile {
        /// File content (text or base64-encoded binary)
        content: String,
        /// Content encoding: "text" or "base64"
        encoding: String,
    },
    /// A directory containing multiple entries
    InlineDirectory {
        /// Map of filename to mount entry
        entries: HashMap<String, MountEntry>,
    },
    /// A virtual file tree served from memory. Read-only, shared across sessions via Arc.
    /// No DB rows created — content is served directly from the in-memory tree.
    Virtual { tree: Arc<VirtualFileTree> },
}

impl MountSource {
    /// Create an inline text file source
    pub fn text_file(content: impl Into<String>) -> Self {
        Self::InlineFile {
            content: content.into(),
            encoding: "text".to_string(),
        }
    }

    /// Create an inline base64-encoded binary file source
    pub fn binary_file(content: impl Into<String>) -> Self {
        Self::InlineFile {
            content: content.into(),
            encoding: "base64".to_string(),
        }
    }

    /// Create an inline directory source
    pub fn directory(entries: HashMap<String, MountEntry>) -> Self {
        Self::InlineDirectory { entries }
    }

    /// Create a virtual mount source from a shared file tree
    pub fn virtual_tree(tree: Arc<VirtualFileTree>) -> Self {
        Self::Virtual { tree }
    }

    /// Check if this source is a directory
    pub fn is_directory(&self) -> bool {
        matches!(self, Self::InlineDirectory { .. } | Self::Virtual { .. })
    }
}

/// A single entry in a mount (file or directory)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    /// The source content
    pub source: MountSource,
}

impl MountEntry {
    /// Create a new mount entry from a source
    pub fn new(source: MountSource) -> Self {
        Self { source }
    }

    /// Create a text file entry
    pub fn text_file(content: impl Into<String>) -> Self {
        Self::new(MountSource::text_file(content))
    }

    /// Create a directory entry with children
    pub fn directory(entries: HashMap<String, MountEntry>) -> Self {
        Self::new(MountSource::directory(entries))
    }
}

/// A mount point declaration from a capability
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPoint {
    /// Target path in the session filesystem (e.g., "/samples")
    pub path: String,
    /// Access mode for the mounted content
    pub access: MountAccess,
    /// The source content to mount
    pub source: MountSource,
    /// ID of the capability providing this mount
    pub capability_id: String,
}

impl MountPoint {
    /// Create a new mount point
    pub fn new(
        path: impl Into<String>,
        access: MountAccess,
        source: MountSource,
        capability_id: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            access,
            source,
            capability_id: capability_id.into(),
        }
    }

    /// Create a read-only mount point
    pub fn readonly(
        path: impl Into<String>,
        source: MountSource,
        capability_id: impl Into<String>,
    ) -> Self {
        Self::new(path, MountAccess::ReadOnly, source, capability_id)
    }

    /// Create a read-write mount point
    pub fn readwrite(
        path: impl Into<String>,
        source: MountSource,
        capability_id: impl Into<String>,
    ) -> Self {
        Self::new(path, MountAccess::ReadWrite, source, capability_id)
    }

    /// Check if this mount is read-only
    pub fn is_readonly(&self) -> bool {
        self.access == MountAccess::ReadOnly
    }
}

/// Builder for creating mount directories with a fluent API
#[derive(Debug, Default)]
pub struct MountDirectoryBuilder {
    entries: HashMap<String, MountEntry>,
}

impl MountDirectoryBuilder {
    /// Create a new directory builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a text file to the directory
    pub fn file(mut self, name: impl Into<String>, content: impl Into<String>) -> Self {
        self.entries
            .insert(name.into(), MountEntry::text_file(content));
        self
    }

    /// Add a subdirectory to the directory
    pub fn dir(mut self, name: impl Into<String>, builder: MountDirectoryBuilder) -> Self {
        self.entries
            .insert(name.into(), MountEntry::directory(builder.entries));
        self
    }

    /// Build the mount source
    pub fn build(self) -> MountSource {
        MountSource::directory(self.entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // AgentCapabilityConfig tests

    #[test]
    fn test_agent_capability_config_with_config() {
        let config = AgentCapabilityConfig::with_config(
            "web_fetch",
            serde_json::json!({"timeout_ms": 30000}),
        );
        assert_eq!(config.capability_id(), "web_fetch");
        assert_eq!(config.config_value()["timeout_ms"], 30000);
    }

    #[test]
    fn test_agent_capability_config_from_capability_id() {
        let config: AgentCapabilityConfig = CapabilityId::new("current_time").into();
        assert_eq!(config.capability_id(), "current_time");
        assert_eq!(config.config_value(), &serde_json::json!({}));
    }

    #[test]
    fn test_agent_capability_config_from_str() {
        let config: AgentCapabilityConfig = "test_math".into();
        assert_eq!(config.capability_id(), "test_math");
    }

    #[test]
    fn test_agent_capability_config_from_string() {
        let config: AgentCapabilityConfig = String::from("web_fetch").into();
        assert_eq!(config.capability_id(), "web_fetch");
    }

    #[test]
    fn test_agent_capability_config_serialization_empty_config() {
        let config = AgentCapabilityConfig::new("noop");
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"ref\":\"noop\""));

        let parsed: AgentCapabilityConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.capability_id(), "noop");
        assert_eq!(parsed.config_value(), &serde_json::json!({}));
    }

    #[test]
    fn test_agent_capability_config_equality() {
        let config1 = AgentCapabilityConfig::new("current_time");
        let config2 = AgentCapabilityConfig::new("current_time");
        let config3 =
            AgentCapabilityConfig::with_config("current_time", serde_json::json!({"key": "value"}));

        assert_eq!(config1, config2);
        assert_ne!(config1, config3); // Different config makes them unequal
    }

    // MountAccess tests

    #[test]
    fn test_mount_access_display() {
        assert_eq!(MountAccess::ReadOnly.to_string(), "readonly");
        assert_eq!(MountAccess::ReadWrite.to_string(), "readwrite");
    }

    #[test]
    fn test_mount_access_default() {
        assert_eq!(MountAccess::default(), MountAccess::ReadOnly);
    }

    // MountSource tests

    #[test]
    fn test_mount_source_text_file() {
        let source = MountSource::text_file("Hello, World!");
        assert!(!source.is_directory());
        match source {
            MountSource::InlineFile { content, encoding } => {
                assert_eq!(content, "Hello, World!");
                assert_eq!(encoding, "text");
            }
            _ => panic!("Expected InlineFile"),
        }
    }

    #[test]
    fn test_mount_source_binary_file() {
        let source = MountSource::binary_file("SGVsbG8=");
        match source {
            MountSource::InlineFile { content, encoding } => {
                assert_eq!(content, "SGVsbG8=");
                assert_eq!(encoding, "base64");
            }
            _ => panic!("Expected InlineFile"),
        }
    }

    #[test]
    fn test_mount_source_directory() {
        let mut entries = HashMap::new();
        entries.insert("test.txt".to_string(), MountEntry::text_file("content"));
        let source = MountSource::directory(entries);
        assert!(source.is_directory());
    }

    // MountEntry tests

    #[test]
    fn test_mount_entry_text_file() {
        let entry = MountEntry::text_file("content");
        match entry.source {
            MountSource::InlineFile { content, encoding } => {
                assert_eq!(content, "content");
                assert_eq!(encoding, "text");
            }
            _ => panic!("Expected InlineFile"),
        }
    }

    // MountPoint tests

    #[test]
    fn test_mount_point_new() {
        let source = MountSource::text_file("content");
        let mount = MountPoint::new("/test", MountAccess::ReadOnly, source, "test_cap");
        assert_eq!(mount.path, "/test");
        assert_eq!(mount.access, MountAccess::ReadOnly);
        assert_eq!(mount.capability_id, "test_cap");
        assert!(mount.is_readonly());
    }

    #[test]
    fn test_mount_point_readonly() {
        let source = MountSource::text_file("content");
        let mount = MountPoint::readonly("/test", source, "test_cap");
        assert!(mount.is_readonly());
        assert_eq!(mount.access, MountAccess::ReadOnly);
    }

    #[test]
    fn test_mount_point_readwrite() {
        let source = MountSource::text_file("content");
        let mount = MountPoint::readwrite("/test", source, "test_cap");
        assert!(!mount.is_readonly());
        assert_eq!(mount.access, MountAccess::ReadWrite);
    }

    // MountDirectoryBuilder tests

    #[test]
    fn test_mount_directory_builder() {
        let source = MountDirectoryBuilder::new()
            .file("readme.txt", "Hello")
            .file("config.json", "{}")
            .dir(
                "nested",
                MountDirectoryBuilder::new().file("inner.txt", "Nested content"),
            )
            .build();

        match source {
            MountSource::InlineDirectory { entries } => {
                assert_eq!(entries.len(), 3);
                assert!(entries.contains_key("readme.txt"));
                assert!(entries.contains_key("config.json"));
                assert!(entries.contains_key("nested"));

                // Check nested directory
                if let MountSource::InlineDirectory { entries: nested } =
                    &entries.get("nested").unwrap().source
                {
                    assert_eq!(nested.len(), 1);
                    assert!(nested.contains_key("inner.txt"));
                } else {
                    panic!("Expected nested InlineDirectory");
                }
            }
            _ => panic!("Expected InlineDirectory"),
        }
    }
}
