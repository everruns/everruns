// Capability type definitions
//
// These are runtime types for the capability system.
// They are re-exported by everruns-contracts with ToSchema for API docs.

use serde::{Deserialize, Serialize};

/// Capability identifier
///
/// These are the internal IDs for built-in capabilities.
/// Re-exported by contracts with ToSchema for API documentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityId {
    /// No-op capability for testing/demo purposes
    Noop,
    /// CurrentTime capability - adds a tool to get the current time
    CurrentTime,
    /// Research capability (coming soon)
    Research,
    /// Sandbox capability (coming soon)
    Sandbox,
    /// FileSystem capability (coming soon)
    FileSystem,
    /// Math capability - adds calculator tools (add, subtract, multiply, divide)
    Math,
    /// Weather capability - adds weather tools (get_weather, get_forecast)
    Weather,
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CapabilityId::Noop => write!(f, "noop"),
            CapabilityId::CurrentTime => write!(f, "current_time"),
            CapabilityId::Research => write!(f, "research"),
            CapabilityId::Sandbox => write!(f, "sandbox"),
            CapabilityId::FileSystem => write!(f, "file_system"),
            CapabilityId::Math => write!(f, "math"),
            CapabilityId::Weather => write!(f, "weather"),
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
            "math" => Ok(CapabilityId::Math),
            "weather" => Ok(CapabilityId::Weather),
            _ => Err(format!("Unknown capability: {}", s)),
        }
    }
}

/// Capability status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        assert_eq!(CapabilityId::Math.to_string(), "math");
        assert_eq!(CapabilityId::Weather.to_string(), "weather");
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
        assert_eq!("math".parse::<CapabilityId>().unwrap(), CapabilityId::Math);
        assert_eq!(
            "weather".parse::<CapabilityId>().unwrap(),
            CapabilityId::Weather
        );
    }

    #[test]
    fn test_capability_id_from_str_unknown() {
        let result = "unknown".parse::<CapabilityId>();
        assert!(result.is_err());
    }
}
