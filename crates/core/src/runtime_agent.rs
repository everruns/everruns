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
    ///     .model("gpt-5.2")
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

    fn client_tool(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition::ClientSide(crate::tool_types::ClientSideTool {
            name: name.into(),
            display_name: Some("Client action".into()),
            description: description.into(),
            parameters: serde_json::json!({"type":"object","properties":{"selector":{"type":"string"}},"required":["selector"]}),
            category: Some("Browser".into()),
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        })
    }

    fn echo_definition() -> ToolDefinition {
        crate::Tool::to_definition(&crate::tools::EchoTool)
            .with_capability_attribution("tool_fixture", Some("Tool Fixture"))
    }

    fn progress_definition() -> ToolDefinition {
        crate::Tool::to_definition(&crate::progress_reporting::ReportProgressTool)
            .with_capability_attribution("prompt_tool_fixture", Some("Prompt Tool Fixture"))
    }

    fn tools_json(tools: &[ToolDefinition]) -> serde_json::Value {
        serde_json::to_value(tools).unwrap()
    }

    #[test]
    fn minimal_construction_and_legacy_wire_input_preserve_iteration_limit() {
        let expected = serde_json::json!({
            "system_prompt":"Custom prompt", "model":"custom-model", "tools":[],
            "max_iterations":500, "temperature":null, "max_tokens":null
        });
        for agent in [
            RuntimeAgent::new("Custom prompt", "custom-model"),
            RuntimeAgentBuilder::new()
                .system_prompt("Custom prompt")
                .model("custom-model")
                .build(),
            serde_json::from_value::<RuntimeAgent>(
                serde_json::json!({"system_prompt":"Custom prompt","model":"custom-model"}),
            )
            .unwrap(),
        ] {
            assert_eq!(serde_json::to_value(agent).unwrap(), expected);
        }
    }

    #[test]
    fn builder_preserves_all_explicit_request_options() {
        let tool = client_tool("click", "Click a selector");
        let policy = crate::network_access::NetworkAccessList::block(["private.example.com"]);
        let agent = RuntimeAgentBuilder::default()
            .system_prompt("You are a coder.")
            .model("gpt-5.4")
            .max_iterations(23)
            .temperature(0.75)
            .max_tokens(2048)
            .parallel_tool_calls(Some(false))
            .network_access(Some(policy.clone()))
            .tool(tool.clone())
            .build();
        assert_eq!(
            serde_json::to_value(agent).unwrap(),
            serde_json::json!({
                "system_prompt":"You are a coder.", "model":"gpt-5.4", "tools":[tool],
                "max_iterations":23, "temperature":0.75, "max_tokens":2048,
                "network_access":policy, "parallel_tool_calls":false
            })
        );
    }

    #[test]
    fn prompt_operations_preserve_order_and_ignore_empty_additions() {
        for (prefix, suffix, expected) in [
            ("", "", "Base prompt."),
            ("Prefix.", "", "Prefix.\n\nBase prompt."),
            ("", "Suffix.", "Base prompt.\n\nSuffix."),
            ("Prefix.", "Suffix.", "Prefix.\n\nBase prompt.\n\nSuffix."),
        ] {
            let agent = RuntimeAgentBuilder::new()
                .system_prompt("Base prompt.")
                .prepend_system_prompt(prefix)
                .append_system_prompt(suffix)
                .build();
            assert_eq!(agent.system_prompt, expected);
        }
        assert_eq!(
            RuntimeAgentBuilder::new()
                .system_prompt("")
                .append_system_prompt("Only suffix.")
                .build()
                .system_prompt,
            "Only suffix."
        );
    }

    #[test]
    fn locale_instructions_trim_input_preserve_base_and_omit_empty_preferences() {
        for locale in [None, Some(""), Some(" \t")] {
            let agent = RuntimeAgentBuilder::new()
                .system_prompt("Base prompt.")
                .with_locale(locale)
                .build();
            assert_eq!(agent.system_prompt, "Base prompt.");
        }
        for locale in ["uk-UA", " uk-UA \n"] {
            let prompt = RuntimeAgentBuilder::new()
                .system_prompt("Base prompt.")
                .with_locale(Some(locale))
                .build()
                .system_prompt;
            assert!(prompt.starts_with("Base prompt.\n\n<locale preference=\"uk-UA\">\n"));
            assert!(prompt.contains("Default locale for this session: uk-UA.\n"));
            assert!(prompt.ends_with("\n</locale>"));
            assert_eq!(prompt.matches("Base prompt.").count(), 1);
            assert_eq!(prompt.matches("<locale ").count(), 1);
        }
    }

    #[tokio::test]
    async fn empty_capability_application_preserves_existing_configuration() {
        let tool = client_tool("click", "existing tool");
        let agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .model("custom-model")
            .max_iterations(19)
            .tool(tool.clone())
            .parallel_tool_calls(Some(false))
            .with_capabilities(&[], &fixture_registry(), &test_ctx())
            .await
            .build();
        assert_eq!(
            serde_json::to_value(agent).unwrap(),
            serde_json::json!({
                "system_prompt":"Base prompt.","model":"custom-model","tools":[tool],
                "max_iterations":19,"temperature":null,"max_tokens":null,"parallel_tool_calls":false
            })
        );
    }

    #[tokio::test]
    async fn direct_capabilities_preserve_complete_tool_definitions() {
        let agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["tool_fixture".into()], &fixture_registry(), &test_ctx())
            .await
            .build();
        assert_eq!(tools_json(&agent.tools), tools_json(&[echo_definition()]));
        assert_eq!(agent.system_prompt, "Base prompt.");
    }

    #[tokio::test]
    async fn agent_application_preserves_client_and_capability_tool_payloads() {
        for (with_capability, with_client) in [(true, false), (false, true), (true, true)] {
            let mut source = AgentDefinition::new(AgentId::new(), "test-agent", "Agent prompt.");
            let client = client_tool("click", "Click the requested selector");
            let mut expected = Vec::new();
            if with_capability {
                source
                    .capabilities
                    .push(AgentCapabilityConfig::new("tool_fixture"));
                expected.push(echo_definition());
            }
            if with_client {
                source.tools.push(client.clone());
                expected.push(client);
            }
            let agent = RuntimeAgentBuilder::new()
                .with_agent(&source, &fixture_registry(), &test_ctx())
                .await
                .build();
            assert_eq!(agent.system_prompt, "Agent prompt.");
            assert_eq!(tools_json(&agent.tools), tools_json(&expected));
        }
    }

    #[tokio::test]
    async fn capability_prompt_follows_stable_base_once() {
        let mut registry = CapabilityRegistry::new();
        registry.register(FileSystemFixture);
        let agent = RuntimeAgentBuilder::new()
            .system_prompt("Base prompt.")
            .with_capabilities(&["session_file_system".into()], &registry, &test_ctx())
            .await
            .build();
        assert_eq!(
            agent.system_prompt,
            "<system-prompt>\nBase prompt.\n</system-prompt>\n\n<capability id=\"session_file_system\">\nThe workspace root is `/workspace`.\n</capability>"
        );
    }

    #[tokio::test]
    async fn additive_capabilities_preserve_prior_tools_and_prompt() {
        let mut source = AgentDefinition::new(AgentId::new(), "test-agent", "Agent prompt.");
        source
            .capabilities
            .push(AgentCapabilityConfig::new("tool_fixture"));
        let registry = fixture_registry();
        let agent = RuntimeAgentBuilder::new()
            .with_agent(&source, &registry, &test_ctx())
            .await
            .with_capabilities(&["prompt_tool_fixture".into()], &registry, &test_ctx())
            .await
            .build();
        assert_eq!(
            tools_json(&agent.tools),
            tools_json(&[echo_definition(), progress_definition()])
        );
        assert_eq!(
            agent.system_prompt,
            "<system-prompt>\nAgent prompt.\n</system-prompt>\n\n<capability id=\"prompt_tool_fixture\">\nTask Management fixture guidance.\n</capability>"
        );
    }

    #[test]
    fn hosted_tool_search_requires_support_and_preserves_explicit_config() {
        for (model, supported) in [
            ("gpt-5.2", false),
            ("claude-3-5-haiku", false),
            ("unknown-model", false),
            ("gpt-5.4", true),
            ("claude-opus-4-8", true),
        ] {
            assert!(
                RuntimeAgentBuilder::new()
                    .model(model)
                    .build()
                    .tool_search
                    .is_none(),
                "must not auto-enable for {model}"
            );
            for (enabled, threshold) in [(true, 5), (false, 0)] {
                let agent = RuntimeAgentBuilder::new()
                    .model(model)
                    .tool_search(ToolSearchConfig { enabled, threshold })
                    .build();
                let expected =
                    supported.then(|| serde_json::json!({"enabled":enabled,"threshold":threshold}));
                assert_eq!(
                    agent.tool_search.map(|v| serde_json::to_value(v).unwrap()),
                    expected,
                    "{model}"
                );
            }
        }
    }

    #[test]
    fn hooks_run_in_order_and_respect_native_configuration_before_model_filtering() {
        struct AppendHook {
            suffix: &'static str,
            native: bool,
        }
        impl ToolDefinitionHook for AppendHook {
            fn transform(&self, mut tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
                for tool in &mut tools {
                    match tool {
                        ToolDefinition::Builtin(t) => t.description.push_str(self.suffix),
                        ToolDefinition::ClientSide(t) => t.description.push_str(self.suffix),
                    }
                }
                tools
            }
            fn applies_with_native_tool_search(&self) -> bool {
                self.native
            }
        }
        for (model, configured, expected_description, kept_native) in [
            ("gpt-5.4", false, "original|first|conditional|last", false),
            ("gpt-5.4", true, "original|first|last", true),
            ("gpt-5.2", true, "original|first|last", false),
        ] {
            let mut builder = RuntimeAgentBuilder::new()
                .model(model)
                .tool(client_tool("click", "original"));
            if configured {
                builder = builder.tool_search(ToolSearchConfig {
                    enabled: true,
                    threshold: 9,
                });
            }
            for (suffix, native) in [("|first", true), ("|conditional", false), ("|last", true)] {
                builder
                    .tool_definition_hooks
                    .push(std::sync::Arc::new(AppendHook { suffix, native }));
            }
            let agent = builder.build();
            assert_eq!(
                tools_json(&agent.tools),
                tools_json(&[client_tool("click", expected_description)])
            );
            assert_eq!(agent.tool_search.is_some(), kept_native);
        }
    }

    #[test]
    fn prompt_cache_config_is_preserved_for_driver_resolution() {
        for model in ["gpt-5.4", "gemini-3-pro", "custom-model"] {
            for enabled in [false, true] {
                let config = PromptCacheConfig {
                    enabled,
                    strategy: crate::driver_registry::PromptCacheStrategy::Auto,
                    gemini_cached_content: Some("cachedContents/review-fixture".into()),
                };
                let agent = RuntimeAgentBuilder::new()
                    .model(model)
                    .prompt_cache(config.clone())
                    .build();
                assert_eq!(agent.prompt_cache, Some(config));
            }
        }
    }

    #[test]
    fn deduplication_keeps_complete_last_definition_and_survivor_order() {
        let mut first = echo_definition();
        if let ToolDefinition::Builtin(t) = &mut first {
            t.name = "click".into();
        }
        let retained = client_tool("search", "retained");
        let last = client_tool("click", "last wins across tool variants");
        let agent = RuntimeAgentBuilder::new()
            .tool(first)
            .tools([retained.clone(), last.clone()])
            .build();
        assert_eq!(tools_json(&agent.tools), tools_json(&[retained, last]));
    }

    struct ConfiguredFixture;
    impl Capability for ConfiguredFixture {
        fn id(&self) -> &str {
            "configured_fixture"
        }
        fn name(&self) -> &str {
            "Configured Fixture"
        }
        fn description(&self) -> &str {
            "Config-driven preferences for assembly tests."
        }
        fn tool_search_config(&self, config: &serde_json::Value) -> Option<ToolSearchConfig> {
            Some(ToolSearchConfig {
                enabled: true,
                threshold: config["threshold"].as_u64().unwrap() as usize,
            })
        }
        fn prompt_cache_config(&self, config: &serde_json::Value) -> Option<PromptCacheConfig> {
            Some(PromptCacheConfig {
                enabled: config["cache_enabled"].as_bool().unwrap(),
                strategy: crate::driver_registry::PromptCacheStrategy::Auto,
                gemini_cached_content: config["cache"].as_str().map(str::to_owned),
            })
        }
        fn parallel_tool_calls_preference(&self, config: &serde_json::Value) -> Option<bool> {
            config["parallel"].as_bool()
        }
        fn openrouter_routing_config(
            &self,
            config: &serde_json::Value,
        ) -> Option<crate::driver_registry::OpenRouterRoutingConfig> {
            Some(serde_json::from_value(config["routing"].clone()).unwrap())
        }
    }

    #[tokio::test]
    async fn canonical_overlay_preserves_configured_contributions_and_explicit_precedence() {
        let mut registry = fixture_registry();
        registry.register(ConfiguredFixture);
        for (capability_parallel, explicit_parallel, expected_parallel) in [
            (true, None, true),
            (false, None, false),
            (true, Some(false), false),
            (false, Some(true), true),
        ] {
            let client = client_tool("click", "overlay client");
            let policy = crate::network_access::NetworkAccessList::block(["private.example.com"]);
            let routing =
                serde_json::json!({"models":["openai/a","anthropic/b"],"route":"fallback"});
            let layer = AgentConfigOverlay {
                system_prompt: Some("Overlay prompt.".into()),
                capabilities: vec![
                    AgentCapabilityConfig::new("tool_fixture"),
                    AgentCapabilityConfig::with_config(
                        "configured_fixture",
                        serde_json::json!({
                            "threshold":37,"cache_enabled":false,"cache":"cachedContents/configured", "parallel":capability_parallel,"routing":routing
                        }),
                    ),
                ],
                tools: vec![client.clone()],
                max_iterations: Some(0),
                network_access: Some(policy.clone()),
                parallel_tool_calls: explicit_parallel,
                ..Default::default()
            };
            let agent = RuntimeAgentBuilder::from_overlay(layer, &registry, &test_ctx())
                .await
                .model("gpt-5.4")
                .build();
            assert_eq!(agent.system_prompt, "Overlay prompt.");
            assert_eq!(
                tools_json(&agent.tools),
                tools_json(&[echo_definition(), client])
            );
            assert_eq!(agent.max_iterations, 0);
            assert_eq!(agent.network_access, Some(policy));
            assert_eq!(agent.parallel_tool_calls, Some(expected_parallel));
            assert_eq!(
                serde_json::to_value(agent.tool_search.unwrap()).unwrap(),
                serde_json::json!({"enabled":true,"threshold":37})
            );
            assert_eq!(
                agent.prompt_cache,
                Some(PromptCacheConfig {
                    enabled: false,
                    strategy: crate::driver_registry::PromptCacheStrategy::Auto,
                    gemini_cached_content: Some("cachedContents/configured".into())
                })
            );
            assert_eq!(
                serde_json::to_value(agent.openrouter_routing.unwrap()).unwrap(),
                routing
            );
        }
    }

    #[tokio::test]
    async fn empty_overlay_clears_default_prompt_without_enabling_preferences() {
        for prompt in [None, Some(String::new())] {
            let agent = RuntimeAgentBuilder::from_overlay(
                AgentConfigOverlay {
                    system_prompt: prompt,
                    ..Default::default()
                },
                &fixture_registry(),
                &test_ctx(),
            )
            .await
            .build();
            assert_eq!(agent.system_prompt, "");
            assert!(agent.tools.is_empty());
            assert_eq!(agent.max_iterations, 500);
            assert!(agent.tool_search.is_none());
            assert!(agent.prompt_cache.is_none());
            assert!(agent.openrouter_routing.is_none());
            assert!(agent.network_access.is_none());
            assert_eq!(agent.parallel_tool_calls, None);
        }
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

        assert_eq!(
            runtime_agent.system_prompt,
            concat!(
                "<system-prompt>\nBase prompt.\n</system-prompt>\n\n",
                "<capability id=\"session_file_system\">\nThe workspace root is `/workspace`.\n</capability>\n\n",
                "<capability id=\"sample_data\">\nRead-only sample files are mounted at `/samples`.\n</capability>"
            )
        );
        assert!(runtime_agent.tools.is_empty());
    }
}
