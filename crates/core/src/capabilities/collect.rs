//! Collecting capabilities, filters, view providers, facts and MCP servers.

//! Capabilities Module for Agent Loop
//!
//! This module provides the capabilities abstraction that allows composing
//! agent functionality through modular units. Each capability can contribute:
//! - System prompt additions
//! - Tools for the agent
//! - Behavior modifications (future)
//!
//! Design decisions:
//! - Capabilities are defined via the Capability trait for flexibility
//! - CapabilityRegistry holds all available capability implementations
//! - apply_capabilities() merges capability contributions into RuntimeAgent
//! - The agent-loop remains execution-focused; capabilities are applied before execution
//! - System prompt sections use XML tags for clear boundaries between components.
//!   This follows Anthropic's recommendation for multi-component prompts and reduces
//!   misattribution between capability instructions, user-provided AGENTS.md, and the
//!   agent's base system prompt. See knowledge/project/xml-prompt-formatting.md for rationale.
//!
//! Each capability is in its own file with collocated tools.

use crate::capability_mcp_server::capability_mcp_servers_to_scoped;
use crate::events::TokenUsage;
use crate::mcp_server::{ScopedMcpServers, merge_scoped_mcp_servers};
use crate::message::Message;
use crate::message_filter::MessageFilterProvider;
use crate::runtime_agent::RuntimeAgent;
use crate::tool_types::ToolDefinition;
use crate::tools::{Tool, ToolRegistry};
use crate::typed_id::SessionId;
use everruns_capability::is_plugin_capability;
use std::collections::HashMap;
use std::sync::Arc;

use super::*;

/// Context available to capability-owned model-view transforms.
pub struct ModelViewContext<'a> {
    pub session_id: SessionId,
    pub prior_usage: Option<&'a TokenUsage>,
}

/// Provider-side hook for building prompt-facing model views.
///
/// Providers receive the output of earlier providers and return the messages
/// that should be sent into provider serialization. Lower priority providers
/// run earlier.
pub trait ModelViewProvider: Send + Sync {
    fn apply_model_view(
        &self,
        messages: Vec<Message>,
        config: &serde_json::Value,
        context: &ModelViewContext<'_>,
    ) -> Vec<Message>;

    fn priority(&self) -> i32 {
        0
    }
}

/// Collected data from capabilities before applying to config.
///
/// This intermediate struct allows sharing the capability collection logic
/// between `apply_capabilities` and `apply_capabilities_to_builder`.
pub struct CollectedCapabilities {
    /// System prompt additions (in order)
    pub system_prompt_parts: Vec<String>,
    /// Source attribution for each system prompt addition.
    pub system_prompt_attributions: Vec<SystemPromptAttribution>,
    /// Conversation-context additions (in order). Unlike system prompt parts,
    /// these render as the leading user-role message of every turn:
    /// model-visible and re-resolved alongside the system prompt, but never
    /// folded into the cached system prompt. Untrusted workspace content
    /// (e.g. AGENTS.md hierarchies) belongs here, below harness safety
    /// instructions in the instruction hierarchy.
    pub conversation_context_parts: Vec<String>,
    /// Source attribution for each conversation-context addition.
    pub conversation_context_attributions: Vec<SystemPromptAttribution>,
    /// Tool implementations for the registry
    pub tools: Vec<Box<dyn Tool>>,
    /// Tool definitions for config
    pub tool_definitions: Vec<ToolDefinition>,
    /// Mount points from capabilities
    pub mounts: Vec<MountPoint>,
    /// Message filter providers with their configs (in priority order)
    pub message_filter_providers: Vec<(Arc<dyn MessageFilterProvider>, serde_json::Value)>,
    /// IDs of capabilities that were collected
    pub applied_ids: Vec<String>,
    /// Tool search configuration (set when openai_tool_search capability is present)
    pub tool_search: Option<crate::driver_registry::ToolSearchConfig>,
    /// Prompt caching configuration (set when prompt_caching capability is present)
    pub prompt_cache: Option<crate::driver_registry::PromptCacheConfig>,
    /// Driver-namespaced opaque per-call options (e.g. provider-executed
    /// server tools contributed by the `openrouter_server_tools` capability).
    /// First contributor wins per key.
    pub driver_options: HashMap<String, serde_json::Value>,
    /// Request-level parallel tool calls preference (set when the
    /// `parallel_tool_calls` capability is present with mode `prefer`/`avoid`).
    /// `None` when absent or mode `none`.
    pub parallel_tool_calls: Option<bool>,
    /// Hooks that transform the final runtime tool definition list.
    pub tool_definition_hooks: Vec<Arc<dyn ToolDefinitionHook>>,
    /// Hooks that inspect or transform model-produced tool calls.
    pub tool_call_hooks: Vec<Arc<dyn ToolCallHook>>,
    /// Scoped remote MCP servers contributed by capabilities.
    pub mcp_servers: ScopedMcpServers,
    // NOTE: output guardrails are intentionally NOT collected here. They are
    // re-derived per turn in `ReasonAtom` directly from the resolved capability
    // configs + registry, because they need the assembled system prompt at
    // arming time (which only exists once the runtime agent is built). Storing
    // them here would duplicate that work for callers that don't run a stream.
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemPromptAttribution {
    pub capability_id: String,
    pub content: String,
}

impl CollectedCapabilities {
    /// Returns the combined system prompt prefix from all capabilities.
    /// Returns None if no capabilities contributed system prompt additions.
    pub fn system_prompt_prefix(&self) -> Option<String> {
        if self.system_prompt_parts.is_empty() {
            None
        } else {
            Some(self.system_prompt_parts.join("\n\n"))
        }
    }

