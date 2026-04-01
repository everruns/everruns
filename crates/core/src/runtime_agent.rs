// Runtime agent configuration for the loop
//
// RuntimeAgent is a DB-agnostic configuration struct that can be:
// - Created directly for standalone usage
// - Built from a Harness and/or Agent entity via builder methods
//
// Prompt layering order (via prepend pattern):
// 1. with_harness() - sets harness system_prompt + harness caps
// 2. with_agent() - prepends agent prompt + agent caps (optional)
// 3. with_capabilities() - prepends session caps
// Result: [session_caps] [agent_caps] [agent_prompt] [harness_caps] [harness_prompt]

use crate::agent::Agent;
use crate::capabilities::{
    CapabilityRegistry, SystemPromptContext, collect_capabilities_with_configs,
    resolve_capability_configs,
};
use crate::harness::Harness;
use crate::llm_driver_registry::ToolSearchConfig;
use crate::llm_model_profiles::get_model_profile;
use crate::llm_models::LlmProviderType;
use crate::tool_types::ToolDefinition;
use serde::{Deserialize, Serialize};

/// Runtime configuration for the agent loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAgent {
    /// System prompt that defines the agent's behavior
    pub system_prompt: String,

    /// Model identifier (e.g., "gpt-5.2", "claude-3-opus")
    pub model: String,

    /// Available tools for the agent
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,

    /// Maximum number of tool-calling iterations (prevents infinite loops)
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,

    /// Temperature for LLM sampling (0.0 - 2.0)
    #[serde(default)]
    pub temperature: Option<f32>,

    /// Maximum tokens to generate per response
    #[serde(default)]
    pub max_tokens: Option<u32>,

    /// Tool search config (set by openai_tool_search capability)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_search: Option<ToolSearchConfig>,
}

/// Default maximum iterations per turn (500).
///
/// Resolution priority: session override > agent config > this default.
pub fn default_max_iterations() -> usize {
    500
}

impl RuntimeAgent {
    /// Create a new runtime agent configuration with required fields only
    pub fn new(system_prompt: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            model: model.into(),
            tools: Vec::new(),
            max_iterations: default_max_iterations(),
            temperature: None,
            max_tokens: None,
            tool_search: None,
        }
    }
}

impl Default for RuntimeAgent {
    fn default() -> Self {
        Self {
            system_prompt: "You are a helpful assistant.".to_string(),
            model: "gpt-5.2".to_string(),
            tools: Vec::new(),
            max_iterations: default_max_iterations(),
            temperature: None,
            max_tokens: None,
            tool_search: None,
        }
    }
}

/// Builder for RuntimeAgent with fluent API
///
/// Use `new()` to start building, then chain methods like `with_agent()`,
/// `model()`, `temperature()`, etc. Call `build()` to get the final runtime agent.
pub struct RuntimeAgentBuilder {
    runtime_agent: RuntimeAgent,
}

impl RuntimeAgentBuilder {
    /// Start building a new runtime agent from scratch
    pub fn new() -> Self {
        Self {
            runtime_agent: RuntimeAgent::default(),
        }
    }

    /// Apply a Harness's configuration to this builder.
    ///
    /// Sets the system prompt from the harness and applies harness capabilities.
    /// Calls `system_prompt_contribution()` on each capability for dynamic content.
    /// Call this BEFORE `with_agent()` to establish the base prompt layer.
    pub async fn with_harness(
        self,
        harness: &Harness,
        registry: &CapabilityRegistry,
        ctx: &SystemPromptContext,
    ) -> Self {
        self.system_prompt(&harness.system_prompt)
            .with_capability_configs(&harness.capabilities, registry, ctx)
            .await
    }

    /// Apply an Agent's configuration to this builder.
    ///
    /// Prepends the agent's system prompt and capabilities on top of the
    /// existing prompt (typically from a harness). Call after `with_harness()`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let ctx = SystemPromptContext::without_file_store(session_id);
    /// let runtime_agent = RuntimeAgentBuilder::new()
    ///     .with_harness(&harness, &registry, &ctx).await
    ///     .with_agent(&agent, &registry, &ctx).await
    ///     .with_capabilities(&session_caps, &registry, &ctx).await
    ///     .model("gpt-4o")
    ///     .build();
    /// ```
    pub async fn with_agent(
        self,
        agent: &Agent,
        registry: &CapabilityRegistry,
        ctx: &SystemPromptContext,
    ) -> Self {
        let mut builder = self
            .system_prompt(&agent.system_prompt)
            .with_capability_configs(&agent.capabilities, registry, ctx)
            .await;

        // Add agent-level client-side tools
        if !agent.tools.is_empty() {
            builder = builder.tools(agent.tools.clone());
        }

        builder
    }

