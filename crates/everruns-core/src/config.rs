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
///
/// Can be created either from scratch with `new()` or from an Agent entity
/// with `from_agent()`. Use `build()` to get just the config, or
/// `build_with_capabilities()` to get the config along with the tool registry.
pub struct AgentConfigBuilder<'a> {
    config: AgentConfig,
    /// Optional: capability IDs to apply (from Agent)
    capability_ids: Vec<String>,
    /// Optional: capability registry for applying capabilities
    capability_registry: Option<&'a CapabilityRegistry>,
}

impl AgentConfigBuilder<'static> {
    /// Start building a new configuration from scratch
    pub fn new() -> Self {
        Self {
            config: AgentConfig::default(),
            capability_ids: Vec::new(),
            capability_registry: None,
        }
    }
}

impl<'a> AgentConfigBuilder<'a> {
    /// Start building a configuration from an Agent entity.
    ///
    /// This initializes the builder with the agent's system prompt and capabilities.
    /// Use `build_with_capabilities()` to get the config with applied capabilities
    /// and the resulting tool registry.
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
    /// let result = AgentConfigBuilder::from_agent(&agent, "gpt-4o", &registry)
    ///     .temperature(0.7)
    ///     .max_iterations(5)
    ///     .build_with_capabilities();
    ///
    /// // Use result.config for LLM calls
    /// // Use result.tool_registry for executing tools
    /// ```
    pub fn from_agent(
        agent: &Agent,
        model: impl Into<String>,
        registry: &'a CapabilityRegistry,
    ) -> AgentConfigBuilder<'a> {
        let capability_ids: Vec<String> = agent
            .capabilities
            .iter()
            .map(|cap_id| cap_id.as_str().to_string())
            .collect();

        AgentConfigBuilder {
            config: AgentConfig {
                system_prompt: agent.system_prompt.clone(),
                model: model.into(),
                tools: Vec::new(),
                max_iterations: default_max_iterations(),
                temperature: None,
                max_tokens: None,
            },
            capability_ids,
            capability_registry: Some(registry),
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

    /// Build the configuration without applying capabilities.
    ///
    /// Returns just the AgentConfig. If you need the tool registry from
    /// applied capabilities, use `build_with_capabilities()` instead.
    pub fn build(self) -> AgentConfig {
        self.config
    }

    /// Build the configuration and apply capabilities.
    ///
    /// If this builder was created with `from_agent()`, capabilities will be applied
    /// and the result includes the tool registry. Otherwise, returns just the config
    /// with an empty tool registry.
    pub fn build_with_capabilities(self) -> AgentConfigBuildResult {
        if let Some(registry) = self.capability_registry {
            // Apply capabilities
            let AppliedCapabilities {
                config,
                tool_registry,
                applied_ids,
            } = apply_capabilities(self.config, &self.capability_ids, registry);

            AgentConfigBuildResult {
                config,
                tool_registry,
                applied_capability_ids: applied_ids,
            }
        } else {
            // No capabilities to apply
            AgentConfigBuildResult {
                config: self.config,
                tool_registry: ToolRegistry::new(),
                applied_capability_ids: Vec::new(),
            }
        }
    }
}

impl Default for AgentConfigBuilder<'static> {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of building an AgentConfig with capabilities applied
pub struct AgentConfigBuildResult {
    /// The agent configuration with capabilities applied
    pub config: AgentConfig,
    /// Tool registry containing capability tools
    pub tool_registry: ToolRegistry,
    /// IDs of capabilities that were applied
    pub applied_capability_ids: Vec<String>,
}
