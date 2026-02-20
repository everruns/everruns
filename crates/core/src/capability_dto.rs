// Capability DTO types
//
// These types are API/DTO types for capabilities with ToSchema support.
// Runtime types (CapabilityId, CapabilityStatus) are in capability_types.rs.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use crate::capability_types::{CapabilityId, CapabilityStatus};
use crate::tool_types::ToolDefinition;

/// Public capability information (without internal details)
/// This is what gets returned from the API
/// Named CapabilityInfo to distinguish from the Capability trait
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CapabilityInfo {
    /// Unique capability identifier
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub id: CapabilityId,
    /// Display name
    pub name: String,
    /// Description of what this capability provides
    pub description: String,
    /// Current status
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub status: CapabilityStatus,
    /// Icon name (for UI rendering)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Category for grouping in UI
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// System prompt addition contributed by this capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// Tool definitions provided by this capability
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<Object>))]
    pub tool_definitions: Vec<ToolDefinition>,
    /// Whether this is an MCP server capability (for UI badge)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_mcp: bool,
    /// Whether this is an Agent Skill capability (for UI badge)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_skill: bool,
    /// IDs of capabilities that this capability depends on.
    /// When this capability is selected, its dependencies are automatically included.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub dependencies: Vec<String>,
    /// UI feature strings this capability contributes to.
    /// Multiple capabilities can contribute the same feature.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub features: Vec<String>,
}

impl CapabilityInfo {
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
            dependencies: cap.dependencies().iter().map(|s| s.to_string()).collect(),
            features: cap.features().iter().map(|s| s.to_string()).collect(),
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
            dependencies: vec![],
            features: vec![],
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
            dependencies: vec![],
            features: vec![],
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
            dependencies: vec!["session_file_system".to_string()],
            features: vec![],
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
            name: "Session Storage".to_string(),
            description: "Storage capability".to_string(),
            status: CapabilityStatus::Available,
            icon: None,
            category: None,
            system_prompt: None,
            tool_definitions: vec![],
            is_mcp: false,
            is_skill: false,
            dependencies: vec![],
            features: vec!["secrets".to_string(), "key_value".to_string()],
        };

        let json = serde_json::to_string(&cap).unwrap();
        assert!(json.contains("\"features\":[\"secrets\",\"key_value\"]"));
    }

    #[test]
    fn test_from_core_populates_features() {
        let registry = crate::capabilities::CapabilityRegistry::with_builtins();

        let schedule_cap = registry.get("session_schedule").unwrap();
        let info = CapabilityInfo::from_core(schedule_cap.as_ref());
        assert_eq!(info.features, vec!["schedules"]);

        let storage_cap = registry.get("session_storage").unwrap();
        let info = CapabilityInfo::from_core(storage_cap.as_ref());
        assert!(info.features.contains(&"secrets".to_string()));
        assert!(info.features.contains(&"key_value".to_string()));

        // Capability with no features
        let noop_cap = registry.get("noop").unwrap();
        let info = CapabilityInfo::from_core(noop_cap.as_ref());
        assert!(info.features.is_empty());
    }
}
