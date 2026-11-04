//! Curated, application-facing session context inspection.

use everruns_core::AssembledTurnContext;

use crate::{ContentPart, MessageRole};
use everruns_contracts::model_spec::ModelSpec;

/// The effective context a session will send to its model on the next turn.
///
/// This intentionally exposes values useful to applications while keeping
/// stored harness/session records and backend-specific context internals out of
/// the Framework API.
#[derive(Debug, Clone)]
pub struct SessionContext {
    /// Credential-free provider and model selection.
    pub model: ModelSpec,
    /// Filtered conversation history visible to the next model call.
    pub messages: Vec<ContextMessage>,
    /// Model-visible tools after capability, plugin, and MCP resolution.
    pub tools: Vec<ToolInfo>,
    /// Effective system instructions after all configured contributions.
    pub instructions: String,
    /// Effective locale, when one was configured.
    pub locale: Option<String>,
    /// Non-fatal warnings produced while loading local plugins.
    pub plugin_warnings: Vec<String>,
    /// Slash commands contributed by the session's active capabilities, in
    /// capability resolution order and deduplicated by name.
    pub commands: Vec<CommandInfo>,
    /// Effective host metadata the next provider request carries, as
    /// assembled for it (see
    /// [`HostServices::request_metadata`](crate::hosting::services::HostServices::request_metadata)).
    /// The runtime adds its own correlation ids (such as `session_id`) per
    /// request; they are not listed here.
    pub request_metadata: std::collections::BTreeMap<String, String>,
}

/// A slash command contributed by an active capability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandInfo {
    /// Command name, invoked as `/name`.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Arguments the command accepts, with static completion suggestions.
    pub arguments: Vec<CommandArgument>,
}

/// An argument of a [`CommandInfo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandArgument {
    /// Argument name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Whether the command requires the argument.
    pub required: bool,
    /// Suggested values for completion; the command still decides what it
    /// accepts.
    pub suggestions: Vec<String>,
}

impl CommandInfo {
    pub(crate) fn from_descriptor(command: everruns_core::command::CommandDescriptor) -> Self {
        Self {
            name: command.name,
            description: command.description,
            arguments: command
                .args
                .into_iter()
                .map(|argument| CommandArgument {
                    name: argument.name,
                    description: argument.description,
                    required: argument.required,
                    suggestions: argument.suggestions,
                })
                .collect(),
        }
    }
}

/// One message in an inspected session context.
#[derive(Debug, Clone)]
pub struct ContextMessage {
    /// The message's conversational role.
    pub role: MessageRole,
    /// Structured text, image, tool-call, and tool-result content.
    pub content: Vec<ContentPart>,
}

/// A model-visible tool in an inspected session context.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    /// Model-facing tool name.
    pub name: String,
    /// Model-facing description.
    pub description: String,
    /// JSON Schema for the tool arguments.
    pub parameters: serde_json::Value,
}

impl SessionContext {
    pub(crate) fn from_runtime(
        context: AssembledTurnContext,
        plugin_warnings: Vec<String>,
        commands: Vec<everruns_core::command::CommandDescriptor>,
    ) -> Self {
        let model = ModelSpec::on(context.model.provider.clone(), context.model.model.clone());
        let messages = context
            .messages
            .into_iter()
            .map(|message| ContextMessage {
                role: message.role,
                content: message.content,
            })
            .collect();
        let tools = context
            .runtime_agent
            .tools
            .iter()
            .map(|tool| ToolInfo {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                parameters: tool.parameters().clone(),
            })
            .collect();

        Self {
            request_metadata: context.embedder_metadata,
            model,
            messages,
            tools,
            instructions: context.runtime_agent.system_prompt,
            locale: context.resolved_locale,
            plugin_warnings,
            commands: commands
                .into_iter()
                .map(CommandInfo::from_descriptor)
                .collect(),
        }
    }
}
