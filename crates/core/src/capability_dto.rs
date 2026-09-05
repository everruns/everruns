// Capability DTO types
//
// These types are API/DTO types for capabilities with ToSchema support.
// Runtime types (CapabilityId, CapabilityStatus) are in capability_types.rs.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::capabilities::RiskLevel;
use crate::capability_types::CapabilityStatus;
use crate::tool_types::ToolDefinition;
use everruns_capability::CapabilityId;

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
    use serde_json::json;

    fn capability_info() -> CapabilityInfo {
        CapabilityInfo {
            id: CapabilityId::new("web_fetch"),
            name: "Web Fetch".into(),
            description: "Fetch content from URLs".into(),
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
        }
    }

    #[test]
    fn capability_info_omits_optional_defaults() {
        let expected = json!({"id": "web_fetch", "name": "Web Fetch", "description": "Fetch content from URLs", "status": "available"});
        assert_eq!(serde_json::to_value(capability_info()).unwrap(), expected);
        let parsed: CapabilityInfo = serde_json::from_value(expected).unwrap();
        assert_eq!(parsed.risk_level, RiskLevel::Low);
        assert!(!parsed.is_mcp && !parsed.is_skill && !parsed.is_guardrail);
        assert!(parsed.dependencies.is_empty() && parsed.features.is_empty());
    }

    #[test]
    fn capability_info_serializes_nondefault_metadata() {
        let mut cap = capability_info();
        cap.icon = Some("search".into());
        cap.category = Some("AI".into());
        cap.system_prompt = Some("Research carefully.".into());
        cap.is_mcp = true;
        cap.is_skill = true;
        cap.is_guardrail = true;
        cap.dependencies = vec!["session_file_system".into()];
        cap.features = vec!["secrets".into(), "key_value".into()];
        cap.agent_count = 3;
        cap.harness_count = 2;
        for (risk, wire) in [(RiskLevel::Medium, "medium"), (RiskLevel::High, "high")] {
            cap.risk_level = risk;
            let expected = json!({
                "id": "web_fetch", "name": "Web Fetch", "description": "Fetch content from URLs", "status": "available",
                "icon": "search", "category": "AI", "system_prompt": "Research carefully.",
                "is_mcp": true, "is_skill": true, "is_guardrail": true,
                "dependencies": ["session_file_system"], "features": ["secrets", "key_value"],
                "risk_level": wire, "agent_count": 3, "harness_count": 2,
            });
            assert_eq!(serde_json::to_value(&cap).unwrap(), expected);
        }
    }

    #[test]
    fn test_agent_capability_serialization() {
        let agent_cap = AgentCapability {
            capability_id: CapabilityId::new("test_math"),
            position: 1,
        };
        assert_eq!(
            serde_json::to_value(&agent_cap).unwrap(),
            json!({"capability_id": "test_math", "position": 1})
        );
    }

    struct ProjectionFixture {
        features: Vec<&'static str>,
        risk: RiskLevel,
    }
    impl crate::capabilities::Capability for ProjectionFixture {
        fn id(&self) -> &str {
            "projection_fixture"
        }
        fn name(&self) -> &str {
            "Projection Fixture"
        }
        fn description(&self) -> &str {
            "Independent capability metadata."
        }
        fn features(&self) -> Vec<&'static str> {
            self.features.clone()
        }
        fn risk_level(&self) -> RiskLevel {
            self.risk
        }
    }

    struct DefaultCapability;
    impl crate::capabilities::Capability for DefaultCapability {
        fn id(&self) -> &str {
            "default_fixture"
        }
        fn name(&self) -> &str {
            "Default Fixture"
        }
        fn description(&self) -> &str {
            "Uses trait metadata defaults."
        }
    }

    #[test]
    fn test_from_core_populates_features() {
        assert!(
            CapabilityInfo::from_core(&DefaultCapability)
                .features
                .is_empty()
        );
        for features in [vec!["secrets", "key_value"], vec![]] {
            let info = CapabilityInfo::from_core(&ProjectionFixture {
                features: features.clone(),
                risk: RiskLevel::Low,
            });
            assert_eq!(info.features, features);
        }
    }

    #[test]
    fn test_from_core_populates_risk_level() {
        assert_eq!(
            CapabilityInfo::from_core(&DefaultCapability).risk_level,
            RiskLevel::Low
        );
        for risk in [RiskLevel::Low, RiskLevel::Medium, RiskLevel::High] {
            let info = CapabilityInfo::from_core(&ProjectionFixture {
                features: vec![],
                risk,
            });
            assert_eq!(info.risk_level, risk);
        }
    }

    #[test]
    fn test_matches_search() {
        let mut cap = capability_info();
        cap.id = CapabilityId::new("unique_identifier");
        cap.name = "Display Label".into();
        cap.description = "Fetch content from URLs".into();
        cap.category = Some("Network".into());
        for query in ["dIsPlAy", "URLS", "UNIQUE_IDENTIFIER", "nEtWoRk"] {
            assert!(cap.matches_search(query), "{query}");
        }
        assert!(!cap.matches_search("zzz_nonexistent"));
        cap.category = None;
        assert!(!cap.matches_search("network"));
    }
}
