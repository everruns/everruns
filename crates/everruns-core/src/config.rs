// Agent configuration for the loop
//
// AgentConfig is a DB-agnostic configuration struct that can be:
// - Created directly for standalone usage
// - Built from an Agent entity via the `with_agent` builder method

use crate::agent::Agent;
use crate::capabilities::{CapabilityRegistry, CapabilityStatus};
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

        // Set system prompt from agent
        let builder = self.system_prompt(&agent.system_prompt);

        // Apply capabilities to builder
        apply_capabilities_to_builder(builder, &capability_ids, registry)
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

/// Apply capabilities to an AgentConfigBuilder using builder methods.
///
/// This function:
/// 1. Collects system prompt additions from capabilities (in order)
/// 2. Prepends them to the builder's system prompt
/// 3. Adds tool definitions from capabilities
///
/// # Arguments
///
/// * `builder` - The builder to modify
/// * `capability_ids` - Ordered list of capability IDs to apply
/// * `registry` - The capability registry containing implementations
fn apply_capabilities_to_builder(
    mut builder: AgentConfigBuilder,
    capability_ids: &[String],
    registry: &CapabilityRegistry,
) -> AgentConfigBuilder {
    let mut system_prompt_parts: Vec<String> = Vec::new();
    let mut tool_definitions: Vec<ToolDefinition> = Vec::new();

    // Collect contributions from capabilities
    for cap_id in capability_ids {
        if let Some(capability) = registry.get(cap_id) {
            // Only apply available capabilities
            if capability.status() != CapabilityStatus::Available {
                continue;
            }

            // Collect system prompt addition
            if let Some(addition) = capability.system_prompt_addition() {
                system_prompt_parts.push(addition.to_string());
            }

            // Collect tool definitions
            tool_definitions.extend(capability.tool_definitions());
        }
    }

    // Apply system prompt additions (prepend to existing)
    if !system_prompt_parts.is_empty() {
        let prefix = system_prompt_parts.join("\n\n");
        builder = builder.prepend_system_prompt(prefix);
    }

    // Apply tool definitions
    if !tool_definitions.is_empty() {
        builder = builder.tools(tool_definitions);
    }

    builder
}