    /// Apply capabilities to this builder.
    ///
    /// Resolves dependencies, then collects contributions from capabilities:
    /// - Dependencies are automatically included (topologically sorted)
    /// - `system_prompt_contribution(ctx)` called on each (may read from filesystem)
    /// - System prompt additions are prepended to the current system prompt
    /// - Tool definitions are added to the tools list
    ///
    /// # Arguments
    ///
    /// * `capability_ids` - Ordered list of capability IDs to apply
    /// * `registry` - The capability registry containing implementations
    /// * `ctx` - Session context for dynamic prompt resolution
    pub async fn with_capabilities(
        self,
        capability_ids: &[String],
        registry: &CapabilityRegistry,
        ctx: &SystemPromptContext,
    ) -> Self {
        let capability_configs: Vec<crate::AgentCapabilityConfig> = capability_ids
            .iter()
            .map(|id| crate::AgentCapabilityConfig::new(id.clone()))
            .collect();
        self.with_capability_configs(&capability_configs, registry, ctx)
            .await
    }

    /// Apply capability configs to this builder, preserving per-capability configuration.
    pub async fn with_capability_configs(
        mut self,
        capability_configs: &[crate::AgentCapabilityConfig],
        registry: &CapabilityRegistry,
        ctx: &SystemPromptContext,
    ) -> Self {
        let resolved_configs = match resolve_capability_configs(capability_configs, registry) {
            Ok(resolved) => resolved,
            Err(e) => {
                tracing::warn!("Failed to resolve capability dependencies: {}", e);
                capability_configs.to_vec()
            }
        };

        let collected = collect_capabilities_with_configs(&resolved_configs, registry, ctx).await;

        // Apply system prompt additions (prepend to existing, wrap base in XML tags)
        if let Some(prefix) = collected.system_prompt_prefix() {
            if !self.runtime_agent.system_prompt.is_empty()
                && !self.runtime_agent.system_prompt.contains("<system-prompt>")
            {
                self.runtime_agent.system_prompt = format!(
                    "<system-prompt>\n{}\n</system-prompt>",
                    self.runtime_agent.system_prompt
                );
            }
            self = self.prepend_system_prompt(prefix);
        }

        // Apply tool definitions
        if !collected.tool_definitions.is_empty() {
            self = self.tools(collected.tool_definitions);
        }

        // Apply tool_search config if capability provided one
        if let Some(ts_config) = collected.tool_search {
            self.runtime_agent.tool_search = Some(ts_config);
        }

        self
    }

