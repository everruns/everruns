// Custom Commands System
//
// Commands are user-invocable actions triggered via /slash syntax.
// Two sources:
// 1. System commands — from capabilities, execute through a dedicated handler
//    without persisting a chat message
// 2. Skill commands — from skills with user-invocable: true, expand to prompt
//
// Skills marked user-invocable appear in the command palette alongside
// system commands. The UI fetches available commands and renders autocomplete.

use crate::message::Controls;
use crate::typed_id::SessionId;
use crate::user_facing_error::UserFacingErrorFields;
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
    /// Built-in system command from a capability, executed out-of-band from the
    /// main chat history. Handlers may call the model or do direct work.
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
    /// Static list of suggested values for this argument. Captured when
    /// `Capability::commands()` is collected so renderers can surface
    /// autocomplete entries without round-tripping back to the capability
    /// on every keystroke. Empty means free-form input. Renderers should
    /// treat the list as suggestions, not constraints — the capability's
    /// `execute_command` is still the authority on what's accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<String>,
}

/// Request payload for executing a system command
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ExecuteCommandRequest {
    /// Command name without the leading slash
    pub name: String,
    /// Raw argument text after the command token
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    /// Optional per-invocation runtime controls
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
}

/// Context handed to [`crate::capabilities::Capability::execute_command`] when a
/// system command is dispatched. Carries only data that is safe to expose
/// across the trait surface; capabilities that need handles to runtime state
/// (provider store, file system, etc.) own those references directly via the
/// capability's constructor.
#[derive(Debug, Clone)]
pub struct CommandExecutionContext {
    /// Session the command is being executed against.
    pub session_id: SessionId,
}

/// Result of executing a system command
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CommandResult {
    /// Whether the command succeeded
    pub success: bool,
    /// Human-readable message describing the result
    pub message: String,
    /// Stable error code when `success` is false. Mirrors the codes emitted on
    /// chat error messages so the UI can localize the copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Optional structured fields associated with `error_code` (provider,
    /// model_id, retry_after, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub error_fields: Option<UserFacingErrorFields>,
}
