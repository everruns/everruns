// Agent configuration for the loop
//
// AgentConfig is a DB-agnostic configuration struct that can be:
// - Created directly for standalone usage
// - Built from an Agent entity via the `from_agent` method

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