    /// Combined conversation context from all capabilities (joined with blank
    /// lines), or `None` when no capability contributed any. Renders as the
    /// leading user-role message of every turn, never as system prompt.
    pub fn conversation_context(&self) -> Option<String> {
        if self.conversation_context_parts.is_empty() {
            None
        } else {
            Some(self.conversation_context_parts.join("\n\n"))
        }
    }

    /// Apply all collected message filter providers to a query.
    ///
    /// Providers are applied in priority order (lower priority first).
    pub fn apply_message_filters(&self, query: &mut crate::message_filter::MessageQuery) {
        // Providers are already sorted by priority during collection
        for (provider, config) in &self.message_filter_providers {
            provider.apply_filters(query, config);
        }
    }

    /// Apply post-load transforms from all message filter providers.
    /// Called after messages are loaded, filtered, and injected.
    pub fn apply_post_load_filters(&self, messages: &mut Vec<crate::message::Message>) {
        for (provider, config) in &self.message_filter_providers {
            provider.post_load(messages, config);
        }
    }

    /// Check if any capabilities contribute message filters.
    pub fn has_message_filters(&self) -> bool {
        !self.message_filter_providers.is_empty()
    }
}

/// Compose the model-visible system prompt from the stable base prompt and
/// collected capability contributions. Keep the base prompt first so changes in
/// dynamic capabilities (for example AGENTS.md reads or environment context)
/// do not invalidate provider prefix caches for the agent's core instructions.
pub fn compose_system_prompt(base_system_prompt: &str, additions: Option<&str>) -> String {
    let Some(additions) = additions.filter(|value| !value.is_empty()) else {
        return base_system_prompt.to_string();
    };

    if base_system_prompt.is_empty() {
        return additions.to_string();
    }

    if base_system_prompt.contains("<system-prompt>") {
        format!("{base_system_prompt}\n\n{additions}")
    } else {
        format!("<system-prompt>\n{base_system_prompt}\n</system-prompt>\n\n{additions}")
    }
}

/// Lightweight result containing only message filter providers.
///
/// Used when callers only need message filtering (e.g., message loading in
/// ReasonAtom) without paying the cost of system prompt contribution or tool
/// collection. This avoids unnecessary filesystem reads (AGENTS.md) and tool
/// instantiation on the message-filter-only path.
pub struct CollectedMessageFilters {
    /// Message filter providers with their configs (in priority order)
    pub message_filter_providers: Vec<(Arc<dyn MessageFilterProvider>, serde_json::Value)>,
}

/// Lightweight result containing only model-view providers.
pub struct CollectedModelViewProviders {
    /// Model-view providers with their configs (in priority order).
    pub model_view_providers: Vec<(Arc<dyn ModelViewProvider>, serde_json::Value)>,
}

// Note: apply_message_filters/apply_post_load_filters mirror the same methods
// on CollectedCapabilities. The duplication is intentional — extracting a trait
// would add indirection for 3 lines of loop body, and the two structs serve
// different purposes (lightweight vs full collection).

impl CollectedMessageFilters {
    /// Apply all collected message filter providers to a query.
    pub fn apply_message_filters(&self, query: &mut crate::message_filter::MessageQuery) {
        for (provider, config) in &self.message_filter_providers {
            provider.apply_filters(query, config);
        }
    }

