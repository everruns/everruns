// Capability DTO types
//
// These types are API/DTO types for capabilities with ToSchema support.
// Runtime types (CapabilityId, CapabilityStatus) are in capability_types.rs.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::capabilities::RiskLevel;
use crate::capability_types::{CapabilityId, CapabilityStatus};
use crate::tool_types::ToolDefinition;

/// Public capability information (without internal details)
/// This is what gets returned from the API
/// Named CapabilityInfo to distinguish from the Capability trait
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CapabilityInfo {
    /// Unique capability identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "session_file_system"))]
    pub id: CapabilityId,
    /// Display name
    #[cfg_attr(feature = "openapi", schema(example = "Session File System"))]
    pub name: String,
    /// Description of what this capability provides
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "Read, write, edit, list, grep, delete, and stat files in the session workspace."
        )
    )]
    pub description: String,
    /// Current status
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "active"))]
    pub status: CapabilityStatus,
    /// Icon name (for UI rendering)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "Folder"))]
    pub icon: Option<String>,
    /// Category for grouping in UI
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "filesystem"))]
    pub category: Option<String>,
    /// System prompt addition contributed by this capability
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "You can read and write files in /workspace via the session_file_system tools."
        )
    )]
    pub system_prompt: Option<String>,
    /// Tool definitions provided by this capability
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(
        feature = "openapi",
        schema(
            value_type = Vec<Object>,
            example = json!([
                {"name": "read_file", "description": "Read a file from the session workspace."},
                {"name": "write_file", "description": "Write or overwrite a file in the session workspace."}
            ])
        )
    )]
    pub tool_definitions: Vec<ToolDefinition>,
    /// Whether this is an MCP server capability (for UI badge)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "openapi", schema(example = false))]
    pub is_mcp: bool,
    /// Whether this is an Agent Skill capability (for UI badge)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "openapi", schema(example = false))]
    pub is_skill: bool,
    /// Whether this capability is a guardrail (constrains agent behavior
    /// rather than granting abilities). Used for UI grouping and filtering.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "openapi", schema(example = false))]
    pub is_guardrail: bool,
    /// IDs of capabilities that this capability depends on.
    /// When this capability is selected, its dependencies are automatically included.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "openapi", schema(example = json!(["approval"])))]
    pub dependencies: Vec<String>,
    /// UI feature strings this capability contributes to.
    /// Multiple capabilities can contribute the same feature.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "openapi", schema(example = json!(["file_browser"])))]
    pub features: Vec<String>,
    /// JSON Schema for capability-specific per-agent config.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(
            value_type = Object,
            example = json!({
                "type": "object",
                "properties": {"max_file_bytes": {"type": "integer", "default": 1048576}}
            })
        )
    )]
    pub config_schema: Option<serde_json::Value>,
    /// react-jsonschema-form uiSchema hints for rendering config_schema.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(
            value_type = Object,
            example = json!({"max_file_bytes": {"ui:widget": "updown"}})
        )
    )]
    pub config_ui_schema: Option<serde_json::Value>,
    /// TM-AGENT-005: Risk level. High-risk capabilities require admin approval.
    #[serde(skip_serializing_if = "is_low_risk", default = "default_risk_level")]
    #[cfg_attr(feature = "openapi", schema(example = "low"))]
    pub risk_level: RiskLevel,
    /// Number of active agents referencing this capability in the org.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    #[cfg_attr(feature = "openapi", schema(example = 42u64))]
    pub agent_count: u64,
    /// Number of active harnesses referencing this capability in the org.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    #[cfg_attr(feature = "openapi", schema(example = 7u64))]
    pub harness_count: u64,
    #[allow(rustdoc::bare_urls)]
    /// Slug under https://dev.everruns.com/capabilities/ when public docs exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(example = "session_file_system"))]
    pub docs_slug: Option<String>,
    /// Localized display strings keyed by lowercase language tag (e.g. "uk").
    /// The "en" entry carries only `config_description`, since the base
    /// name/description/config_schema strings are already English.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = json!({
                "uk": {"name": "Пам'ять", "description": "Монтує спільні файли пам'яті в сесії."}
            })
        )
    )]
    pub localizations: std::collections::BTreeMap<String, CapabilityLocalizationInfo>,
}

/// Localized display strings for one locale of a capability.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CapabilityLocalizationInfo {
    /// Localized display name; absent means fall back to `name`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Localized description; absent means fall back to `description`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// One-line summary of what this capability's config controls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_description: Option<String>,
    /// Overlay merged into `config_schema` before rendering: mirrors the
    /// schema property tree with `title`/`description`/`enum_labels` leaves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub config_overlay: Option<serde_json::Value>,
}

