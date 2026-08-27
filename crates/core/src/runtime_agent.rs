// Runtime agent configuration for the loop
//
// RuntimeAgent is a DB-agnostic configuration struct that can be:
// - Created directly for standalone usage
// - Built from a AgentConfigOverlay via `from_overlay()` (preferred)
// - Built from individual Harness/Agent entities via builder methods (legacy)
//
// Preferred usage: merge Harness/Agent/Session into a AgentConfigOverlay, then:
//   RuntimeAgentBuilder::from_overlay(layer, &registry, &ctx).await
//       .model("gpt-5.2")
//       .build()
//
// Legacy per-entity methods (with_harness, with_agent) are kept for
// backward compatibility but the AgentConfigOverlay path is canonical.

use crate::agent_definition::AgentDefinition;
use crate::capabilities::{
    CapabilityRegistry, SystemPromptContext, ToolDefinitionHook, collect_capabilities_with_configs,
    compose_system_prompt, resolve_capability_configs,
};
use crate::config_layer::AgentConfigOverlay;
use crate::driver_registry::{PromptCacheConfig, ToolSearchConfig};
use crate::harness_definition::HarnessDefinition;
use crate::model_profiles::get_model_profile;
use crate::provider::DriverId;
use crate::tool_types::ToolDefinition;
use serde::{Deserialize, Serialize};

/// Runtime configuration for the agent loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAgent {
    /// System prompt that defines the agent's behavior
    pub system_prompt: String,

    /// Model identifier (e.g., "gpt-5.2", "claude-opus-5")
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

    /// Prompt caching config (set by prompt_caching capability)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache: Option<PromptCacheConfig>,

    /// OpenRouter routing controls, including provider-executed server tools
    /// (set by the `openrouter_server_tools` capability). Only forwarded to
    /// OpenRouter-compatible endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openrouter_routing: Option<crate::driver_registry::OpenRouterRoutingConfig>,

    /// Merged network access list (harness ∩ agent ∩ session).
    /// Used by tools (web_fetch) to enforce URL access policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<crate::network_access::NetworkAccessList>,

    /// Request-level parallel tool calling preference (EVE-598).
    ///
    /// `None` (default) preserves provider defaults and the act scheduler's
    /// class-aware concurrent schedule. `Some(true)` explicitly signals the
    /// provider that parallel tool calls are wanted; `Some(false)` asks the
    /// provider to emit at most one tool call per turn AND forces the act
    /// scheduler to serialize the batch (see `ActInput.parallel_tool_calls`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
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
            prompt_cache: None,
            openrouter_routing: None,
            network_access: None,
            parallel_tool_calls: None,
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
            prompt_cache: None,
            openrouter_routing: None,
            network_access: None,
            parallel_tool_calls: None,
        }
    }
}

/// Builder for RuntimeAgent with fluent API
///
/// Use `new()` to start building, then chain methods like `with_agent()`,
/// `model()`, `temperature()`, etc. Call `build()` to get the final runtime agent.
pub struct RuntimeAgentBuilder {
    runtime_agent: RuntimeAgent,
    tool_definition_hooks: Vec<std::sync::Arc<dyn ToolDefinitionHook>>,
}

impl RuntimeAgentBuilder {
    /// Start building a new runtime agent from scratch
    pub fn new() -> Self {
        Self {
            runtime_agent: RuntimeAgent::default(),
            tool_definition_hooks: Vec::new(),
        }
    }

