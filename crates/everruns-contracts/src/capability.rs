// Capability DTOs - defines agent capabilities that add functionality
//
// Design Decision: Capabilities are external to the Agent Loop.
// They are resolved at the service/API layer and contribute to AgentConfig
// (system prompt additions, tools, etc.). The Agent Loop remains focused on
// execution and receives a fully-configured AgentConfig.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Known capability identifiers
/// These are the internal IDs for built-in capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityId {
    /// No-op capability for testing/demo purposes
    Noop,
    /// CurrentTime capability - adds a tool to get the current time
    CurrentTime,
    /// Research capability - enables deep research with scratchpad and web tools
    Research,
    /// Sandbox capability - enables sandboxed code execution
    Sandbox,
    /// FileSystem capability - adds file system access tools
    FileSystem,
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilityId::Noop => write!(f, "noop"),
            CapabilityId::CurrentTime => write!(f, "current_time"),
            CapabilityId::Research => write!(f, "research"),
            CapabilityId::Sandbox => write!(f, "sandbox"),
            CapabilityId::FileSystem => write!(f, "file_system"),
        }
    }
}

impl std::str::FromStr for CapabilityId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "noop" => Ok(CapabilityId::Noop),
            "current_time" => Ok(CapabilityId::CurrentTime),
            "research" => Ok(CapabilityId::Research),
            "sandbox" => Ok(CapabilityId::Sandbox),
            "file_system" => Ok(CapabilityId::FileSystem),
            _ => Err(format!("Unknown capability: {}", s)),
        }
    }
}

/// Capability status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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

/// Public capability information (without internal details)
/// This is what gets returned from the API
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Capability {
    /// Unique capability identifier
    pub id: CapabilityId,
    /// Display name
    pub name: String,
    /// Description of what this capability provides
    pub description: String,
    /// Current status
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
    pub capability_id: CapabilityId,
    /// Position in the chain (lower = earlier)
    pub position: i32,
}

/// Request to update agent capabilities
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct UpdateAgentCapabilitiesRequest {
    /// List of capability IDs in desired order
    /// Position is determined by array index
    pub capabilities: Vec<CapabilityId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_id_display() {
        assert_eq!(CapabilityId::Noop.to_string(), "noop");
        assert_eq!(CapabilityId::CurrentTime.to_string(), "current_time");
        assert_eq!(CapabilityId::Research.to_string(), "research");
        assert_eq!(CapabilityId::Sandbox.to_string(), "sandbox");
        assert_eq!(CapabilityId::FileSystem.to_string(), "file_system");
    }

    #[test]
    fn test_capability_id_from_str() {
        assert_eq!("noop".parse::<CapabilityId>().unwrap(), CapabilityId::Noop);
        assert_eq!(
            "current_time".parse::<CapabilityId>().unwrap(),
            CapabilityId::CurrentTime
        );
        assert_eq!(
            "research".parse::<CapabilityId>().unwrap(),
            CapabilityId::Research
        );
        assert_eq!(
            "sandbox".parse::<CapabilityId>().unwrap(),
            CapabilityId::Sandbox
        );
        assert_eq!(
            "file_system".parse::<CapabilityId>().unwrap(),
            CapabilityId::FileSystem
        );
    }

    #[test]
    fn test_capability_id_from_str_unknown() {
        let result = "unknown".parse::<CapabilityId>();
        assert!(result.is_err());
    }

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
}