fn is_low_risk(r: &RiskLevel) -> bool {
    *r == RiskLevel::Low
}
fn default_risk_level() -> RiskLevel {
    RiskLevel::Low
}
fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

#[allow(rustdoc::bare_urls)]
/// Mapping from built-in capability ID to its docs slug under
/// https://dev.everruns.com/capabilities/. Returns None for IDs that
/// have no published documentation page.
pub fn builtin_capability_docs_slug(id: &str) -> Option<&'static str> {
    match id {
        "agent_instructions" => Some("agent-instructions"),
        "skills" => Some("agent-skills"),
        "browserless" => Some("browserless"),
        "budgeting" => Some("budgeting"),
        "current_time" => Some("current-time"),
        "daytona" => Some("daytona"),
        "fake_aws" => Some("fake-aws"),
        "fake_crm" => Some("fake-crm"),
        "fake_warehouse" => Some("fake-warehouse"),
        "github_scout" => Some("github-scout"),
        "session_file_system" => Some("file-system"),
        "infinity_context" => Some("infinity-context"),
        "openai_image_generation" => Some("openai-image-generation"),
        "openai_tool_search" => Some("openai-tool-search"),
        "tool_search" => Some("tool-search"),
        "auto_tool_search" => Some("auto-tool-search"),
        "platform_management" => Some("platform-management"),
        "prompt_canary_guardrail" => Some("prompt-canary-guardrail"),
        "self_budget" => Some("self-budget"),
        "session_schedule" => Some("session-schedules"),
        "session_storage" => Some("session-storage"),
        "session_sandbox" => Some("session"),
        "session_sql_database" => Some("sql-database"),
        "subagents" => Some("sub-agents"),
        "stateless_todo_list" => Some("task-management"),
        "bashkit_shell" => Some("bashkit-shell"),
        "web_fetch" => Some("web-fetch"),
        _ => None,
    }
}

impl CapabilityInfo {
    /// Case-insensitive search across name, description, category, and ID.
    pub fn matches_search(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.name.to_lowercase().contains(&q)
            || self.description.to_lowercase().contains(&q)
            || self.id.as_str().to_lowercase().contains(&q)
            || self
                .category
                .as_deref()
                .is_some_and(|cat| cat.to_lowercase().contains(&q))
    }

    /// Create a CapabilityInfo DTO from a core Capability trait object
    pub fn from_core(cap: &dyn crate::capabilities::Capability) -> Self {
        // Check if this is an MCP or skill capability by checking the ID prefix
        let id_str = cap.id();
        let is_mcp = id_str.starts_with("mcp:");
        let is_skill =
            id_str.starts_with("skill:") || id_str == "skills" || cap.category() == Some("Skills");

        Self {
            id: CapabilityId::new(id_str),
            name: cap.name().to_string(),
            description: cap.description().to_string(),
            status: cap.status(),
            icon: cap.icon().map(|s| s.to_string()),
            category: cap.category().map(|s| s.to_string()),
            system_prompt: cap.system_prompt_preview(),
            tool_definitions: cap.tool_definitions(),
            is_mcp,
            is_skill,
            is_guardrail: cap.is_guardrail(),
            dependencies: cap.dependencies().iter().map(|s| s.to_string()).collect(),
            features: cap.features().iter().map(|s| s.to_string()).collect(),
            config_schema: cap.config_schema(),
            config_ui_schema: cap.config_ui_schema(),
            risk_level: cap.risk_level(),
            agent_count: 0,
            harness_count: 0,
            docs_slug: builtin_capability_docs_slug(id_str).map(|s| s.to_string()),
            localizations: cap
                .localizations()
                .into_iter()
                .map(|entry| {
                    (
                        entry.locale.to_lowercase(),
                        CapabilityLocalizationInfo {
                            name: entry.name.map(str::to_string),
                            description: entry.description.map(str::to_string),
                            config_description: entry.config_description.map(str::to_string),
                            config_overlay: entry.config_overlay,
                        },
                    )
                })
                .collect(),
        }
    }
}

