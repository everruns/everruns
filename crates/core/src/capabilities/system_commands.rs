// System Commands Capability
//
// Provides built-in /slash commands that execute directly without the LLM.
// These are session-control commands like /clear, /status, /compact.
//
// System commands are surfaced to the UI via the commands() trait method
// and executed via the commands API endpoint.

use super::{Capability, CapabilityStatus};
use crate::command::{CommandArg, CommandDescriptor, CommandSource};

/// System commands capability ID
pub const SYSTEM_COMMANDS_CAPABILITY_ID: &str = "system_commands";

/// Built-in system commands capability.
///
/// Provides session-control commands that execute directly without
/// involving the LLM. These appear in the UI command palette.
pub struct SystemCommandsCapability;

impl Capability for SystemCommandsCapability {
    fn id(&self) -> &str {
        SYSTEM_COMMANDS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "System Commands"
    }

    fn description(&self) -> &str {
        "Built-in session control commands like /clear and /status."
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

    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![
            CommandDescriptor {
                name: "clear".to_string(),
                description: "Clear conversation history and start fresh".to_string(),
                source: CommandSource::System,
                args: vec![],
            },
            CommandDescriptor {
                name: "status".to_string(),
                description: "Show session status, active model, and token usage".to_string(),
                source: CommandSource::System,
                args: vec![],
            },
            CommandDescriptor {
                name: "compact".to_string(),
                description: "Summarize conversation to free context space".to_string(),
                source: CommandSource::System,
                args: vec![],
            },
            CommandDescriptor {
                name: "model".to_string(),
                description: "Switch the active LLM model for this session".to_string(),
                source: CommandSource::System,
                args: vec![CommandArg {
                    name: "model_id".to_string(),
                    description: "Model identifier to switch to".to_string(),
                    required: false,
                }],
            },
        ]
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
    fn test_system_commands_provides_commands() {
        let cap = SystemCommandsCapability;
        let commands = cap.commands();
        assert!(!commands.is_empty());

        let names: Vec<&str> = commands.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"clear"));
        assert!(names.contains(&"status"));
        assert!(names.contains(&"compact"));
        assert!(names.contains(&"model"));

        // All system commands
        for cmd in &commands {
            assert_eq!(cmd.source, CommandSource::System);
        }
    }

    #[test]
    fn test_model_command_has_args() {
        let cap = SystemCommandsCapability;
        let commands = cap.commands();
        let model_cmd = commands.iter().find(|c| c.name == "model").unwrap();
        assert_eq!(model_cmd.args.len(), 1);
        assert_eq!(model_cmd.args[0].name, "model_id");
        assert!(!model_cmd.args[0].required);
    }
}