    /// Build from a pre-merged AgentConfigOverlay.
    ///
    /// This is the preferred way to build a RuntimeAgent. The caller merges
    /// Harness/Agent/Session into a single AgentConfigOverlay (via `AgentConfigOverlay::fold`),
    /// then this method resolves capabilities and assembles the final config.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let layer = AgentConfigOverlay::fold([
    ///     AgentConfigOverlay::from(&harness),
    ///     AgentConfigOverlay::from(&agent),
    ///     AgentConfigOverlay::from(&session),
    /// ]);
    /// let runtime_agent = RuntimeAgentBuilder::from_overlay(layer, &registry, &ctx)
    ///     .await
    ///     .model("gpt-5.2")
    ///     .build();
    /// ```
    pub async fn from_overlay(
        layer: AgentConfigOverlay,
        registry: &CapabilityRegistry,
        ctx: &SystemPromptContext,
    ) -> Self {
        let mut builder = Self::new();

        // Always set system prompt (even to empty) so an intentionally empty
        // merged prompt clears the builder default instead of leaving it.
        builder = builder.system_prompt(layer.system_prompt.unwrap_or_default());

        // Resolve merged capabilities (once, on the effective set)
        builder = builder
            .with_capability_configs(&layer.capabilities, registry, ctx)
            .await;

        // Add tools from all layers
        if !layer.tools.is_empty() {
            builder = builder.tools(layer.tools);
        }

        // Set max_iterations if any layer specified it
        if let Some(max) = layer.max_iterations {
            builder = builder.max_iterations(max);
        }

        // Set merged network_access
        builder = builder.network_access(layer.network_access);

        // Set merged request-level parallel_tool_calls preference (EVE-598).
        // The explicit field is an escape hatch and wins over the
        // `parallel_tool_calls` capability applied during capability collection;
        // when unset, the capability-derived preference (if any) stands.
        if let Some(explicit) = layer.parallel_tool_calls {
            builder = builder.parallel_tool_calls(Some(explicit));
        }

        builder
    }

    /// Apply a Harness's configuration to this builder.
    ///
    /// Sets the system prompt from the harness and applies harness capabilities.
    /// Calls `system_prompt_contribution()` on each capability for dynamic content.
    /// Call this BEFORE `with_agent()` to establish the base prompt layer.
    pub async fn with_harness(
        self,
        harness: &HarnessDefinition,
        registry: &CapabilityRegistry,
        ctx: &SystemPromptContext,
    ) -> Self {
        self.system_prompt(harness.system_prompt.clone().unwrap_or_default())
            .with_capability_configs(&harness.capabilities, registry, ctx)
            .await
    }

    /// Apply an Agent's configuration to this builder.
    ///
    /// Applies the agent's system prompt and capabilities on top of the
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
        agent: &AgentDefinition,
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
    /// - System prompt additions are appended after the current system prompt
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

        // Apply system prompt additions after the stable base prompt.
        if let Some(prefix) = collected.system_prompt_prefix() {
            self.runtime_agent.system_prompt =
                compose_system_prompt(&self.runtime_agent.system_prompt, Some(&prefix));
        }

        // Apply tool definitions
        if !collected.tool_definitions.is_empty() {
            self = self.tools(collected.tool_definitions);
        }

        // Apply tool_search config if capability provided one
        if let Some(ts_config) = collected.tool_search {
            self.runtime_agent.tool_search = Some(ts_config);
        }

        if let Some(pc_config) = collected.prompt_cache {
            self.runtime_agent.prompt_cache = Some(pc_config);
        }

        if let Some(routing) = collected.openrouter_routing {
            self.runtime_agent.openrouter_routing = Some(routing);
        }

        // Apply the `parallel_tool_calls` capability preference. An explicit
        // request-level field set later (see `from_overlay`) takes precedence.
        if let Some(ptc) = collected.parallel_tool_calls {
            self.runtime_agent.parallel_tool_calls = Some(ptc);
        }

        self.tool_definition_hooks
            .extend(collected.tool_definition_hooks);

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

    /// Append locale instructions for session-aware localization.
    pub fn with_locale(self, locale: Option<&str>) -> Self {
        let Some(locale) = locale.map(str::trim).filter(|value| !value.is_empty()) else {
            return self;
        };

        self.append_system_prompt(format!(
            "<locale preference=\"{locale}\">\n\
             Default locale for this session: {locale}.\n\
             Unless the user explicitly asks otherwise, respond in this locale and use its language, spelling, and regional formatting conventions for dates, times, numbers, and currency.\n\
             </locale>"
        ))
    }