    /// Apply post-load transforms from all message filter providers.
    pub fn apply_post_load_filters(&self, messages: &mut Vec<crate::message::Message>) {
        for (provider, config) in &self.message_filter_providers {
            provider.post_load(messages, config);
        }
    }
}

impl CollectedModelViewProviders {
    /// Apply all collected model-view providers in priority order.
    pub fn apply_model_view(
        &self,
        mut messages: Vec<Message>,
        context: &ModelViewContext<'_>,
    ) -> Vec<Message> {
        for (provider, config) in &self.model_view_providers {
            messages = provider.apply_model_view(messages, config, context);
        }
        messages
    }
}

/// True when an available capability contributes compaction policy in this set.
///
/// Infinity context defers token-budget eviction to compaction when both are
/// enabled (see knowledge/runtime-resources/infinity-context.md) so that compaction's summary — not a
/// bare "hidden" notice — covers trimmed history.
pub(crate) fn compaction_is_enabled(
    capability_configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
) -> bool {
    capability_configs.iter().any(|cap_config| {
        registry.get(cap_config.capability_id()).is_some_and(|cap| {
            cap.status().is_active() && cap.compaction_policy(cap_config.config_value()).is_some()
        })
    })
}

/// Collect only message filter providers from capabilities, skipping system
/// prompt contributions, tools, mounts, and other expensive work.
///
/// This is a fast path for callers that only need message filtering (e.g.,
/// the message-loading step in ReasonAtom before RuntimeAgent is built).
pub fn collect_message_filters_only(
    capability_configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
) -> CollectedMessageFilters {
    let mut message_filter_providers: Vec<(Arc<dyn MessageFilterProvider>, serde_json::Value)> =
        Vec::new();
    let compaction_on = compaction_is_enabled(capability_configs, registry);

    for cap_config in capability_configs {
        let cap_id = cap_config.capability_id();
        if let Some(capability) = registry.get(cap_id) {
            if !capability.status().is_active() {
                continue;
            }
            // Resolve against None: no model is known at message-filter collection
            // time, so fall back to the model-agnostic variant if present.
            let effective: &dyn Capability = capability
                .resolve_for_model(None)
                .unwrap_or_else(|| capability.as_ref());
            if let Some(provider) = effective.message_filter_provider() {
                let config =
                    effective.message_filter_config(cap_config.config_value(), compaction_on);
                message_filter_providers.push((provider, config));
            }
        }
    }

    message_filter_providers.sort_by_key(|(p, _)| p.priority());

    CollectedMessageFilters {
        message_filter_providers,
    }
}

/// Collect only model-view providers from capabilities.
///
/// `model` should be the LLM model name when it is known at call time (e.g. the
/// ReasonAtom already holds a resolved model execution). Pass `None` only when the
/// model is genuinely unavailable so capabilities fall back to the model-agnostic
/// variant.
pub fn collect_model_view_providers(
    capability_configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
    model: Option<&str>,
) -> CollectedModelViewProviders {
    let mut model_view_providers: Vec<(Arc<dyn ModelViewProvider>, serde_json::Value)> = Vec::new();

    for cap_config in capability_configs {
        let cap_id = cap_config.capability_id();
        if let Some(capability) = registry.get(cap_id) {
            if !capability.status().is_active() {
                continue;
            }
            let effective: &dyn Capability = capability
                .resolve_for_model(model)
                .unwrap_or_else(|| capability.as_ref());
            if let Some(provider) = effective.model_view_provider() {
                model_view_providers.push((provider, cap_config.config_value().clone()));
            }
        }
    }

    model_view_providers.sort_by_key(|(p, _)| p.priority());

    CollectedModelViewProviders {
        model_view_providers,
    }
}

/// Collect [`Volatility::Dynamic`] facts from every active capability, in
/// configured order. Called by `ReasonAtom` once per request so live values
/// (e.g. the current time) are fresh, then rendered into the trailing `<facts>`
/// block. Static facts are ignored here — they already live in the cached
/// system prompt.
pub fn collect_dynamic_facts(
    capability_configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
    model: Option<&str>,
    ctx: &FactsContext,
) -> Vec<Fact> {
    let mut dynamic = Vec::new();
    for cap_config in capability_configs {
        let cap_id = cap_config.capability_id();
        if let Some(capability) = registry.get(cap_id) {
            if !capability.status().is_active() {
                continue;
            }
            let effective: &dyn Capability = capability
                .resolve_for_model(model)
                .unwrap_or_else(|| capability.as_ref());
            for fact in effective.facts(cap_config.config_value(), ctx) {
                if fact.volatility == Volatility::Dynamic {
                    dynamic.push(fact);
                }
            }
        }
    }
    dynamic
}

pub fn collect_capability_mcp_servers(
    capability_configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
) -> ScopedMcpServers {
    let mut servers = ScopedMcpServers::default();

    for cap_config in capability_configs {
        let cap_id = cap_config.capability_id();
        // Both `declarative:` and `plugin:` carry a serialized
        // `DeclarativeCapabilityDefinition`; handle them the same way.
        if is_declarative_capability(cap_id) || is_plugin_capability(cap_id) {
            if let Ok(definition) = serde_json::from_value::<DeclarativeCapabilityDefinition>(
                cap_config.config_value().clone(),
            ) {
                if !definition.status.is_active() {
                    continue;
                }
                if let Some(contributed) = definition.mcp_servers {
                    servers = merge_scoped_mcp_servers(
                        &servers,
                        &capability_mcp_servers_to_scoped(&contributed),
                    );
                }
            }
            continue;
        }
        if let Some(capability) = registry.get(cap_id) {
            if !capability.status().is_active() {
                continue;
            }
            let contributed = capability.mcp_servers_with_config(cap_config.config_value());
            servers =
                merge_scoped_mcp_servers(&servers, &capability_mcp_servers_to_scoped(&contributed));
        }
    }

    servers
}

// ============================================================================
// Dependency Resolution
// ============================================================================

/// Collect contributions from capabilities without applying them.
///
/// Resolves dependencies first, then calls `system_prompt_contribution()` (async)
/// on each capability, enabling dynamic content generation based on session context
/// (e.g., reading AGENTS.md, discovering skills).
///
/// Note: This function does not collect message filter providers since it doesn't
/// have access to per-agent capability configs. Use `collect_capabilities_with_configs`
/// if you need message filter providers.
///
/// # Arguments
///
/// * `capability_ids` - Ordered list of capability IDs to collect
/// * `registry` - The capability registry containing implementations
/// * `ctx` - Session context for dynamic prompt resolution
pub async fn collect_capabilities(
    capability_ids: &[String],
    registry: &CapabilityRegistry,
    ctx: &SystemPromptContext,
) -> CollectedCapabilities {
    // Resolve dependencies so that transitive capabilities (e.g. session_storage
    // via browserless) are included automatically.
    let resolved_ids = match resolve_dependencies(capability_ids, registry) {
        Ok(resolved) => resolved.resolved_ids,
        Err(e) => {
            tracing::warn!("Failed to resolve capability dependencies: {}", e);
            capability_ids.to_vec()
        }
    };

    // Convert to AgentCapabilityConfig with empty configs
    let configs: Vec<AgentCapabilityConfig> = resolved_ids
        .iter()
        .map(|id| {
            AgentCapabilityConfig::with_config(
                CapabilityId::new(id),
                serde_json::Value::Object(serde_json::Map::new()),
            )
        })
        .collect();

    collect_capabilities_with_configs(&configs, registry, ctx).await
}

/// Collect contributions from capabilities with their per-agent configurations.
///
/// Calls `system_prompt_contribution()` (async) on each capability, enabling
/// dynamic content generation based on session context.
///
/// # Arguments
///
/// * `capability_configs` - Ordered list of capability configs (ID + per-agent config)
/// * `registry` - The capability registry containing implementations
/// * `ctx` - Session context for dynamic prompt resolution
pub async fn collect_capabilities_with_configs(
    capability_configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
    ctx: &SystemPromptContext,
) -> CollectedCapabilities {
    let mut system_prompt_parts: Vec<String> = Vec::new();
    let mut system_prompt_attributions: Vec<SystemPromptAttribution> = Vec::new();
    let mut conversation_context_parts: Vec<String> = Vec::new();
    let mut conversation_context_attributions: Vec<SystemPromptAttribution> = Vec::new();
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut tool_definitions: Vec<ToolDefinition> = Vec::new();
    let mut mounts: Vec<MountPoint> = Vec::new();
    let mut message_filter_providers: Vec<(Arc<dyn MessageFilterProvider>, serde_json::Value)> =
        Vec::new();
    let mut applied_ids: Vec<String> = Vec::new();
    let mut tool_search: Option<crate::driver_registry::ToolSearchConfig> = None;
    let mut prompt_cache: Option<crate::driver_registry::PromptCacheConfig> = None;
    let mut driver_options: HashMap<String, serde_json::Value> = HashMap::new();
    let mut parallel_tool_calls: Option<bool> = None;
    let mut tool_definition_hooks: Vec<Arc<dyn ToolDefinitionHook>> = Vec::new();
    let mut tool_call_hooks: Vec<Arc<dyn ToolCallHook>> = Vec::new();
    // Per-capability narration adapters, appended after explicit tool-call
    // hooks so model-authored narration (human_intent) keeps precedence.
    let mut narration_hooks: Vec<Arc<dyn ToolCallHook>> = Vec::new();
    let mut mcp_servers = ScopedMcpServers::default();
    // Facts contributed by capabilities. Static facts fold into the cached
    // system prompt below; a single note is added when any dynamic fact exists,
    // explaining the live `<facts>` block that `ReasonAtom` appends per turn.
    let mut static_facts: Vec<Fact> = Vec::new();
    let mut has_dynamic_facts = false;
    let facts_ctx = FactsContext::new(ctx.session_id);
    let compaction_on = compaction_is_enabled(capability_configs, registry);
    let mut delegation_targets: Vec<DelegationTargetProvider> = Vec::new();

    for cap_config in capability_configs {
        let cap_id = cap_config.capability_id();
        // `declarative:` and `plugin:` refs both carry a serialized
        // `DeclarativeCapabilityDefinition` in their config and execute through
        // the same runtime path. `plugin:` is handled first (more specific
        // prefix), then `declarative:`, then the registry lookup.
        if is_declarative_capability(cap_id) || is_plugin_capability(cap_id) {
            match serde_json::from_value::<DeclarativeCapabilityDefinition>(
                cap_config.config_value().clone(),
            ) {
                Ok(definition) => {
                    if !definition.status.is_active() {
                        continue;
                    }

                    if let Some(prompt) = definition.system_prompt.as_deref() {
                        let contribution =
                            format!("<capability id=\"{}\">\n{}\n</capability>", cap_id, prompt);
                        system_prompt_attributions.push(SystemPromptAttribution {
                            capability_id: cap_id.to_string(),
                            content: contribution.clone(),
                        });
                        system_prompt_parts.push(contribution);
                    }

                    mounts.extend(definition.mounts(cap_id));
                    if let Some(ref servers) = definition.mcp_servers {
                        mcp_servers = merge_scoped_mcp_servers(
                            &mcp_servers,
                            &capability_mcp_servers_to_scoped(servers),
                        );
                    }
                    for skill in definition.skill_contributions() {
                        mounts.push(skill.to_mount(cap_id));
                    }

                    applied_ids.push(cap_id.to_string());
                }
                Err(error) => {
                    tracing::warn!(
                        capability_id = %cap_id,
                        error = %error,
                        "Skipping invalid declarative/plugin capability config"
                    );
                }
            }
            continue;
        }
        if let Some(capability) = registry.get(cap_id) {
            // Skip inert capabilities: `ComingSoon` is not implemented yet and
            // `Retired` has been removed. Both resolve to a no-op rather than an
            // error so an agent that still references one keeps running.
            if !capability.status().is_active() {
                continue;
            }

            // Model-adaptive dispatch: a capability may delegate its contributions
            // to a different underlying capability based on the agent's model
            // (e.g. `auto_tool_search` picks hosted vs client-side tool search).
            // Every contribution below is collected from `effective` (system prompt,
            // tools, hooks, tool definitions, mounts, MCP servers, skills, message
            // filters); for the common non-delegating case `effective` is just
            // `capability`. Driver preferences are also contributed through the
            // effective implementation's neutral trait methods, so a resolved
            // `auto_tool_search` behaves as whichever mechanism it became.
            // Attribution stays on the configured `cap_id`/`capability` so tools
            // surface under the capability the user actually configured.
            let effective: &dyn Capability =
                match capability.resolve_for_model(ctx.model.as_deref()) {
                    Some(inner) => inner,
                    None => capability.as_ref(),
                };
            let delegation_target =
                effective.delegation_target_with_config(cap_config.config_value());

            // Collect dynamic system prompt contribution (config-aware, may read from filesystem)
            if let Some(contribution) = effective
                .system_prompt_contribution_with_config(ctx, cap_config.config_value())
                .await
            {
                system_prompt_attributions.push(SystemPromptAttribution {
                    capability_id: cap_id.to_string(),
                    content: contribution.clone(),
                });
                system_prompt_parts.push(contribution);
            }

            // Collect conversation-context contribution (config-aware, may read
            // from filesystem). Renders as the leading user-role message of
            // every turn, never as system prompt, so untrusted workspace
            // content cannot share privilege with harness safety instructions.
            if let Some(contribution) = effective
                .conversation_context_contribution_with_config(ctx, cap_config.config_value())
                .await
            {
                conversation_context_attributions.push(SystemPromptAttribution {
                    capability_id: cap_id.to_string(),
                    content: contribution.clone(),
                });
                conversation_context_parts.push(contribution);
            }

            // Collect declared facts. Static facts fold into the cached prompt
            // below; dynamic facts are re-collected per request by `ReasonAtom`
            // and appended at the conversation tail, so here we only note their
            // presence to add the explanatory system-prompt line.
            for fact in effective.facts(cap_config.config_value(), &facts_ctx) {
                match fact.volatility {
                    Volatility::Static => static_facts.push(fact),
                    Volatility::Dynamic => has_dynamic_facts = true,
                }
            }

            // Collect tools and hooks (config-aware: capabilities can adapt based on per-agent config)
            tools.extend(effective.tools_with_config(cap_config.config_value()));
            if let Some(target) = delegation_target {
                delegation_targets.push(target);
            }
            tool_definition_hooks.extend(
                effective.tool_definition_hooks_with_context(ctx, cap_config.config_value()),
            );
            tool_call_hooks.extend(effective.tool_call_hooks());
            // Route this capability's `narrate()` through the hook channel.
            narration_hooks.push(Arc::new(CapabilityNarrationHook(capability.clone())));
            // Output guardrails are NOT collected here — see CollectedCapabilities
            // for rationale. ReasonAtom re-derives them at stream-arming time.

            // Collect tool definitions, propagating capability category if not already set
            let cap_category = effective.category();
            for def in effective.tool_definitions() {
                let def = match (def.category(), cap_category) {
                    (None, Some(cat)) => def.with_category(cat),
                    _ => def,
                }
                .with_capability_attribution(cap_id, Some(capability.name()));
                tool_definitions.push(def);
            }

            tool_search = effective
                .tool_search_config(cap_config.config_value())
                .or(tool_search);
            prompt_cache = effective
                .prompt_cache_config(cap_config.config_value())
                .or(prompt_cache);
            parallel_tool_calls = effective
                .parallel_tool_calls_preference(cap_config.config_value())
                .or(parallel_tool_calls);

            for (key, value) in effective.driver_options(cap_config.config_value()) {
                driver_options.entry(key).or_insert(value);
            }

            // Collect mount points
            mounts.extend(effective.mounts());

            let contributed = effective.mcp_servers_with_config(cap_config.config_value());
            mcp_servers = merge_scoped_mcp_servers(
                &mcp_servers,
                &capability_mcp_servers_to_scoped(&contributed),
            );

            // Normalize capability-contributed skills into mount points under
            // `/.agents/skills/{name}/`. Discovery/activation stays with the
            // built-in `skills` capability — see knowledge/project/skills-registry.md.
            for skill in effective.contribute_skills() {
                mounts.push(skill.to_mount(cap_id));
            }

            // Collect message filter provider
            if let Some(provider) = effective.message_filter_provider() {
                let config =
                    effective.message_filter_config(cap_config.config_value(), compaction_on);
                message_filter_providers.push((provider, config));
            }

            applied_ids.push(cap_id.to_string());
        }
    }

    // Delegation providers share one model-facing `spawn_agent` dispatcher.
    // Unknown tools with that name still win to preserve their contract.
    if !tools.iter().any(|tool| tool.name() == "spawn_agent") && !delegation_targets.is_empty() {
        let tool = UnifiedSpawnAgentTool::new(delegation_targets);
        let def = tool
            .to_definition()
            .with_category("Orchestration")
            .with_capability_attribution("agent_delegation", Some("Agent Delegation"));
        tools.push(Box::new(tool));
        tool_definitions.push(def);
    }

    // Auto-activated adapters are selected through a neutral capability hook;
    // core does not name or own the hosted implementation.
    let auto_activated: Vec<_> = registry
        .list()
        .into_iter()
        .filter(|cap| {
            !applied_ids.iter().any(|id| id == cap.id())
                && cap.status().is_active()
                && cap.auto_activates_for(&tool_definitions)
        })
        .cloned()
        .collect();
    for cap in auto_activated {
        tools.extend(cap.tools());
        let cap_category = cap.category();
        for def in cap.tool_definitions() {
            let def = match (def.category(), cap_category) {
                (None, Some(cat)) => def.with_category(cat),
                _ => def,
            }
            .with_capability_attribution(cap.id(), Some(cap.name()));
            tool_definitions.push(def);
        }
        narration_hooks.push(Arc::new(CapabilityNarrationHook(cap.clone())));
        applied_ids.push(cap.id().to_string());
    }

    // Fold static facts into the cached system-prompt prefix, and add the
    // dynamic-facts note once when any capability declared a dynamic fact. Both
    // are stable across turns, so they stay in the cached prefix; the live
    // dynamic values are appended at the conversation tail per request.
    if let Some(block) = facts::render_facts_block(&static_facts) {
        system_prompt_attributions.push(SystemPromptAttribution {
            capability_id: "facts".to_string(),
            content: block.clone(),
        });
        system_prompt_parts.push(block);
    }
    if has_dynamic_facts {
        system_prompt_attributions.push(SystemPromptAttribution {
            capability_id: "facts".to_string(),
            content: FACTS_DYNAMIC_NOTE.to_string(),
        });
        system_prompt_parts.push(FACTS_DYNAMIC_NOTE.to_string());
    }

    // Append per-capability narration adapters after every explicit tool-call
    // hook so capability-owned narration is consulted only once model-authored
    // hooks (human_intent) have had their say.
    tool_call_hooks.extend(narration_hooks);

    // Sort message filter providers by priority (lower = earlier)
    message_filter_providers.sort_by_key(|(p, _)| p.priority());

    CollectedCapabilities {
        system_prompt_parts,
        system_prompt_attributions,
        conversation_context_parts,
        conversation_context_attributions,
        tools,
        tool_definitions,
        mounts,
        message_filter_providers,
        applied_ids,
        tool_search,
        prompt_cache,
        driver_options,
        parallel_tool_calls,
        tool_definition_hooks,
        tool_call_hooks,
        mcp_servers,
    }
}

// ============================================================================
// Apply Capabilities to RuntimeAgent
// ============================================================================

/// Result of applying capabilities to a base runtime agent
pub struct AppliedCapabilities {
    /// The modified runtime agent with capability contributions merged
    pub runtime_agent: RuntimeAgent,
    /// Tool registry containing all capability tools
    pub tool_registry: ToolRegistry,
    /// IDs of capabilities that were applied
    pub applied_ids: Vec<String>,
}

/// Apply capabilities to a base runtime agent configuration.
///
/// This function:
/// 1. Collects system prompt contributions from capabilities (in order)
/// 2. Appends them after the agent's base system prompt
/// 3. Collects all tools from capabilities
/// 4. Returns the modified runtime agent and a tool registry
///
/// # Arguments
///
/// * `base_runtime_agent` - The agent's base runtime configuration
/// * `capability_ids` - Ordered list of capability IDs to apply
/// * `registry` - The capability registry containing implementations
/// * `ctx` - Session context for dynamic prompt resolution
///
/// # Returns
///
/// An `AppliedCapabilities` struct containing the modified runtime agent,
/// tool registry, and list of applied capability IDs.
///
/// # Example
///
/// ```ignore
/// use everruns_core::capabilities::{apply_capabilities, CapabilityRegistry, SystemPromptContext};
/// use everruns_core::runtime_agent::RuntimeAgent;
///
/// let registry = CapabilityRegistry::new();
/// let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");
/// let ctx = SystemPromptContext::without_file_store(SessionId::new());
///
/// let capability_ids = Vec::new();
/// let applied = apply_capabilities(base_runtime_agent, &capability_ids, &registry, &ctx).await;
///
/// assert!(applied.applied_ids.is_empty());
/// ```
pub async fn apply_capabilities(
    base_runtime_agent: RuntimeAgent,
    capability_ids: &[String],
    registry: &CapabilityRegistry,
    ctx: &SystemPromptContext,
) -> AppliedCapabilities {
    let collected = collect_capabilities(capability_ids, registry, ctx).await;

    // Build final system prompt: base prompt first, then capability additions.
    let final_system_prompt = compose_system_prompt(
        &base_runtime_agent.system_prompt,
        collected.system_prompt_prefix().as_deref(),
    );

    // Conversation context (e.g. hierarchical AGENTS.md) renders as the
    // leading user-role message, never as system prompt. Bound before fields
    // move out of `collected` below.
    let conversation_context = collected.conversation_context();
    // Build tool registry from collected tools
    let mut tool_registry = ToolRegistry::new();
    for tool in collected.tools {
        tool_registry.register_boxed(tool);
    }

    // Create modified runtime agent
    let mut tools = collected.tool_definitions;
    for hook in &collected.tool_definition_hooks {
        tools = hook.transform(tools);
    }

    let runtime_agent = RuntimeAgent {
        system_prompt: final_system_prompt,
        model: base_runtime_agent.model,
        tools,
        max_iterations: base_runtime_agent.max_iterations,
        temperature: base_runtime_agent.temperature,
        max_tokens: base_runtime_agent.max_tokens,
        tool_search: collected.tool_search,
        prompt_cache: collected.prompt_cache,
        driver_options: collected.driver_options,
        network_access: base_runtime_agent.network_access,
        // Explicit request-level preference (escape hatch) wins; otherwise the
        // `parallel_tool_calls` capability supplies the preference.
        parallel_tool_calls: base_runtime_agent
            .parallel_tool_calls
            .or(collected.parallel_tool_calls),
        // Conversation context (e.g. hierarchical AGENTS.md) renders as the
        // leading user-role message, never as system prompt.
        conversation_context,
    };

    AppliedCapabilities {
        runtime_agent,
        tool_registry,
        applied_ids: collected.applied_ids,
    }
}

// ============================================================================
// Tests
// ============================================================================