    /// Set the system prompt
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.runtime_agent.system_prompt = prompt.into();
        self
    }

    /// Prepend text to the system prompt
    pub fn prepend_system_prompt(mut self, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into();
        if !prefix.is_empty() {
            self.runtime_agent.system_prompt =
                format!("{}\n\n{}", prefix, self.runtime_agent.system_prompt);
        }
        self
    }

    /// Prepend locale instructions for session-aware localization.
    pub fn with_locale(self, locale: Option<&str>) -> Self {
        let Some(locale) = locale.map(str::trim).filter(|value| !value.is_empty()) else {
            return self;
        };

        self.prepend_system_prompt(format!(
            "<locale preference=\"{locale}\">\n\
             Default locale for this session: {locale}.\n\
             Unless the user explicitly asks otherwise, respond in this locale and use its language, spelling, and regional formatting conventions for dates, times, numbers, and currency.\n\
             </locale>"
        ))
    }

    /// Set the model
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.runtime_agent.model = model.into();
        self
    }

    /// Add a tool
    pub fn tool(mut self, tool: ToolDefinition) -> Self {
        self.runtime_agent.tools.push(tool);
        self
    }

    /// Add multiple tools
    pub fn tools(mut self, tools: impl IntoIterator<Item = ToolDefinition>) -> Self {
        self.runtime_agent.tools.extend(tools);
        self
    }

    /// Set maximum iterations
    pub fn max_iterations(mut self, max: usize) -> Self {
        self.runtime_agent.max_iterations = max;
        self
    }

    /// Set temperature
    pub fn temperature(mut self, temp: f32) -> Self {
        self.runtime_agent.temperature = Some(temp);
        self
    }

    /// Set max tokens
    pub fn max_tokens(mut self, tokens: u32) -> Self {
        self.runtime_agent.max_tokens = Some(tokens);
        self
    }

    /// Set tool_search configuration
    pub fn tool_search(mut self, config: ToolSearchConfig) -> Self {
        self.runtime_agent.tool_search = Some(config);
        self
    }

    /// Build the runtime agent.
    ///
    /// Validates that tool_search is only enabled for models that support it
    /// (currently GPT-5.4+). Clears tool_search for unsupported models to
    /// prevent 400 errors from the OpenAI API.
    ///
    /// tool_search is capability-driven: it is only set when the
    /// `openai_tool_search` capability is explicitly added to the agent
    /// or harness. This method does NOT auto-enable it.
    pub fn build(mut self) -> RuntimeAgent {
        // Deduplicate tools by name (last wins). Tools are collected additively
        // from harness, agent, MCP servers, session capabilities, and client-side
        // tools — duplicates can occur when the same tool is registered by
        // multiple sources.
        {
            let mut seen = std::collections::HashSet::new();
            let mut deduped = Vec::with_capacity(self.runtime_agent.tools.len());
            // Iterate in reverse so the last-added tool wins, then reverse back.
            for tool in self.runtime_agent.tools.drain(..).rev() {
                if seen.insert(tool.name().to_owned()) {
                    deduped.push(tool);
                }
            }
            deduped.reverse();
            self.runtime_agent.tools = deduped;
        }

        if self.runtime_agent.tool_search.is_some() {
            let model_supports =
                get_model_profile(&LlmProviderType::Openai, &self.runtime_agent.model)
                    .is_some_and(|p| p.tool_search);
            if !model_supports {
                tracing::debug!(
                    model = %self.runtime_agent.model,
                    "tool_search capability configured but model does not support it; disabling"
                );
                self.runtime_agent.tool_search = None;
            }
        }
        self.runtime_agent
    }
}