    /// Append text to the system prompt
    pub fn append_system_prompt(mut self, suffix: impl Into<String>) -> Self {
        let suffix = suffix.into();
        if !suffix.is_empty() {
            if self.runtime_agent.system_prompt.is_empty() {
                self.runtime_agent.system_prompt = suffix;
            } else {
                self.runtime_agent.system_prompt =
                    format!("{}\n\n{}", self.runtime_agent.system_prompt, suffix);
            }
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

    /// Set the merged network access list.
    pub fn network_access(
        mut self,
        network_access: Option<crate::network_access::NetworkAccessList>,
    ) -> Self {
        self.runtime_agent.network_access = network_access;
        self
    }

    /// Set the request-level parallel tool calling preference (EVE-598).
    pub fn parallel_tool_calls(mut self, parallel_tool_calls: Option<bool>) -> Self {
        self.runtime_agent.parallel_tool_calls = parallel_tool_calls;
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

    /// Set prompt caching configuration
    pub fn prompt_cache(mut self, config: PromptCacheConfig) -> Self {
        self.runtime_agent.prompt_cache = Some(config);
        self
    }

    /// Build the runtime agent.
    ///
    /// Validates that a hosted tool_search config is only kept for models that
    /// support it (OpenAI GPT-5.4+ and Claude Sonnet 4 / Opus 4 / Haiku 4.5 /
    /// Fable 5 and newer). Clears it for unsupported models to prevent 400 errors
    /// from the provider API.
    ///
    /// tool_search is capability-driven: a hosted config is only set when the
    /// `openai_tool_search` / `claude_tool_search` capability (directly or via
    /// `auto_tool_search`) is added to the agent or harness. This method does NOT
    /// auto-enable it.
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

        // Resolve tool_search (deferred tool loading). The mechanism is already
        // chosen at capability-collection time (see `Capability::resolve_for_model`
        // and `auto_tool_search`): a hosted `ToolSearchConfig` means the hosted
        // (native) mechanism; client-side deferral arrives as `DeferSchemaHook`
        // plus a `tool_search` tool. This step only reconciles a hosted config
        // with the model — collection may have set one (via a direct
        // `openai_tool_search` capability) that the model can't honor.
        // A hosted config is honorable when any provider with a driver that
        // renders the hosted format advertises tool_search for this model:
        // OpenAI (Responses) and Anthropic (Messages). A model id resolves under
        // at most one of these provider profiles, so the other lookup is None.
        let model_supports_native =
            [DriverId::OpenAI, DriverId::Anthropic]
                .iter()
                .any(|provider| {
                    get_model_profile(provider, &self.runtime_agent.model)
                        .is_some_and(|p| p.tool_search)
                });

        // Hosted (native) deferral hides schemas server-side, so client-side
        // opt-out hooks (DeferSchemaHook) must be skipped while a hosted config
        // is present — even on an unsupported model, where the hosted config is
        // disabled below (full schemas, no client-side fallback). This is what
        // makes a hand-configured `openai_tool_search` win over a separately
        // configured `tool_search`.
        let native_tool_search = self.runtime_agent.tool_search.is_some();
        for hook in &self.tool_definition_hooks {
            if native_tool_search && !hook.applies_with_native_tool_search() {
                continue;
            }
            self.runtime_agent.tools =
                hook.transform(std::mem::take(&mut self.runtime_agent.tools));
        }

        // Clear a hosted config the model can't honor (a direct `openai_tool_search`
        // on an unsupported model): it simply sends full schemas. `auto_tool_search`
        // never reaches here on an unsupported model — it resolves to the generic
        // client-side mechanism at collection time and sets no hosted config.
        if self.runtime_agent.tool_search.is_some() && !model_supports_native {
            tracing::debug!(
                model = %self.runtime_agent.model,
                "hosted tool_search not supported by model; disabling (full schemas)"
            );
            self.runtime_agent.tool_search = None;
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
    use crate::AgentCapabilityConfig;
    use crate::capabilities::{Capability, SystemPromptContext};
    use crate::typed_id::AgentId;

    struct ToolFixtureCapability;

    impl Capability for ToolFixtureCapability {
        fn id(&self) -> &str {
            "tool_fixture"
        }

        fn name(&self) -> &str {
            "Tool Fixture"
        }

        fn description(&self) -> &str {
            "Neutral capability fixture with one tool."
        }

        fn tools(&self) -> Vec<Box<dyn crate::Tool>> {
            vec![Box::new(crate::tools::EchoTool)]
        }
    }

    struct PromptToolFixtureCapability;

    impl Capability for PromptToolFixtureCapability {
        fn id(&self) -> &str {
            "prompt_tool_fixture"
        }

        fn name(&self) -> &str {
            "Prompt Tool Fixture"
        }

        fn description(&self) -> &str {
            "Neutral capability fixture with a prompt and tool."
        }

        fn system_prompt_addition(&self) -> Option<&str> {
            Some("Task Management fixture guidance.")
        }

        fn tools(&self) -> Vec<Box<dyn crate::Tool>> {
            vec![Box::new(crate::progress_reporting::ReportProgressTool)]
        }
    }

    fn fixture_registry() -> CapabilityRegistry {
        let mut registry = crate::CapabilityRegistry::new();
        registry.register(ToolFixtureCapability);
        registry.register(PromptToolFixtureCapability);
        registry
    }

    fn test_ctx() -> SystemPromptContext {
        SystemPromptContext::without_file_store(crate::typed_id::SessionId::new())
    }

    struct FileSystemFixture;

    impl crate::capabilities::Capability for FileSystemFixture {
        fn id(&self) -> &str {
            "session_file_system"
        }
        fn name(&self) -> &str {
            "Fixture Filesystem"
        }
        fn description(&self) -> &str {
            "Fixture for host-supplied filesystem composition."
        }
        fn system_prompt_addition(&self) -> Option<&str> {
            Some("The workspace root is `/workspace`.")
        }
    }

    #[test]
    fn test_runtime_agent_new() {
        let runtime_agent = RuntimeAgent::new("You are helpful.", "gpt-5.2");

        assert_eq!(runtime_agent.system_prompt, "You are helpful.");
        assert_eq!(runtime_agent.model, "gpt-5.2");
        assert!(runtime_agent.tools.is_empty());
        assert_eq!(runtime_agent.max_iterations, 500);
        assert!(runtime_agent.temperature.is_none());
        assert!(runtime_agent.max_tokens.is_none());
    }

    #[test]
    fn test_runtime_agent_default() {
        let runtime_agent = RuntimeAgent::default();

        assert_eq!(runtime_agent.system_prompt, "You are a helpful assistant.");
        assert_eq!(runtime_agent.model, "gpt-5.2");
        assert!(runtime_agent.tools.is_empty());
        assert_eq!(runtime_agent.max_iterations, 500);
    }

    #[test]
    fn test_builder_basic() {
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Custom prompt")
            .model("claude-opus-5")
            .build();

        assert_eq!(runtime_agent.system_prompt, "Custom prompt");
        assert_eq!(runtime_agent.model, "claude-opus-5");
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
    fn test_builder_with_locale_appends_locale_instructions() {
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_locale(Some("uk-UA"))
            .build();

        assert!(runtime_agent.system_prompt.starts_with("Base prompt."));
        assert!(runtime_agent.system_prompt.contains("<locale"));
        assert!(runtime_agent.system_prompt.contains("uk-UA"));
        assert!(runtime_agent.system_prompt.ends_with("</locale>"));
    }

    #[tokio::test]
    async fn test_builder_with_capabilities_empty() {
        let registry = fixture_registry();
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

        let registry = fixture_registry();
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["tool_fixture".to_string()], &registry, &test_ctx())
            .await
            .build();

        assert_eq!(runtime_agent.tools.len(), 1);
        match &runtime_agent.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "echo");
            }
            _ => panic!("expected Builtin variant"),
        }
    }

    #[tokio::test]
    async fn test_builder_with_capabilities_keeps_base_prompt_first() {
        let mut registry = CapabilityRegistry::new();
        registry.register(FileSystemFixture);
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
                .starts_with("<system-prompt>\nBase prompt.\n</system-prompt>")
        );
    }

