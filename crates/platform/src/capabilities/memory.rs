//! Memory capability
//!
//! Mounts org-scoped Memories into session workspaces. See `knowledge/runtime-resources/memory.md`
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

use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel};
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
    /// classify as Medium risk per `knowledge/security/threat-model.md`.
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
                                "oneOf": [
                                    { "const": "readonly", "title": "Read-only" },
                                    { "const": "readwrite", "title": "Read-write" }
                                ],
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

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Choose which Memories are mounted into /workspace and whether each \
                     mount is read-only or read-write.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Пам'ять"),
                description: Some(
                    "Підключає іменовані Пам'яті організації до робочого простору сесії як \
                     довідкові дані лише для читання або як спільну робочу пам'ять із \
                     читанням і записом.",
                ),
                config_description: Some(
                    "Визначає, які Пам'яті монтуються у /workspace і чи доступні вони лише \
                     для читання, чи для читання й запису.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        "mounts": {
                            "title": "Монтування",
                            "description": "Пам'яті, що монтуються у /workspace для сесій із цією можливістю.",
                            "items": {
                                "properties": {
                                    "memory": {
                                        "title": "Пам'ять",
                                        "description": "Ідентифікатор Пам'яті (mem_<32-hex>) для монтування."
                                    },
                                    "path": {
                                        "title": "Шлях монтування",
                                        "description": "Абсолютний шлях у /workspace (наприклад, /workspace/research)."
                                    },
                                    "mode": {
                                        "title": "Режим доступу",
                                        "description": "Режим доступу до монтування.",
                                        "enum_labels": {
                                            "readonly": "Лише читання",
                                            "readwrite": "Читання й запис"
                                        }
                                    }
                                }
                            }
                        }
                    }
                })),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Metadata/dependency constants covered by builtin_capabilities_satisfy_registry_invariants.

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
