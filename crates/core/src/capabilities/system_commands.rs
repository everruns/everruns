// System Commands Capability
//
// Extension point for built-in /slash commands that execute directly
// without the LLM. Commands will be added here as they are implemented
// (e.g., /clear, /status, /compact).
//
// System commands are surfaced to the UI via the commands() trait method
// and executed via the commands API endpoint.

use super::{Capability, CapabilityStatus};

/// System commands capability ID
pub const SYSTEM_COMMANDS_CAPABILITY_ID: &str = "system_commands";

/// Built-in system commands capability.
///
/// Provides session-control commands that execute directly without
/// involving the LLM. These appear in the UI command palette.
/// Commands are added as their handlers are implemented.
pub struct SystemCommandsCapability;

impl Capability for SystemCommandsCapability {
    fn id(&self) -> &str {
        SYSTEM_COMMANDS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "System Commands"
    }

    fn description(&self) -> &str {
        "Built-in session control commands."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("terminal")
    }

    fn category(&self) -> Option<&str> {
        Some("System")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_commands_capability_metadata() {
        let cap = SystemCommandsCapability;
        assert_eq!(cap.id(), SYSTEM_COMMANDS_CAPABILITY_ID);
        assert_eq!(cap.name(), "System Commands");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("terminal"));
        assert_eq!(cap.category(), Some("System"));
    }

    #[test]
    fn test_system_commands_no_tools() {
        let cap = SystemCommandsCapability;
        assert!(cap.tools().is_empty());
        assert!(cap.tool_definitions().is_empty());
    }

    #[test]
    fn test_system_commands_no_system_prompt() {
        let cap = SystemCommandsCapability;
        assert!(cap.system_prompt_addition().is_none());
    }

    #[test]
    fn test_system_commands_empty_until_implemented() {
        let cap = SystemCommandsCapability;
        let commands = cap.commands();
        assert!(commands.is_empty());
    }
}