    #[tokio::test]
    async fn test_builder_with_agent() {
        use crate::tool_types::ToolDefinition;
        use uuid::{NoContext, Timestamp, Uuid};

        let registry = fixture_registry();
        let ts = Timestamp::now(NoContext);
        let uuid = Uuid::new_v7(ts);
        let agent = AgentDefinition {
            display_name: Some("Test Agent".to_string()),
            capabilities: vec![AgentCapabilityConfig::new("tool_fixture")],
            ..AgentDefinition::new(
                AgentId::from_uuid(uuid),
                "test-agent".to_string(),
                "Agent prompt.".to_string(),
            )
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
                assert_eq!(tool.name, "echo");
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
        // Local stand-in for the `sample_data` fixture (now in
        // everruns-test-support, EVE-875): mounts + a dependency on
        // session_file_system.
        struct SampleDataFixture;

        impl crate::capabilities::Capability for SampleDataFixture {
            fn id(&self) -> &str {
                "sample_data"
            }
            fn name(&self) -> &str {
                "Sample Data"
            }
            fn description(&self) -> &str {
                "Fixture: mounted sample files."
            }
            fn system_prompt_addition(&self) -> Option<&str> {
                Some("Read-only sample files are mounted at `/samples`.")
            }
            fn dependencies(&self) -> Vec<&'static str> {
                vec!["session_file_system"]
            }
        }

        // Sample Data depends on Session File System
        // When we request only Sample Data, we should get system prompt from both
        let mut registry = CapabilityRegistry::new();
        registry.register(FileSystemFixture);
        registry.register(SampleDataFixture);
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

        let registry = fixture_registry();

        // Apply capabilities additively (simulating session-level capabilities)
        let runtime_agent = RuntimeAgentBuilder::new()
            .system_prompt("Agent prompt.")
            .with_capabilities(&["tool_fixture".to_string()], &registry, &test_ctx())
            .await
            .build();

        // Should have the tool from capability
        assert_eq!(runtime_agent.tools.len(), 1);
        match &runtime_agent.tools[0] {
            ToolDefinition::Builtin(tool) => {
                assert_eq!(tool.name, "echo");
            }
            _ => panic!("expected Builtin variant"),
        }
    }