impl Default for RuntimeAgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentStatus;
    use crate::capabilities::{AgentCapabilityConfig, SystemPromptContext};
    use crate::typed_id::AgentId;

    fn test_ctx() -> SystemPromptContext {
        SystemPromptContext::without_file_store(crate::typed_id::SessionId::new())
    }

    #[test]
    fn test_runtime_agent_new() {
        let runtime_agent = RuntimeAgent::new("You are helpful.", "gpt-5.2");

        assert_eq!(runtime_agent.system_prompt, "You are helpful.");
        assert_eq!(runtime_agent.model, "gpt-5.2");
        assert!(runtime_agent.tools.is_empty());
        assert_eq!(runtime_agent.max_iterations, 100);
        assert!(runtime_agent.temperature.is_none());
        assert!(runtime_agent.max_tokens.is_none());
    }

    #[test]
    fn test_runtime_agent_default() {
        let runtime_agent = RuntimeAgent::default();

        assert_eq!(runtime_agent.system_prompt, "You are a helpful assistant.");
        assert_eq!(runtime_agent.model, "gpt-5.2");
        assert!(runtime_agent.tools.is_empty());
        assert_eq!(runtime_agent.max_iterations, 100);
    }

    #[test]
    fn test_builder_basic() {
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Custom prompt")
            .model("claude-3-opus")
            .build();

        assert_eq!(runtime_agent.system_prompt, "Custom prompt");
        assert_eq!(runtime_agent.model, "claude-3-opus");
    }

    #[test]
    fn test_builder_with_all_options() {
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("You are a coder.")
            .model("gpt-5.2")
            .max_iterations(20)
            .temperature(0.7)
            .max_tokens(4096)
            .build();

        assert_eq!(runtime_agent.system_prompt, "You are a coder.");
        assert_eq!(runtime_agent.model, "gpt-5.2");
        assert_eq!(runtime_agent.max_iterations, 20);
        assert_eq!(runtime_agent.temperature, Some(0.7));
        assert_eq!(runtime_agent.max_tokens, Some(4096));
    }

    #[test]
    fn test_builder_prepend_system_prompt() {
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .prepend_system_prompt("Prefix text.")
            .build();

        assert_eq!(runtime_agent.system_prompt, "Prefix text.\n\nBase prompt.");
    }

    #[test]
    fn test_builder_prepend_empty_string_does_nothing() {
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .prepend_system_prompt("")
            .build();

        assert_eq!(runtime_agent.system_prompt, "Base prompt.");
    }

    #[test]
    fn test_builder_with_locale_prepends_locale_instructions() {
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_locale(Some("uk-UA"))
            .build();

        assert!(runtime_agent.system_prompt.contains("<locale"));
        assert!(runtime_agent.system_prompt.contains("uk-UA"));
        assert!(runtime_agent.system_prompt.contains("Base prompt."));
    }

    #[tokio::test]
    async fn test_builder_with_capabilities_empty() {
        let registry = CapabilityRegistry::with_builtins();
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&[], &registry, &test_ctx())
            .await
            .build();

        assert_eq!(runtime_agent.system_prompt, "Base prompt.");
        assert!(runtime_agent.tools.is_empty());
    }

    #[tokio::test]
    async fn test_builder_with_capabilities_adds_tools() {
        use crate::tool_types::ToolDefinition;

        let registry = CapabilityRegistry::with_builtins();
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["current_time".to_string()], &registry, &test_ctx())
            .await
            .build();

        assert_eq!(runtime_agent.tools.len(), 1);
        match &runtime_agent.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "get_current_time");
            }
            _ => panic!("expected Builtin variant"),
        }
    }

    #[tokio::test]
    async fn test_builder_with_capabilities_prepends_system_prompt() {
        let registry = CapabilityRegistry::with_builtins();
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["session_file_system".to_string()], &registry, &test_ctx())
            .await
            .build();

        assert!(runtime_agent.system_prompt.contains("/workspace"));
        // Base prompt wrapped in <system-prompt> tags
        assert!(runtime_agent.system_prompt.contains("<system-prompt>"));
        assert!(
            runtime_agent
                .system_prompt
                .ends_with("<system-prompt>\nBase prompt.\n</system-prompt>")
        );
    }

    #[tokio::test]
    async fn test_builder_with_agent() {
        use crate::tool_types::ToolDefinition;
        use uuid::{NoContext, Timestamp, Uuid};

        let registry = CapabilityRegistry::with_builtins();
        let ts = Timestamp::now(NoContext);
        let uuid = Uuid::new_v7(ts);
        let agent = Agent {
            public_id: AgentId::from_uuid(uuid),
            internal_id: uuid,
            name: "Test Agent".to_string(),
            description: None,
            system_prompt: "Agent prompt.".to_string(),
            capabilities: vec![AgentCapabilityConfig::new("current_time")],
            initial_files: vec![],
            max_iterations: None,
            tools: vec![],
            status: AgentStatus::Active,
            default_model_id: None,
            tags: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        };

        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &registry, &test_ctx())
            .await
            .model("gpt-5.2")
            .build();

        assert!(runtime_agent.system_prompt.contains("Agent prompt."));
        assert_eq!(runtime_agent.tools.len(), 1);
        match &runtime_agent.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "get_current_time");
            }
            _ => panic!("expected Builtin variant"),
        }
    }

    #[test]
    fn test_builder_default() {
        let builder = RuntimeAgentBuilder::default();
        let runtime_agent = builder.build();

        assert_eq!(runtime_agent.system_prompt, "You are a helpful assistant.");
        assert_eq!(runtime_agent.model, "gpt-5.2");
    }

    #[tokio::test]
    async fn test_builder_with_capabilities_resolves_dependencies() {
        // Sample Data depends on Session File System
        // When we request only Sample Data, we should get system prompt from both
        let registry = CapabilityRegistry::with_builtins();
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["sample_data".to_string()], &registry, &test_ctx())
            .await
            .build();

        // System prompt should include File System's contribution (the dependency) in XML tags
        assert!(
            runtime_agent
                .system_prompt
                .contains("<capability id=\"session_file_system\">"),
            "Should include File System capability in XML tags"
        );
        assert!(
            runtime_agent.system_prompt.contains("/workspace"),
            "Should include File System system prompt (mentions workspace root)"
        );
        // Should also include Sample Data's contribution in XML tags
        assert!(
            runtime_agent
                .system_prompt
                .contains("<capability id=\"sample_data\">"),
            "Should include Sample Data capability in XML tags"
        );
        assert!(
            runtime_agent.system_prompt.contains("/samples"),
            "Should include Sample Data system prompt (mentions /samples path)"
        );
        // Base prompt should still be there, wrapped
        assert!(
            runtime_agent.system_prompt.contains("Base prompt."),
            "Should preserve base prompt"
        );
        assert!(
            runtime_agent.system_prompt.contains("<system-prompt>"),
            "Base prompt should be wrapped in system-prompt tags"
        );
    }

    #[tokio::test]
    async fn test_builder_additive_capabilities() {
        use crate::tool_types::ToolDefinition;

        let registry = CapabilityRegistry::with_builtins();

        // Apply capabilities additively (simulating session-level capabilities)
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Agent prompt.")
            .with_capabilities(&["current_time".to_string()], &registry, &test_ctx())
            .await
            .build();

        // Should have the tool from capability
        assert_eq!(runtime_agent.tools.len(), 1);
        match &runtime_agent.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "get_current_time");
            }
            _ => panic!("expected Builtin variant"),
        }
    }

    #[tokio::test]
    async fn test_builder_with_agent_client_side_tools() {
        use crate::tool_types::{ClientSideTool, DeferrablePolicy, ToolDefinition};
        use uuid::{NoContext, Timestamp, Uuid};

        let registry = CapabilityRegistry::with_builtins();
        let ts = Timestamp::now(NoContext);
        let uuid = Uuid::new_v7(ts);

        let client_tool = ToolDefinition::ClientSide(ClientSideTool {
            name: "browser_click".to_string(),
            display_name: None,
            description: "Click an element in the browser".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "selector": {"type": "string"}
                }
            }),
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: crate::tool_types::ToolHints::default(),
        });

        let agent = Agent {
            public_id: AgentId::from_uuid(uuid),
            internal_id: uuid,
            name: "Client Tool Agent".to_string(),
            description: None,
            system_prompt: "Agent with client tools.".to_string(),
            capabilities: vec![],
            initial_files: vec![],
            max_iterations: None,
            tools: vec![client_tool],
            status: AgentStatus::Active,
            default_model_id: None,
            tags: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        };

        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &registry, &test_ctx())
            .await
            .model("gpt-5.2")
            .build();

        assert_eq!(runtime_agent.tools.len(), 1);
        assert_eq!(runtime_agent.tools[0].name(), "browser_click");
        assert_eq!(
            runtime_agent.tools[0].policy(),
            &crate::tool_types::ToolPolicy::ClientSide
        );
    }

    #[tokio::test]
    async fn test_builder_with_agent_client_side_and_capabilities() {
        use crate::tool_types::{ClientSideTool, DeferrablePolicy, ToolDefinition};
        use uuid::{NoContext, Timestamp, Uuid};

        let registry = CapabilityRegistry::with_builtins();
        let ts = Timestamp::now(NoContext);
        let uuid = Uuid::new_v7(ts);

        let client_tool = ToolDefinition::ClientSide(ClientSideTool {
            name: "deploy_staging".to_string(),
            display_name: None,
            description: "Deploy to staging".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: crate::tool_types::ToolHints::default(),
        });

        let agent = Agent {
            public_id: AgentId::from_uuid(uuid),
            internal_id: uuid,
            name: "Mixed Tool Agent".to_string(),
            description: None,
            system_prompt: "Agent with mixed tools.".to_string(),
            capabilities: vec![AgentCapabilityConfig::new("current_time")],
            initial_files: vec![],
            max_iterations: None,
            tools: vec![client_tool],
            status: AgentStatus::Active,
            default_model_id: None,
            tags: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        };

        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &registry, &test_ctx())
            .await
            .model("gpt-5.2")
            .build();

        // Should have capability tool + client-side tool
        assert_eq!(runtime_agent.tools.len(), 2);
        let tool_names: Vec<&str> = runtime_agent.tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"get_current_time"));
        assert!(tool_names.contains(&"deploy_staging"));

        // Verify the client tool is ClientSide variant
        let deploy_tool = runtime_agent
            .tools
            .iter()
            .find(|t| t.name() == "deploy_staging")
            .unwrap();
        assert!(matches!(deploy_tool, ToolDefinition::ClientSide(_)));
    }

    #[tokio::test]
    async fn test_builder_with_agent_and_additive_capabilities() {
        use uuid::{NoContext, Timestamp, Uuid};

        let registry = CapabilityRegistry::with_builtins();
        let ts = Timestamp::now(NoContext);

        // Agent has current_time capability (no system prompt addition)
        let uuid = Uuid::new_v7(ts);
        let agent = Agent {
            public_id: AgentId::from_uuid(uuid),
            internal_id: uuid,
            name: "Test Agent".to_string(),
            description: None,
            system_prompt: "Agent prompt.".to_string(),
            capabilities: vec![AgentCapabilityConfig::new("current_time")],
            initial_files: vec![],
            max_iterations: None,
            tools: vec![],
            status: AgentStatus::Active,
            default_model_id: None,
            tags: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        };

        // Session adds stateless_todo_list capability (additive — has system prompt addition)
        let session_capability_ids = vec!["stateless_todo_list".to_string()];

        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &registry, &test_ctx())
            .await
            .with_capabilities(&session_capability_ids, &registry, &test_ctx())
            .await
            .model("gpt-5.2")
            .build();

        // Should have tools from both agent and session capabilities
        assert!(runtime_agent.tools.len() >= 2);
        let tool_names: Vec<&str> = runtime_agent.tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"get_current_time"));
        assert!(tool_names.contains(&"write_todos"));

        // System prompt should contain both capability additions and agent prompt
        assert!(runtime_agent.system_prompt.contains("Agent prompt."));
        assert!(runtime_agent.system_prompt.contains("Task Management"));
        assert!(
            runtime_agent
                .system_prompt
                .contains("<capability id=\"stateless_todo_list\">")
        );
        // Base prompt should be wrapped in <system-prompt> tags (no double wrapping)
        let system_prompt_count = runtime_agent
            .system_prompt
            .matches("<system-prompt>")
            .count();
        assert_eq!(
            system_prompt_count, 1,
            "Should have exactly one <system-prompt> tag, not double-wrapped"
        );
    }

    #[test]
    fn test_build_clears_tool_search_for_unsupported_model() {
        let agent = RuntimeAgentBuilder::new()
            .model("gpt-5.2")
            .tool_search(ToolSearchConfig {
                enabled: true,
                threshold: 15,
            })
            .build();

        assert!(
            agent.tool_search.is_none(),
            "tool_search should be cleared for gpt-5.2 (unsupported)"
        );
    }

    #[test]
    fn test_build_keeps_tool_search_for_supported_model() {
        let agent = RuntimeAgentBuilder::new()
            .model("gpt-5.4")
            .tool_search(ToolSearchConfig {
                enabled: true,
                threshold: 15,
            })
            .build();

        assert!(
            agent.tool_search.is_some(),
            "tool_search should be kept for gpt-5.4 (supported)"
        );
    }

    #[test]
    fn test_build_clears_tool_search_for_non_openai_model() {
        let agent = RuntimeAgentBuilder::new()
            .model("claude-sonnet-4-5-20250514")
            .tool_search(ToolSearchConfig {
                enabled: true,
                threshold: 15,
            })
            .build();

        assert!(
            agent.tool_search.is_none(),
            "tool_search should be cleared for non-OpenAI models"
        );
    }

    #[test]
    fn test_build_no_auto_enable_tool_search_without_capability() {
        // tool_search requires explicit openai_tool_search capability.
        // Even GPT-5.4 (which supports it) should not get it automatically.
        let agent = RuntimeAgentBuilder::new().model("gpt-5.4").build();

        assert!(
            agent.tool_search.is_none(),
            "tool_search must not be auto-enabled; it is capability-driven"
        );
    }

    #[test]
    fn test_build_preserves_explicit_tool_search_config_for_supported_model() {
        // Simulates Generic harness setting openai_tool_search capability
        // with custom threshold — build() must preserve it.
        let agent = RuntimeAgentBuilder::new()
            .model("gpt-5.4")
            .tool_search(ToolSearchConfig {
                enabled: true,
                threshold: 5,
            })
            .build();

        let ts = agent
            .tool_search
            .expect("explicit tool_search should be preserved");
        assert!(ts.enabled);
        assert_eq!(
            ts.threshold, 5,
            "custom threshold from capability must be preserved"
        );
    }

    #[test]
    fn test_build_deduplicates_tools_by_name() {
        use crate::tool_types::{BuiltinTool, ToolDefinition, ToolPolicy};

        let make_tool = |name: &str, desc: &str| {
            ToolDefinition::Builtin(BuiltinTool {
                name: name.to_string(),
                display_name: None,
                description: desc.to_string(),
                parameters: serde_json::json!({}),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: Default::default(),
                hints: crate::tool_types::ToolHints::default(),
            })
        };

        let agent = RuntimeAgentBuilder::new()
            .tool(make_tool("kv_store", "first"))
            .tool(make_tool("browser", "only one"))
            .tool(make_tool("kv_store", "second (should win)"))
            .build();

        assert_eq!(agent.tools.len(), 2);
        // Last-added kv_store wins
        assert_eq!(agent.tools[0].name(), "browser");
        assert_eq!(agent.tools[1].name(), "kv_store");
        assert_eq!(agent.tools[1].description(), "second (should win)");
    }
}
