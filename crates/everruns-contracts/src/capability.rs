// Capability DTOs - defines agent capabilities that add functionality
//
// Runtime types (CapabilityId, CapabilityStatus) are defined in everruns-core
// and re-exported here. This file defines the API/DTO types with ToSchema.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// Re-export capability types from core
pub use everruns_core::capability_types::{CapabilityId, CapabilityStatus};

/// Public capability information (without internal details)
/// This is what gets returned from the API
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Capability {
    /// Unique capability identifier
    #[schema(value_type = String)]
    pub id: CapabilityId,
    /// Display name
    pub name: String,
    /// Description of what this capability provides
    pub description: String,
    /// Current status
    #[schema(value_type = String)]
    pub status: CapabilityStatus,
    /// Icon name (for UI rendering)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Category for grouping in UI
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// Agent capability assignment with ordering
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AgentCapability {
    /// The capability ID
    #[schema(value_type = String)]
    pub capability_id: CapabilityId,
    /// Position in the chain (lower = earlier)
    pub position: i32,
}

/// Request to update agent capabilities
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateAgentCapabilitiesRequest {
    /// List of capability IDs in desired order
    /// Position is determined by array index
    #[schema(value_type = Vec<String>)]
    pub capabilities: Vec<CapabilityId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_serialization() {
        let cap = Capability {
            id: CapabilityId::Research,
            name: "Research".to_string(),
            description: "Deep research capability".to_string(),
            status: CapabilityStatus::Available,
            icon: Some("search".to_string()),
            category: Some("AI".to_string()),
        };

        let json = serde_json::to_string(&cap).unwrap();
        assert!(json.contains("\"id\":\"research\""));
        assert!(json.contains("\"status\":\"available\""));
    }

    #[test]
    fn test_agent_capability_serialization() {
        let agent_cap = AgentCapability {
            capability_id: CapabilityId::Sandbox,
            position: 1,
        };

        let json = serde_json::to_string(&agent_cap).unwrap();
        assert!(json.contains("\"capability_id\":\"sandbox\""));
        assert!(json.contains("\"position\":1"));
    }

    #[test]
    fn test_test_capabilities() {
        // Verify test math and weather capabilities are available
        assert_eq!(CapabilityId::TestMath.to_string(), "test_math");
        assert_eq!(CapabilityId::TestWeather.to_string(), "test_weather");
        assert_eq!(
            "test_math".parse::<CapabilityId>().unwrap(),
            CapabilityId::TestMath
        );
        assert_eq!(
            "test_weather".parse::<CapabilityId>().unwrap(),
            CapabilityId::TestWeather
        );
    }
}
