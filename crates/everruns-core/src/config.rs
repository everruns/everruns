// Agent configuration for the loop
//
// AgentConfig is a DB-agnostic configuration struct that can be:
// - Created directly for standalone usage
// - Built from an Agent entity via the `from_agent` method

use crate::agent::Agent;
use crate::capabilities::{apply_capabilities, AppliedCapabilities, CapabilityRegistry};
use crate::tool_types::ToolDefinition;
use crate::tools::ToolRegistry;
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
pub struct AgentConfigBuilder {
    config: AgentConfig,
}

impl AgentConfigBuilder {
    /// Start building a new configuration
    pub fn new() -> Self {
        Self {
            config: AgentConfig::default(),
        }
    }

    /// Set the system prompt
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.config.system_prompt = prompt.into();
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

/// Result of building configuration from an Agent with capabilities
pub struct AgentConfigFromAgent {
    /// The agent configuration with capabilities applied
    pub config: AgentConfig,
    /// Tool registry containing all capability tools
    pub tool_registry: ToolRegistry,
    /// IDs of capabilities that were applied
    pub applied_capability_ids: Vec<String>,
}

impl AgentConfigBuilder {
    /// Build an AgentConfig from an Agent entity, applying its capabilities.
    ///
    /// This method:
    /// 1. Creates a base config from the agent's system prompt and model
    /// 2. Applies the agent's capabilities using the provided registry
    /// 3. Returns the config with tool registry and applied capability IDs
    ///
    /// # Arguments
    ///
    /// * `agent` - The Agent entity to build config from
    /// * `model` - The model to use (since Agent doesn't have model_id resolved yet)
    /// * `registry` - The capability registry containing capability implementations
    ///
    /// # Example
    ///
    /// ```ignore
    /// use everruns_core::config::AgentConfigBuilder;
    /// use everruns_core::capabilities::CapabilityRegistry;
    ///
    /// let registry = CapabilityRegistry::with_builtins();
    /// let result = AgentConfigBuilder::from_agent(&agent, "gpt-4o", &registry);
    ///
    /// // Use result.config for LLM calls
    /// // Use result.tool_registry for executing tools
    /// ```
    pub fn from_agent(
        agent: &Agent,
        model: impl Into<String>,
        registry: &CapabilityRegistry,
    ) -> AgentConfigFromAgent {
        // Create base config from agent
        let base_config = AgentConfigBuilder::new()
            .system_prompt(&agent.system_prompt)
            .model(model)
            .build();

        // Get capability IDs as strings
        let capability_ids: Vec<String> = agent
            .capabilities
            .iter()
            .map(|cap_id| cap_id.as_str().to_string())
            .collect();

        // Apply capabilities
        let AppliedCapabilities {
            config,
            tool_registry,
            applied_ids,
        } = apply_capabilities(base_config, &capability_ids, registry);

        AgentConfigFromAgent {
            config,
            tool_registry,
            applied_capability_ids: applied_ids,
        }
    }
}
