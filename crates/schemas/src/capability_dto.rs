// Capability DTO types
//
// These types are API/DTO types for capabilities with ToSchema support.
// Runtime types (CapabilityId, CapabilityStatus) are in capability_types.rs.
// Note: from_core method is in runtime crate, not here.

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
        };

        let json = serde_json::to_string(&cap).unwrap();
        assert!(json.contains("\"id\":\"research\""));
        assert!(json.contains("\"status\":\"available\""));
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
}
