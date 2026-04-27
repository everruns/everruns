//! Workspace Volumes capability (EVE-396)
//!
//! Mounts org-scoped Volumes into session workspaces. See `specs/volumes.md`
//! for the durable design.
//!
//! This module registers the capability and validates the structural shape of
//! its config (`mounts[]` entries: `vol_`-prefixed Volume IDs, paths under
//! `/workspace`, no `..`, no `//`, no overlaps). Domain-level cross-validation
//! (cross-org references, archived/deleted Volumes, capability-mount overlaps,
//! reserved system paths) and runtime mount resolution / read-only write
//! protection / read-write writeback ship in follow-up vertical slices on top
//! of this foundation.

use serde_json::{Value, json};

use super::{Capability, CapabilityStatus, RiskLevel};
use crate::volume::{WorkspaceVolumesConfig, validate_workspace_volumes_config};

/// Stable string id for the workspace volumes capability.
pub const WORKSPACE_VOLUMES_CAPABILITY_ID: &str = "workspace_volumes";

pub struct WorkspaceVolumesCapability;

impl Capability for WorkspaceVolumesCapability {
    fn id(&self) -> &str {
        WORKSPACE_VOLUMES_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Workspace Volumes"
    }

    fn description(&self) -> &str {
        "Mount org-scoped, named filesystem trees into the session workspace as \
         read-only reference data or read-write shared working memory."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("hard-drive")
    }

    fn category(&self) -> Option<&str> {
        Some("File System")
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
                    "description": "Volumes mounted into /workspace for sessions using this capability.",
                    "items": {
                        "type": "object",
                        "required": ["volume", "path"],
                        "properties": {
                            "volume": {
                                "type": "string",
                                "title": "Volume",
                                "description": "Volume ID (vol_<32-hex>) to mount.",
                                "pattern": "^vol_[0-9a-f]{32}$"
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
        let typed: WorkspaceVolumesConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("invalid workspace_volumes config: {e}"))?;
        validate_workspace_volumes_config(&typed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_and_name() {
        let cap = WorkspaceVolumesCapability;
        assert_eq!(cap.id(), "workspace_volumes");
        assert_eq!(cap.name(), "Workspace Volumes");
    }

    #[test]
    fn dependencies_include_file_system() {
        let cap = WorkspaceVolumesCapability;
        assert_eq!(cap.dependencies(), vec!["session_file_system"]);
    }

    #[test]
    fn validate_accepts_empty_config() {
        let cap = WorkspaceVolumesCapability;
        assert!(cap.validate_config(&json!({})).is_ok());
        assert!(cap.validate_config(&json!({ "mounts": [] })).is_ok());
        assert!(cap.validate_config(&Value::Null).is_ok());
    }

    #[test]
    fn validate_accepts_well_formed_mount() {
        let cap = WorkspaceVolumesCapability;
        let cfg = json!({
            "mounts": [
                {
                    "volume": "vol_00000000000000000000000000000001",
                    "path": "/workspace/research",
                    "mode": "readonly"
                }
            ]
        });
        assert!(cap.validate_config(&cfg).is_ok());
    }

    #[test]
    fn validate_rejects_overlapping_mounts() {
        let cap = WorkspaceVolumesCapability;
        let cfg = json!({
            "mounts": [
                {
                    "volume": "vol_00000000000000000000000000000001",
                    "path": "/workspace/data"
                },
                {
                    "volume": "vol_00000000000000000000000000000002",
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
        let cap = WorkspaceVolumesCapability;
        let cfg = json!({
            "mounts": [
                { "volume": "vol_00000000000000000000000000000001", "path": "/etc/passwd" }
            ]
        });
        assert!(cap.validate_config(&cfg).is_err());
    }

    #[test]
    fn config_schema_is_present() {
        let cap = WorkspaceVolumesCapability;
        let schema = cap.config_schema().expect("config schema");
        assert_eq!(schema["type"], "object");
    }
}
