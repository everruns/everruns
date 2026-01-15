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
}

impl CapabilityInfo {
    /// Create a CapabilityInfo DTO from a core Capability trait object
    pub fn from_core(cap: &dyn crate::capabilities::Capability) -> Self {
        // Check if this is an MCP capability by checking the ID prefix
        let id_str = cap.id();
        let is_mcp = id_str.starts_with("mcp:");

        Self {
            id: CapabilityId::new(id_str),
            name: cap.name().to_string(),
            description: cap.description().to_string(),
            status: cap.status(),
            icon: cap.icon().map(|s| s.to_string()),
            category: cap.category().map(|s| s.to_string()),
            system_prompt: cap.system_prompt_addition().map(|s| s.to_string()),
            tool_definitions: cap.tool_definitions(),
            is_mcp,
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
            id: CapabilityId::research(),
            name: "Research".to_string(),
            description: "Deep research capability".to_string(),
            status: CapabilityStatus::Available,
            icon: Some("search".to_string()),
            category: Some("AI".to_string()),
            system_prompt: Some("You have research capabilities.".to_string()),
            tool_definitions: vec![],
            is_mcp: false,
        };

        let json = serde_json::to_string(&cap).unwrap();
        assert!(json.contains("\"id\":\"research\""));
        assert!(json.contains("\"status\":\"available\""));
        assert!(json.contains("\"system_prompt\":\"You have research capabilities.\""));
        // is_mcp: false should be skipped in serialization
        assert!(!json.contains("\"is_mcp\""));
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
        };

        let json = serde_json::to_string(&cap).unwrap();
        assert!(json.contains("\"is_mcp\":true"));
    }

    #[test]
    fn test_agent_capability_serialization() {
        let agent_cap = AgentCapability {
            capability_id: CapabilityId::sandbox(),
            position: 1,
        };

        let json = serde_json::to_string(&agent_cap).unwrap();
        assert!(json.contains("\"capability_id\":\"sandbox\""));
        assert!(json.contains("\"position\":1"));
    }

    #[test]
    fn test_test_capabilities() {
        // Verify test math and weather capabilities are available
        assert_eq!(CapabilityId::test_math().to_string(), "test_math");
        assert_eq!(CapabilityId::test_weather().to_string(), "test_weather");
    }

    #[test]
    fn test_custom_capability_id() {
        // Custom capability IDs should work
        let custom = CapabilityId::new("my_custom_capability");
        assert_eq!(custom.to_string(), "my_custom_capability");

        let json = serde_json::to_string(&custom).unwrap();
        assert_eq!(json, "\"my_custom_capability\"");
    }
}
