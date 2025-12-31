// Agent configuration for the loop
//
// AgentConfig is a DB-agnostic configuration struct that can be:
// - Created directly for standalone usage
// - Built from an Agent entity via the `with_agent` builder method

use crate::agent::Agent;
use crate::capabilities::{collect_capabilities, CapabilityRegistry};
use crate::tool_types::ToolDefinition;
use serde::{Deserialize, Serialize};

/// Configuration for the agent loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
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
}

fn default_max_iterations() -> usize {
    10
}

impl AgentConfig {
    /// Create a new agent configuration with required fields only
    pub fn new(system_prompt: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            model: model.into(),
            tools: Vec::new(),
            max_iterations: default_max_iterations(),
            temperature: None,
            max_tokens: None,
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            system_prompt: "You are a helpful assistant.".to_string(),
            model: "gpt-5.2".to_string(),
            tools: Vec::new(),
            max_iterations: default_max_iterations(),
            temperature: None,
            max_tokens: None,
        }
    }
}

/// Builder for AgentConfig with fluent API
///
/// Use `new()` to start building, then chain methods like `with_agent()`,
/// `model()`, `temperature()`, etc. Call `build()` to get the final config.
pub struct AgentConfigBuilder {
    config: AgentConfig,
}

impl AgentConfigBuilder {
    /// Start building a new configuration from scratch
    pub fn new() -> Self {
        Self {
            config: AgentConfig::default(),
        }
    }

    /// Apply an Agent's configuration to this builder.
    ///
    /// This sets the system prompt from the agent and applies the agent's
    /// capabilities (tools and system prompt additions).
    ///
    /// # Arguments
    ///
    /// * `agent` - The Agent entity to apply
    /// * `registry` - The capability registry containing capability implementations
    ///
    /// # Example
    ///
    /// ```ignore
    /// use everruns_core::config::AgentConfigBuilder;
    /// use everruns_core::capabilities::CapabilityRegistry;
    ///
    /// let registry = CapabilityRegistry::with_builtins();
    /// let config = AgentConfigBuilder::new()
    ///     .with_agent(&agent, &registry)
    ///     .model("gpt-4o")
    ///     .temperature(0.7)
    ///     .build();
    /// ```
    pub fn with_agent(self, agent: &Agent, registry: &CapabilityRegistry) -> Self {
        let capability_ids: Vec<String> = agent
            .capabilities
            .iter()
            .map(|cap_id| cap_id.as_str().to_string())
            .collect();

        self.system_prompt(&agent.system_prompt)
            .with_capabilities(&capability_ids, registry)
    }

    /// Apply capabilities to this builder.
    ///
    /// This collects contributions from the given capabilities and applies them:
    /// - System prompt additions are prepended to the current system prompt
    /// - Tool definitions are added to the tools list
    ///
    /// # Arguments
    ///
    /// * `capability_ids` - Ordered list of capability IDs to apply
    /// * `registry` - The capability registry containing implementations
    pub fn with_capabilities(
        mut self,
        capability_ids: &[String],
        registry: &CapabilityRegistry,
    ) -> Self {
        let collected = collect_capabilities(capability_ids, registry);

        // Apply system prompt additions (prepend to existing)
        if let Some(prefix) = collected.system_prompt_prefix() {
            self = self.prepend_system_prompt(prefix);
        }

        // Apply tool definitions
        if !collected.tool_definitions.is_empty() {
            self = self.tools(collected.tool_definitions);
        }

        self
    }

    /// Set the system prompt
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.config.system_prompt = prompt.into();
        self
    }

    /// Prepend text to the system prompt
    pub fn prepend_system_prompt(mut self, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into();
        if !prefix.is_empty() {
            self.config.system_prompt = format!("{}\n\n{}", prefix, self.config.system_prompt);
        }
        self
    }

    /// Set the model
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.config.model = model.into();
        self
    }

    /// Add a tool
    pub fn tool(mut self, tool: ToolDefinition) -> Self {
        self.config.tools.push(tool);
        self
    }

    /// Add multiple tools
    pub fn tools(mut self, tools: impl IntoIterator<Item = ToolDefinition>) -> Self {
        self.config.tools.extend(tools);
        self
    }

    /// Set maximum iterations
    pub fn max_iterations(mut self, max: usize) -> Self {
        self.config.max_iterations = max;
        self
    }

    /// Set temperature
    pub fn temperature(mut self, temp: f32) -> Self {
        self.config.temperature = Some(temp);
        self
    }

    /// Set max tokens
    pub fn max_tokens(mut self, tokens: u32) -> Self {
        self.config.max_tokens = Some(tokens);
        self
    }

    /// Build the configuration
    pub fn build(self) -> AgentConfig {
        self.config
    }
}