    #[tokio::test]
    async fn test_builder_with_agent_client_side_tools() {
        use crate::tool_types::{ClientSideTool, DeferrablePolicy, ToolDefinition};
        use uuid::{NoContext, Timestamp, Uuid};

        let registry = fixture_registry();
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
            full_parameters: None,
        });

        let agent = AgentDefinition {
            display_name: Some("Client Tool Agent".to_string()),
            tools: vec![client_tool],
            ..AgentDefinition::new(
                AgentId::from_uuid(uuid),
                "client-tool-agent".to_string(),
                "Agent with client tools.".to_string(),
            )
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

        let registry = fixture_registry();
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
            full_parameters: None,
        });

        let agent = AgentDefinition {
            display_name: Some("Mixed Tool Agent".to_string()),
            capabilities: vec![AgentCapabilityConfig::new("tool_fixture")],
            tools: vec![client_tool],
            ..AgentDefinition::new(
                AgentId::from_uuid(uuid),
                "mixed-tool-agent".to_string(),
                "Agent with mixed tools.".to_string(),
            )
        };

        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &registry, &test_ctx())
            .await
            .model("gpt-5.2")
            .build();

        // Should have capability tool + client-side tool
        assert_eq!(runtime_agent.tools.len(), 2);
        let tool_names: Vec<&str> = runtime_agent.tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"echo"));
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

        let registry = fixture_registry();
        let ts = Timestamp::now(NoContext);

        // Agent has a tool-only capability (no system prompt addition).
        let uuid = Uuid::new_v7(ts);
        let agent = AgentDefinition {
            display_name: Some("Test Agent".to_string()),
            capabilities: vec![AgentCapabilityConfig::new("tool_fixture")],
            ..AgentDefinition::new(
                AgentId::from_uuid(uuid),
                "test-agent".to_string(),
                "Agent prompt.".to_string(),
            )
        };

        // Session adds a prompt-bearing capability additively.
        let session_capability_ids = vec!["prompt_tool_fixture".to_string()];

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
        assert!(tool_names.contains(&"echo"));
        assert!(tool_names.contains(&"report_progress"));

        // System prompt should contain both capability additions and agent prompt
        assert!(runtime_agent.system_prompt.contains("Agent prompt."));
        assert!(runtime_agent.system_prompt.contains("Task Management"));
        assert!(
            runtime_agent
                .system_prompt
                .contains("<capability id=\"prompt_tool_fixture\">")
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
    fn test_build_skips_client_side_hook_when_native_tool_search_configured() {
        use crate::tool_types::{BuiltinTool, ToolPolicy};
        use std::sync::Arc;

        // A hook that would clear all tools if it ran, but opts out of coexisting
        // with native tool_search (like the generic tool_search DeferSchemaHook).
        struct ClearAllHook;
        impl ToolDefinitionHook for ClearAllHook {
            fn transform(&self, _tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
                vec![]
            }
            fn applies_with_native_tool_search(&self) -> bool {
                false
            }
        }

        let tool = ToolDefinition::Builtin(BuiltinTool {
            name: "read_file".to_string(),
            display_name: None,
            description: "read".to_string(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        });

        // Native tool_search configured → opt-out hook is skipped (tools kept).
        let mut builder = RuntimeAgentBuilder::new()
            .model("gpt-5.4")
            .tools(vec![tool.clone()])
            .tool_search(ToolSearchConfig {
                enabled: true,
                threshold: 15,
            });
        builder.tool_definition_hooks.push(Arc::new(ClearAllHook));
        assert_eq!(
            builder.build().tools.len(),
            1,
            "opt-out hook must be skipped when native tool_search is configured"
        );

        // No native tool_search → the same hook runs and clears the tools.
        let mut builder = RuntimeAgentBuilder::new()
            .model("claude-sonnet-4-5-20250514")
            .tools(vec![tool]);
        builder.tool_definition_hooks.push(Arc::new(ClearAllHook));
        assert!(
            builder.build().tools.is_empty(),
            "opt-out hook runs when native tool_search is not configured"
        );
    }

    #[test]
    fn test_build_clears_tool_search_for_non_native_model() {
        // A retired pre-4 Claude model has no profile (and no hosted
        // tool_search support on either provider), so a hosted config is
        // cleared (full schemas sent).
        let agent = RuntimeAgentBuilder::new()
            .model("claude-3-5-haiku")
            .tool_search(ToolSearchConfig {
                enabled: true,
                threshold: 15,
            })
            .build();

        assert!(
            agent.tool_search.is_none(),
            "tool_search should be cleared for models with no hosted support"
        );
    }

    #[test]
    fn test_build_keeps_tool_search_for_native_anthropic_model() {
        // Claude 4-family models support Anthropic's hosted tool_search, so the
        // hosted config survives build() (the Anthropic driver renders it).
        let agent = RuntimeAgentBuilder::new()
            .model("claude-opus-4-8")
            .tool_search(ToolSearchConfig {
                enabled: true,
                threshold: 15,
            })
            .build();

        assert!(
            agent.tool_search.is_some(),
            "tool_search should be kept for native Anthropic models"
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

    // Note: `auto_tool_search`'s hosted-vs-client-side selection now happens at
    // capability-collection time (see `Capability::resolve_for_model` and the
    // collection tests in `capabilities::mod`), not in `build()`. `build()` only
    // reconciles a hosted config with the model, covered by the tests above.

    #[test]
    fn test_build_preserves_prompt_cache_for_supported_provider() {
        let agent = RuntimeAgentBuilder::new()
            .model("gpt-5.4")
            .prompt_cache(PromptCacheConfig {
                enabled: true,
                strategy: crate::driver_registry::PromptCacheStrategy::Auto,
                gemini_cached_content: None,
            })
            .build();

        let prompt_cache = agent
            .prompt_cache
            .expect("explicit prompt_cache should be preserved");
        assert!(prompt_cache.enabled);
        assert_eq!(
            prompt_cache.strategy,
            crate::driver_registry::PromptCacheStrategy::Auto
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
                full_parameters: None,
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
