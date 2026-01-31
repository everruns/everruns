// Runtime agent configuration for the loop
//
// RuntimeAgent is a DB-agnostic configuration struct that can be:
// - Created directly for standalone usage
// - Built from an Agent entity via the `with_agent` builder method

use crate::agent::Agent;
use crate::capabilities::{CapabilityRegistry, collect_capabilities, resolve_dependencies};
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
}

fn default_max_iterations() -> usize {
    100
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
    /// use everruns_core::runtime_agent::RuntimeAgentBuilder;
    /// use everruns_core::capabilities::CapabilityRegistry;
    ///
    /// let registry = CapabilityRegistry::with_builtins();
    /// let runtime_agent = RuntimeAgentBuilder::new()
    ///     .with_agent(&agent, &registry)
    ///     .model("gpt-4o")
    ///     .temperature(0.7)
    ///     .build();
    /// ```
    pub fn with_agent(self, agent: &Agent, registry: &CapabilityRegistry) -> Self {
        let capability_ids: Vec<String> = agent
            .capabilities
            .iter()
            .map(|cap| cap.capability_id().to_string())
            .collect();

        self.system_prompt(&agent.system_prompt)
            .with_capabilities(&capability_ids, registry)
    }

    /// Apply capabilities to this builder.
    ///
    /// This resolves dependencies, then collects contributions from capabilities:
    /// - Dependencies are automatically included (topologically sorted)
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
        // Resolve dependencies first (dependencies come before dependents)
        let resolved_ids = match resolve_dependencies(capability_ids, registry) {
            Ok(resolved) => resolved.resolved_ids,
            Err(e) => {
                // Log error but continue with original IDs to avoid breaking
                tracing::warn!("Failed to resolve capability dependencies: {}", e);
                capability_ids.to_vec()
            }
        };

        let collected = collect_capabilities(&resolved_ids, registry);

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

    /// Build the runtime agent
    pub fn build(self) -> RuntimeAgent {
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
    use crate::capabilities::AgentCapabilityConfig;

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
    fn test_builder_with_capabilities_empty() {
        let registry = CapabilityRegistry::with_builtins();
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&[], &registry)
            .build();

        assert_eq!(runtime_agent.system_prompt, "Base prompt.");
        assert!(runtime_agent.tools.is_empty());
    }

    #[test]
    fn test_builder_with_capabilities_adds_tools() {
        use crate::tool_types::ToolDefinition;

        let registry = CapabilityRegistry::with_builtins();
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["current_time".to_string()], &registry)
            .build();

        assert_eq!(runtime_agent.tools.len(), 1);
        match &runtime_agent.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "get_current_time");
            }
        }
    }

    #[test]
    fn test_builder_with_capabilities_prepends_system_prompt() {
        let registry = CapabilityRegistry::with_builtins();
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["test_math".to_string()], &registry)
            .build();

        assert!(runtime_agent.system_prompt.contains("math tools"));
        assert!(runtime_agent.system_prompt.ends_with("Base prompt."));
    }

    #[test]
    fn test_builder_with_agent() {
        use crate::tool_types::ToolDefinition;
        use uuid::{NoContext, Timestamp, Uuid};

        let registry = CapabilityRegistry::with_builtins();
        let ts = Timestamp::now(NoContext);
        let agent = Agent {
            id: Uuid::new_v7(ts).into(),
            name: "Test Agent".to_string(),
            description: None,
            system_prompt: "Agent prompt.".to_string(),
            capabilities: vec![AgentCapabilityConfig::new("current_time")],
            status: AgentStatus::Active,
            default_model_id: None,
            tags: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            usage: None,
        };

        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &registry)
            .model("gpt-5.2")
            .build();

        assert!(runtime_agent.system_prompt.contains("Agent prompt."));
        assert_eq!(runtime_agent.tools.len(), 1);
        match &runtime_agent.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "get_current_time");
            }
        }
    }

    #[test]
    fn test_builder_default() {
        let builder = RuntimeAgentBuilder::default();
        let runtime_agent = builder.build();

        assert_eq!(runtime_agent.system_prompt, "You are a helpful assistant.");
        assert_eq!(runtime_agent.model, "gpt-5.2");
    }

    #[test]
    fn test_builder_with_capabilities_resolves_dependencies() {
        // Sample Data depends on Session File System
        // When we request only Sample Data, we should get system prompt from both
        let registry = CapabilityRegistry::with_builtins();
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["sample_data".to_string()], &registry)
            .build();

        // System prompt should include File System's contribution (the dependency)
        assert!(
            runtime_agent.system_prompt.contains("read_file"),
            "Should include File System system prompt (mentions read_file tool)"
        );
        assert!(
            runtime_agent.system_prompt.contains("write_file"),
            "Should include File System system prompt (mentions write_file tool)"
        );
        // Should also include Sample Data's contribution
        assert!(
            runtime_agent.system_prompt.contains("/samples"),
            "Should include Sample Data system prompt (mentions /samples path)"
        );
        // Base prompt should still be there
        assert!(
            runtime_agent.system_prompt.contains("Base prompt."),
            "Should preserve base prompt"
        );
    }

    #[test]
    fn test_builder_additive_capabilities() {
        use crate::tool_types::ToolDefinition;

        let registry = CapabilityRegistry::with_builtins();

        // Apply capabilities additively (simulating session-level capabilities)
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Agent prompt.")
            .with_capabilities(&["current_time".to_string()], &registry)
            .build();

        // Should have the tool from capability
        assert_eq!(runtime_agent.tools.len(), 1);
        match &runtime_agent.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "get_current_time");
            }
        }
    }

    #[test]
    fn test_builder_with_agent_and_additive_capabilities() {
        use crate::tool_types::ToolDefinition;
        use uuid::{NoContext, Timestamp, Uuid};

        let registry = CapabilityRegistry::with_builtins();
        let ts = Timestamp::now(NoContext);

        // Agent has current_time capability
        let agent = Agent {
            id: Uuid::new_v7(ts).into(),
            name: "Test Agent".to_string(),
            description: None,
            system_prompt: "Agent prompt.".to_string(),
            capabilities: vec![AgentCapabilityConfig::new("current_time")],
            status: AgentStatus::Active,
            default_model_id: None,
            tags: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            usage: None,
        };

        // Session adds test_math capability (additive)
        let session_capability_ids = vec!["test_math".to_string()];

        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &registry)
            .with_capabilities(&session_capability_ids, &registry)
            .model("gpt-5.2")
            .build();

        // Should have tools from both agent and session capabilities
        assert!(runtime_agent.tools.len() >= 2);
        let tool_names: Vec<&str> = runtime_agent
            .tools
            .iter()
            .map(|t| match t {
                ToolDefinition::Builtin(b) => b.name.as_str(),
            })
            .collect();
        assert!(tool_names.contains(&"get_current_time"));
        assert!(tool_names.contains(&"add"));

        // System prompt should contain both capability additions and agent prompt
        assert!(runtime_agent.system_prompt.contains("Agent prompt."));
        assert!(runtime_agent.system_prompt.contains("math tools"));
    }
}
