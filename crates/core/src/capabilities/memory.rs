//! Memory capability
//!
//! Mounts org-scoped Memories into session workspaces. See `specs/memory.md`
//! for the durable design.
//!
//! This module registers the capability and validates the structural shape of
//! its config (`mounts[]` entries: `mem_`-prefixed Memory IDs, paths under
//! `/workspace`, no `..`, no `//`, no overlaps). Domain-level cross-validation
//! (cross-org references, archived/deleted Memories, capability-mount overlaps,
//! reserved system paths) and runtime mount resolution / read-only write
//! protection / read-write writeback ship in follow-up vertical slices on top
//! of this foundation.

use serde_json::{Value, json};

use super::{Capability, CapabilityStatus, RiskLevel};
use crate::memory::{MemoryConfig, validate_memory_config};

/// Stable string id for the memory capability.
pub const MEMORY_CAPABILITY_ID: &str = "memory";

pub struct MemoryCapability;

impl Capability for MemoryCapability {
    fn id(&self) -> &str {
        MEMORY_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Memory"
    }

    fn description(&self) -> &str {
        "Mount org-scoped, named Memories into the session workspace as \
         read-only reference data or read-write shared working memory."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("brain")
    }

    fn category(&self) -> Option<&str> {
        Some("Memory")
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }

    /// Read-write shared mounts let one session influence future sessions, so
    /// classify as Medium risk per `specs/threat-model.md`.
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Medium
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "mounts": {
                    "type": "array",
                    "title": "Mounts",
                    "description": "Memories mounted into /workspace for sessions using this capability.",
                    "items": {
                        "type": "object",
                        "required": ["memory", "path"],
                        "properties": {
                            "memory": {
                                "type": "string",
                                "title": "Memory",
                                "description": "Memory ID (mem_<32-hex>) to mount.",
                                "pattern": "^mem_[0-9a-f]{32}$"
                            },
                            "path": {
                                "type": "string",
                                "title": "Mount path",
                                "description": "Absolute path under /workspace (e.g. /workspace/research).",
                                "pattern": "^/workspace(/[^/\\0]+)*$"
                            },
                            "mode": {
                                "type": "string",
                                "title": "Access mode",
                                "description": "Access mode for the mount.",
                                "enum": ["readonly", "readwrite"],
                                "default": "readonly"
                            }
                        }
                    }
                }
            }
        }))
    }

    fn config_ui_schema(&self) -> Option<Value> {
        Some(json!({
            "mounts": {
                "items": {
                    "mode": { "ui:widget": "select" }
                }
            }
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        // Empty / null configs are valid (no mounts).
        if config.is_null() {
            return Ok(());
        }
        let typed: MemoryConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid memory config: {e}"))?;
        validate_memory_config(&typed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_name() {
        let cap = MemoryCapability;
        assert_eq!(cap.id(), "memory");
        assert_eq!(cap.name(), "Memory");
    }

    #[test]
    fn dependencies_include_file_system() {
        let cap = MemoryCapability;
        assert_eq!(cap.dependencies(), vec!["session_file_system"]);
    }

    #[test]
    fn validate_accepts_empty_config() {
        let cap = MemoryCapability;
        assert!(cap.validate_config(&json!({})).is_ok());
        assert!(cap.validate_config(&json!({ "mounts": [] })).is_ok());
        assert!(cap.validate_config(&Value::Null).is_ok());
    }

    #[test]
    fn validate_accepts_well_formed_mount() {
        let cap = MemoryCapability;
        let cfg = json!({
            "mounts": [
                {
                    "memory": "mem_00000000000000000000000000000001",
                    "path": "/workspace/research",
                    "mode": "readonly"
                }
            ]
        });
        assert!(cap.validate_config(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_overlapping_mounts() {
        let cap = MemoryCapability;
        let cfg = json!({
            "mounts": [
                {
                    "memory": "mem_00000000000000000000000000000001",
                    "path": "/workspace/data"
                },
                {
                    "memory": "mem_00000000000000000000000000000002",
                    "path": "/workspace/data/inner",
                    "mode": "readwrite"
                }
            ]
        });
        let err = cap.validate_config(&cfg).unwrap_err();
        assert!(err.contains("overlapping"));
    }

    #[test]
    fn validate_rejects_path_outside_workspace() {
        let cap = MemoryCapability;
        let cfg = json!({
            "mounts": [
                { "memory": "mem_00000000000000000000000000000001", "path": "/etc/passwd" }
            ]
        });
        assert!(cap.validate_config(&cfg).is_err());
    }

    #[test]
    fn config_schema_is_present() {
        let cap = MemoryCapability;
        let schema = cap.config_schema().expect("config schema");
        assert_eq!(schema["type"], "object");
    }
}
