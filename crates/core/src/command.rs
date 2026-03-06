// Custom Commands System
//
// Commands are user-invocable actions triggered via /slash syntax.
// Two sources:
// 1. System commands — from capabilities, execute directly (no LLM)
// 2. Skill commands — from skills with user-invocable: true, expand to prompt
//
// Skills marked user-invocable appear in the command palette alongside
// system commands. The UI fetches available commands and renders autocomplete.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Descriptor for a command available in a session
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CommandDescriptor {
    /// Command name (used as /name)
    pub name: String,
    /// Human-readable description shown in autocomplete
    pub description: String,
    /// Where this command comes from
    pub source: CommandSource,
    /// Arguments this command accepts
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<CommandArg>,
}

/// Where a command originates from
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    /// Built-in system command from a capability (no LLM, direct action)
    System,
    /// From a skill with user-invocable: true (expands to prompt, triggers LLM)
    Skill,
}

/// Argument descriptor for a command
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CommandArg {
    /// Argument name
    pub name: String,
    /// Description of the argument
    pub description: String,
    /// Whether the argument is required
    #[serde(default)]
    pub required: bool,
}

/// Result of executing a system command
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CommandResult {
    /// Whether the command succeeded
    pub success: bool,
    /// Human-readable message describing the result
    pub message: String,
}
