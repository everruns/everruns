//! Curated, application-facing session context inspection.

use everruns_core::AssembledTurnContext;

use crate::{ContentPart, MessageRole};
use everruns_provider::model_spec::ModelSpec;

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
            model,
            messages,
            tools,
            instructions: context.runtime_agent.system_prompt,
            locale: context.resolved_locale,
            plugin_warnings,
        }
    }
}
