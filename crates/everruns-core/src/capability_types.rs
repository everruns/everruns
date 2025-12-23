// Capability type definitions
//
// Design Decision: Capability IDs are now String-based to allow adding new capabilities
// without requiring database migrations or code changes to enums.
// Validation happens at the registry level rather than the type level.

use serde::{Deserialize, Serialize};

/// Capability identifier - a string-based ID for extensibility
///
/// This allows new capabilities to be added without database changes.
/// The ID is validated at runtime against the capability registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CapabilityId(String);

impl CapabilityId {
    /// Create a new capability ID from a string
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Get the ID as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }

    // Built-in capability ID constants for convenience
    pub const NOOP: &'static str = "noop";
    pub const CURRENT_TIME: &'static str = "current_time";
    pub const RESEARCH: &'static str = "research";
    pub const SANDBOX: &'static str = "sandbox";
    pub const FILE_SYSTEM: &'static str = "file_system";
    pub const TEST_MATH: &'static str = "test_math";
    pub const TEST_WEATHER: &'static str = "test_weather";
    pub const STATELESS_TODO_LIST: &'static str = "stateless_todo_list";

    /// Create the noop capability ID
    pub fn noop() -> Self {
        Self::new(Self::NOOP)
    }

    /// Create the current_time capability ID
    pub fn current_time() -> Self {
        Self::new(Self::CURRENT_TIME)
    }

    /// Create the research capability ID
    pub fn research() -> Self {
        Self::new(Self::RESEARCH)
    }

    /// Create the sandbox capability ID
    pub fn sandbox() -> Self {
        Self::new(Self::SANDBOX)
    }

    /// Create the file_system capability ID
    pub fn file_system() -> Self {
        Self::new(Self::FILE_SYSTEM)
    }

    /// Create the test_math capability ID
    pub fn test_math() -> Self {
        Self::new(Self::TEST_MATH)
    }

    /// Create the test_weather capability ID
    pub fn test_weather() -> Self {
        Self::new(Self::TEST_WEATHER)
    }

    /// Create the stateless_todo_list capability ID
    pub fn stateless_todo_list() -> Self {
        Self::new(Self::STATELESS_TODO_LIST)
    }
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for CapabilityId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

impl From<&str> for CapabilityId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for CapabilityId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for CapabilityId {
    fn as_ref(&self) -> &str {
        &self.0
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
        assert_eq!(CapabilityId::noop().to_string(), "noop");
        assert_eq!(CapabilityId::current_time().to_string(), "current_time");
        assert_eq!(CapabilityId::research().to_string(), "research");
        assert_eq!(CapabilityId::sandbox().to_string(), "sandbox");
        assert_eq!(CapabilityId::file_system().to_string(), "file_system");
        assert_eq!(CapabilityId::test_math().to_string(), "test_math");
        assert_eq!(CapabilityId::test_weather().to_string(), "test_weather");
        assert_eq!(
            CapabilityId::stateless_todo_list().to_string(),
            "stateless_todo_list"
        );
    }

    #[test]
    fn test_capability_id_from_str() {
        assert_eq!(
            "noop".parse::<CapabilityId>().unwrap(),
            CapabilityId::noop()
        );
        assert_eq!(
            "current_time".parse::<CapabilityId>().unwrap(),
            CapabilityId::current_time()
        );
        assert_eq!(
            "research".parse::<CapabilityId>().unwrap(),
            CapabilityId::research()
        );
        assert_eq!(
            "sandbox".parse::<CapabilityId>().unwrap(),
            CapabilityId::sandbox()
        );
        assert_eq!(
            "file_system".parse::<CapabilityId>().unwrap(),
            CapabilityId::file_system()
        );
        assert_eq!(
            "test_math".parse::<CapabilityId>().unwrap(),
            CapabilityId::test_math()
        );
        assert_eq!(
            "test_weather".parse::<CapabilityId>().unwrap(),
            CapabilityId::test_weather()
        );
        assert_eq!(
            "stateless_todo_list".parse::<CapabilityId>().unwrap(),
            CapabilityId::stateless_todo_list()
        );
    }

    #[test]
    fn test_capability_id_from_custom_string() {
        // Custom capability IDs should work
        let custom = CapabilityId::new("my_custom_capability");
        assert_eq!(custom.to_string(), "my_custom_capability");
        assert_eq!(custom.as_str(), "my_custom_capability");
    }

    #[test]
    fn test_capability_id_serialization() {
        let id = CapabilityId::current_time();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"current_time\"");

        let parsed: CapabilityId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn test_capability_id_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(CapabilityId::noop());
        set.insert(CapabilityId::current_time());
        set.insert(CapabilityId::new("noop")); // Should not add a duplicate

        assert_eq!(set.len(), 2);
    }
}