impl Default for AgentConfigBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::AgentStatus;
    use crate::capabilities::CapabilityId;
    use crate::capability_types::CapabilityId as CapabilityIdType;

    #[test]
    fn test_agent_config_new() {
        let config = AgentConfig::new("You are helpful.", "gpt-5.2");

        assert_eq!(config.system_prompt, "You are helpful.");
        assert_eq!(config.model, "gpt-5.2");
        assert!(config.tools.is_empty());
        assert_eq!(config.max_iterations, 10);
        assert!(config.temperature.is_none());
        assert!(config.max_tokens.is_none());
    }

    #[test]
    fn test_agent_config_default() {
        let config = AgentConfig::default();

        assert_eq!(config.system_prompt, "You are a helpful assistant.");
        assert_eq!(config.model, "gpt-5.2");
        assert!(config.tools.is_empty());
        assert_eq!(config.max_iterations, 10);
    }

    #[test]
    fn test_builder_basic() {
        let config = AgentConfigBuilder::new()
            .system_prompt("Custom prompt")
            .model("claude-3-opus")
            .build();

        assert_eq!(config.system_prompt, "Custom prompt");
        assert_eq!(config.model, "claude-3-opus");
    }

    #[test]
    fn test_builder_with_all_options() {
        let config = AgentConfigBuilder::new()
            .system_prompt("You are a coder.")
            .model("gpt-5.2")
            .max_iterations(20)
            .temperature(0.7)
            .max_tokens(4096)
            .build();

        assert_eq!(config.system_prompt, "You are a coder.");
        assert_eq!(config.model, "gpt-5.2");
        assert_eq!(config.max_iterations, 20);
        assert_eq!(config.temperature, Some(0.7));
        assert_eq!(config.max_tokens, Some(4096));
    }

    #[test]
    fn test_builder_prepend_system_prompt() {
        let config = AgentConfigBuilder::new()
            .system_prompt("Base prompt.")
            .prepend_system_prompt("Prefix text.")
            .build();

        assert_eq!(config.system_prompt, "Prefix text.\n\nBase prompt.");
    }

    #[test]
    fn test_builder_prepend_empty_string_does_nothing() {
        let config = AgentConfigBuilder::new()
            .system_prompt("Base prompt.")
            .prepend_system_prompt("")
            .build();

        assert_eq!(config.system_prompt, "Base prompt.");
    }

    #[test]
    fn test_builder_with_capabilities_empty() {
        let registry = CapabilityRegistry::with_builtins();
        let config = AgentConfigBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&[], &registry)
            .build();

        assert_eq!(config.system_prompt, "Base prompt.");
        assert!(config.tools.is_empty());
    }

    #[test]
    fn test_builder_with_capabilities_adds_tools() {
        use crate::tool_types::ToolDefinition;

        let registry = CapabilityRegistry::with_builtins();
        let config = AgentConfigBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&[CapabilityId::CURRENT_TIME.to_string()], &registry)
            .build();

        assert_eq!(config.tools.len(), 1);
        match &config.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "get_current_time");
            }
        }
    }

    #[test]
    fn test_builder_with_capabilities_prepends_system_prompt() {
        let registry = CapabilityRegistry::with_builtins();
        let config = AgentConfigBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&[CapabilityId::TEST_MATH.to_string()], &registry)
            .build();

        assert!(config.system_prompt.contains("math tools"));
        assert!(config.system_prompt.ends_with("Base prompt."));
    }

    #[test]
    fn test_builder_with_agent() {
        use crate::tool_types::ToolDefinition;
        use uuid::{NoContext, Timestamp, Uuid};

        let registry = CapabilityRegistry::with_builtins();
        let ts = Timestamp::now(NoContext);
        let agent = Agent {
            id: Uuid::new_v7(ts),
            name: "Test Agent".to_string(),
            description: None,
            system_prompt: "Agent prompt.".to_string(),
            capabilities: vec![CapabilityIdType::from(CapabilityId::CURRENT_TIME)],
            status: AgentStatus::Active,
            default_model_id: None,
            tags: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let config = AgentConfigBuilder::new()
            .with_agent(&agent, &registry)
            .model("gpt-5.2")
            .build();

        assert!(config.system_prompt.contains("Agent prompt."));
        assert_eq!(config.tools.len(), 1);
        match &config.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "get_current_time");
            }
        }
    }

    #[test]
    fn test_builder_default() {
        let builder = AgentConfigBuilder::default();
        let config = builder.build();

        assert_eq!(config.system_prompt, "You are a helpful assistant.");
        assert_eq!(config.model, "gpt-5.2");
    }
}