/// Agent capability assignment with ordering
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AgentCapability {
    /// The capability ID
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub capability_id: CapabilityId,
    /// Position in the chain (lower = earlier)
    pub position: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_info_serialization() {
        let cap = CapabilityInfo {
            id: CapabilityId::new("research"),
            name: "Research".to_string(),
            description: "Deep research capability".to_string(),
            status: CapabilityStatus::Available,
            icon: Some("search".to_string()),
            category: Some("AI".to_string()),
            system_prompt: Some("You have research capabilities.".to_string()),
            tool_definitions: vec![],
            is_mcp: false,
            is_skill: false,
            is_guardrail: false,
            dependencies: vec![],
            features: vec![],
            config_schema: None,
            config_ui_schema: None,
            risk_level: RiskLevel::Low,
            agent_count: 0,
            harness_count: 0,
            docs_slug: None,
            localizations: Default::default(),
        };

        let json = serde_json::to_string(&cap).unwrap();
        assert!(json.contains("\"id\":\"research\""));
        assert!(json.contains("\"status\":\"available\""));
        assert!(json.contains("\"system_prompt\":\"You have research capabilities.\""));
        // is_mcp: false should be skipped in serialization
        assert!(!json.contains("\"is_mcp\""));
        // is_skill: false should be skipped in serialization
        assert!(!json.contains("\"is_skill\""));
        // Empty dependencies should be skipped in serialization
        assert!(!json.contains("\"dependencies\""));
        // Empty features should be skipped in serialization
        assert!(!json.contains("\"features\""));
    }

    #[test]
    fn test_mcp_capability_info_serialization() {
        let cap = CapabilityInfo {
            id: CapabilityId::new("mcp:550e8400-e29b-41d4-a716-446655440000"),
            name: "Microsoft Learn".to_string(),
            description: "MCP Server for Microsoft documentation".to_string(),
            status: CapabilityStatus::Available,
            icon: Some("plug".to_string()),
            category: Some("MCP Servers".to_string()),
            system_prompt: None,
            tool_definitions: vec![],
            is_mcp: true,
            is_skill: false,
            is_guardrail: false,
            dependencies: vec![],
            features: vec![],
            config_schema: None,
            config_ui_schema: None,
            risk_level: RiskLevel::Low,
            agent_count: 0,
            harness_count: 0,
            docs_slug: None,
            localizations: Default::default(),
        };

        let json = serde_json::to_string(&cap).unwrap();
        assert!(json.contains("\"is_mcp\":true"));
    }

    #[test]
    fn test_capability_with_dependencies_serialization() {
        let cap = CapabilityInfo {
            id: CapabilityId::new("sample_data"),
            name: "Sample Data".to_string(),
            description: "Sample data for testing".to_string(),
            status: CapabilityStatus::Available,
            icon: None,
            category: None,
            system_prompt: None,
            tool_definitions: vec![],
            is_mcp: false,
            is_skill: false,
            is_guardrail: false,
            dependencies: vec!["session_file_system".to_string()],
            features: vec![],
            config_schema: None,
            config_ui_schema: None,
            risk_level: RiskLevel::Low,
            agent_count: 0,
            harness_count: 0,
            docs_slug: None,
            localizations: Default::default(),
        };

        let json = serde_json::to_string(&cap).unwrap();
        assert!(json.contains("\"dependencies\":[\"session_file_system\"]"));
    }

    #[test]
    fn test_agent_capability_serialization() {
        let agent_cap = AgentCapability {
            capability_id: CapabilityId::new("test_math"),
            position: 1,
        };

        let json = serde_json::to_string(&agent_cap).unwrap();
        assert!(json.contains("\"capability_id\":\"test_math\""));
        assert!(json.contains("\"position\":1"));
    }

    #[test]
    fn test_test_capabilities() {
        // Verify test math and weather capabilities are available
        assert_eq!(CapabilityId::new("test_math").to_string(), "test_math");
        assert_eq!(
            CapabilityId::new("test_weather").to_string(),
            "test_weather"
        );
    }

    #[test]
    fn test_custom_capability_id() {
        // Custom capability IDs should work
        let custom = CapabilityId::new("my_custom_capability");
        assert_eq!(custom.to_string(), "my_custom_capability");

        let json = serde_json::to_string(&custom).unwrap();
        assert_eq!(json, "\"my_custom_capability\"");
    }

    #[test]
    fn test_capability_with_features_serialization() {
        let cap = CapabilityInfo {
            id: CapabilityId::new("session_storage"),
            name: "Storage".to_string(),
            description: "Storage capability".to_string(),
            status: CapabilityStatus::Available,
            icon: None,
            category: None,
            system_prompt: None,
            tool_definitions: vec![],
            is_mcp: false,
            is_skill: false,
            is_guardrail: false,
            dependencies: vec![],
            features: vec!["secrets".to_string(), "key_value".to_string()],
            config_schema: None,
            config_ui_schema: None,
            risk_level: RiskLevel::Low,
            agent_count: 0,
            harness_count: 0,
            docs_slug: None,
            localizations: Default::default(),
        };

        let json = serde_json::to_string(&cap).unwrap();
        assert!(json.contains("\"features\":[\"secrets\",\"key_value\"]"));
    }

    #[test]
    fn test_from_core_populates_features() {
        /// Declares features the way the product capabilities do. The ones that
        /// used to stand in here (`session_storage`) moved out of the kernel
        /// (EVE-886); what this test covers is the projection.
        struct FeatureCapability;

        impl crate::capabilities::Capability for FeatureCapability {
            fn id(&self) -> &str {
                "feature_fixture"
            }
            fn name(&self) -> &str {
                "Feature Fixture"
            }
            fn description(&self) -> &str {
                "Fixture capability declaring features."
            }
            fn features(&self) -> Vec<&'static str> {
                vec!["secrets", "key_value"]
            }
        }

        let info = CapabilityInfo::from_core(&FeatureCapability);
        assert!(info.features.contains(&"secrets".to_string()));
        assert!(info.features.contains(&"key_value".to_string()));

        struct FeaturelessCapability;
        impl crate::capabilities::Capability for FeaturelessCapability {
            fn id(&self) -> &str {
                "featureless_fixture"
            }
            fn name(&self) -> &str {
                "Featureless Fixture"
            }
            fn description(&self) -> &str {
                "Fixture with no declared features."
            }
        }
        let info = CapabilityInfo::from_core(&FeaturelessCapability);
        assert!(info.features.is_empty());
    }

    #[test]
    fn test_risk_level_serialization() {
        // Low risk should be skipped in serialization
        let cap = CapabilityInfo {
            id: CapabilityId::new("safe"),
            name: "Safe".to_string(),
            description: "Low risk".to_string(),
            status: CapabilityStatus::Available,
            icon: None,
            category: None,
            system_prompt: None,
            tool_definitions: vec![],
            is_mcp: false,
            is_skill: false,
            is_guardrail: false,
            dependencies: vec![],
            features: vec![],
            config_schema: None,
            config_ui_schema: None,
            risk_level: RiskLevel::Low,
            agent_count: 0,
            harness_count: 0,
            docs_slug: None,
            localizations: Default::default(),
        };
        let json = serde_json::to_string(&cap).unwrap();
        assert!(
            !json.contains("\"risk_level\""),
            "Low risk should be omitted"
        );

        // High risk should be present
        let cap_high = CapabilityInfo {
            risk_level: RiskLevel::High,
            ..cap
        };
        let json = serde_json::to_string(&cap_high).unwrap();
        assert!(json.contains("\"risk_level\":\"high\""));
    }

    #[test]
    fn test_from_core_populates_risk_level() {
        struct HighRiskCapability;
        impl crate::capabilities::Capability for HighRiskCapability {
            fn id(&self) -> &str {
                "high_risk_fixture"
            }
            fn name(&self) -> &str {
                "High Risk Fixture"
            }
            fn description(&self) -> &str {
                "Fixture for risk projection."
            }
            fn risk_level(&self) -> RiskLevel {
                RiskLevel::High
            }
        }
        struct LowRiskCapability;
        impl crate::capabilities::Capability for LowRiskCapability {
            fn id(&self) -> &str {
                "low_risk_fixture"
            }
            fn name(&self) -> &str {
                "Low Risk Fixture"
            }
            fn description(&self) -> &str {
                "Fixture for default risk projection."
            }
        }

        let info = CapabilityInfo::from_core(&HighRiskCapability);
        assert_eq!(info.risk_level, RiskLevel::High);

        let info = CapabilityInfo::from_core(&LowRiskCapability);
        assert_eq!(info.risk_level, RiskLevel::Low);
    }

    #[test]
    fn test_matches_search() {
        let cap = CapabilityInfo {
            id: CapabilityId::new("web_fetch"),
            name: "Web Fetch".to_string(),
            description: "Fetch content from URLs".to_string(),
            status: CapabilityStatus::Available,
            icon: None,
            category: Some("Network".to_string()),
            system_prompt: None,
            tool_definitions: vec![],
            is_mcp: false,
            is_skill: false,
            is_guardrail: false,
            dependencies: vec![],
            features: vec![],
            config_schema: None,
            config_ui_schema: None,
            risk_level: RiskLevel::Low,
            agent_count: 0,
            harness_count: 0,
            docs_slug: None,
            localizations: Default::default(),
        };

        // Matches by name (case-insensitive)
        assert!(cap.matches_search("web"));
        assert!(cap.matches_search("WEB FETCH"));
        // Matches by description
        assert!(cap.matches_search("urls"));
        // Matches by ID
        assert!(cap.matches_search("web_fetch"));
        // Matches by category
        assert!(cap.matches_search("network"));
        // No match
        assert!(!cap.matches_search("zzz_nonexistent"));
    }
}
