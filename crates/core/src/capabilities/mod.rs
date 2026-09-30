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

use crate::capability_mcp_server::CapabilityMcpServers;
use crate::command::{
    CommandDescriptor, CommandExecutionContext, CommandResult, ExecuteCommandRequest,
};
use crate::message_filter::MessageFilterProvider;
use crate::session_files::SessionFileSystem;
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::tools::Tool;
use crate::typed_id::SessionId;
use async_trait::async_trait;
use std::sync::Arc;

// Split out of one 5200-line file; every item keeps its visibility, so the module's public surface is unchanged.
mod blueprint;
mod collect;
mod dependencies;
mod registry;
mod spawn_agent;

pub use blueprint::*;
pub use collect::*;
pub use dependencies::*;
pub use registry::*;
pub use spawn_agent::*;

/// Descriptor an integration crate publishes for one of its capabilities.
///
/// Decision: integration crates expose these as plain `const` slices and a
/// catalog crate names every one it composes. The registration used to happen
/// through `inventory::submit!`, which made presence in a registry a linker
/// side effect: a forgotten `extern crate` in a binary dropped an integration
/// with no compile error, and the same registry differed between binaries
/// depending on what happened to be linked. A named list costs one line per
/// integration and gets the compiler to check it.
///
/// Host or product composition iterates these descriptors and applies its
/// deployment-grade and feature-selection policy. Core only owns the neutral
/// registration contract.
///
/// # Example
///
/// ```ignore
/// // In integrations/daytona/src/lib.rs:
/// pub const CAPABILITY_PLUGINS: &[everruns_core::capabilities::IntegrationPlugin] =
///     &[everruns_core::capabilities::IntegrationPlugin {
///         experimental_only: false,
///         feature_flag: None,
///         factory: || Box::new(DaytonaCapability),
///     }];
/// ```
pub struct IntegrationPlugin {
    /// If true, product composition registers this only for experimental grades.
    pub experimental_only: bool,
    /// If set, only registered when the named deployment feature flag is enabled.
    /// Resolved at registry build time via `ExecutionFeatureDecisions`: internal
    /// infrastructure flags first, otherwise the explicit `FEATURE_<NAME>` env
    /// var (fail-closed — no grade-based default for registration gates).
    pub feature_flag: Option<&'static str>,
    /// Factory function that creates the capability instance.
    pub factory: fn() -> Box<dyn Capability>,
}

pub use crate::capability_types::{
    CapabilityStatus, MountAccess, MountDirectoryBuilder, MountEntry, MountPoint, MountSource,
};
use everruns_capability::{CapabilityId, CapabilityRef as AgentCapabilityConfig};

// ============================================================================
// Capability contract modules
// ============================================================================

mod declarative;
pub mod facts;
pub mod skill_contribution;
pub mod util;

// Re-export capabilities
/// Capability ID for outbound A2A agent delegation. Defined ungated so session
/// attachment logic can reference it even when the `a2a` feature (and the
/// delegation implementation) is compiled out.
pub const A2A_AGENT_DELEGATION_CAPABILITY_ID: &str = "a2a_agent_delegation";
/// KV key prefix for A2A delegation run records. Defined ungated so the
/// session-storage internal-prefix reservation (a TM-TOOL/TM-AGENT mitigation
/// against forged attachments) holds even when the `a2a` feature is compiled out.
pub const AGENT_RUN_KEY_PREFIX: &str = "agent_run:";
/// Shared concurrency class for every provider of the model-visible
/// `spawn_agent` tool. Implementations live in host/integration crates, while
/// collection keeps the merged tool serialized through this neutral key.
pub const SPAWN_AGENT_CONCURRENCY_CLASS: &str = "spawn_agent";
pub use declarative::{
    DECLARATIVE_CAPABILITY_PREFIX, DeclarativeCapabilityDefinition, DeclarativeCapabilityFile,
    DeclarativeCapabilitySkill, DeclarativeCapabilitySkillFile, declarative_capability_id,
    declarative_capability_info, hydrate_declarative_capability_config,
    hydrate_plugin_capability_config, is_declarative_capability, parse_declarative_capability_id,
    plugin_capability_info, validate_declarative_capability_definition,
};
pub use facts::{FACTS_DYNAMIC_NOTE, Fact, FactsContext, Volatility, render_facts_block};
pub use skill_contribution::{
    MAX_SKILLS_PER_CAPABILITY, SKILL_CAPABILITY_PREFIX, SKILLS_DISCOVERY_PATH,
    SkillCapabilityIdExt, SkillContribution, SkillInstructions, SkillMeta, SkillSource,
    discover_skills_from_entries, is_skill_capability, parse_skill_capability_id,
    reconstruct_skill_md, skill_capability_id,
};

// ============================================================================
// System Prompt Context
// ============================================================================

/// Context provided to capabilities when resolving dynamic system prompt contributions.
///
/// This gives capabilities access to session-specific resources (filesystem, etc.)
/// so they can generate system prompt content at runtime rather than returning
/// only static text.
pub struct SystemPromptContext {
    /// The current session ID
    pub session_id: SessionId,
    /// Optional locale for localized prompts and tool behavior.
    pub locale: Option<String>,
    /// Optional file store for reading session files (e.g., AGENTS.md)
    pub file_store: Option<Arc<dyn SessionFileSystem>>,
    /// The model the agent will run on, when known at collection time.
    ///
    /// Enables model-adaptive capabilities (see [`Capability::resolve_for_model`],
    /// e.g. `auto_tool_search`). `None` when the model is not yet resolved; such
    /// capabilities then fall back to their provider-agnostic behavior.
    pub model: Option<String>,
    /// Optional session key/value store, for capabilities whose contribution is
    /// persisted session state rather than static text (e.g. `channel_context`
    /// reading the accumulated `ThreadContext`). `None` for callers that do not
    /// provide one; such capabilities then contribute nothing.
    pub session_storage: Option<Arc<dyn crate::session_services::SessionStorageStore>>,
}

impl SystemPromptContext {
    /// Create context with no file store (for callers that don't need filesystem access)
    pub fn without_file_store(session_id: SessionId) -> Self {
        Self {
            session_id,
            locale: None,
            file_store: None,
            model: None,
            session_storage: None,
        }
    }

    /// Set the model the agent will run on (drives model-adaptive capabilities).
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

// ============================================================================
// Capability Trait
// ============================================================================

/// Trait for implementing capabilities that extend agent functionality.
///
/// A capability can contribute:
/// - System prompt additions (appended after the agent's base system prompt)
/// - Tools (added to agent's available tools)
///
/// # System Prompt Contributions
///
/// Capabilities provide system prompt content via `system_prompt_contribution()`.
/// This async method receives a `SystemPromptContext` with access to the session
/// filesystem, allowing capabilities to generate dynamic content (e.g., reading
/// AGENTS.md or scanning for skills).
///
/// The default implementation wraps the static `system_prompt_addition()` text
/// in `<capability id="...">` XML tags. Capabilities that need dynamic content
/// override `system_prompt_contribution()` directly.
///
/// # Example
///
/// ```ignore
/// use everruns_core::capabilities::Capability;
///
/// struct CurrentTimeCapability;
///
/// impl Capability for CurrentTimeCapability {
///     fn id(&self) -> &str {
///         "current_time"
///     }
///
///     fn name(&self) -> &str {
///         "Current Time"
///     }
///
///     fn description(&self) -> &str {
///         "Provides tools to get the current date and time."
///     }
///
///     fn tools(&self) -> Vec<Box<dyn Tool>> {
///         vec![Box::new(GetCurrentTimeTool)]
///     }
/// }
/// ```
/// Localized display strings for one locale.
///
/// Base English strings stay in `name()` / `description()` / `config_schema()`;
/// localizations are additive overlays, so adding a locale never changes the
/// `Capability` trait contract for existing implementations.
#[derive(Debug, Clone)]
pub struct CapabilityLocalization {
    /// Language tag this entry applies to, lowercase (e.g. `"uk"` or `"uk-ua"`).
    pub locale: &'static str,
    /// Localized display name; `None` falls back to `name()`.
    pub name: Option<&'static str>,
    /// Localized description; `None` falls back to `description()`.
    pub description: Option<&'static str>,
    /// One-line summary of what this capability's config controls.
    ///
    /// Provide an `"en"` entry for the base locale; capabilities without
    /// config leave this `None` everywhere.
    pub config_description: Option<&'static str>,
    /// Overlay merged into `config_schema()` by clients before rendering.
    ///
    /// Mirrors JSON Schema structure (`properties` / `items` nesting); nodes
    /// carry `title`, `description`, and `enum_labels` (map from enum value
    /// to localized label, applied to `oneOf` `const`/`title` entries).
    pub config_overlay: Option<serde_json::Value>,
}

impl CapabilityLocalization {
    /// Entry with only display strings (no config).
    pub fn text(locale: &'static str, name: &'static str, description: &'static str) -> Self {
        Self {
            locale,
            name: Some(name),
            description: Some(description),
            config_description: None,
            config_overlay: None,
        }
    }
}

/// Resolve a localized field with the standard fallback chain:
/// exact tag → language family → `"en"`. Returns `None` when no entry
/// provides the field; callers fall back to the unlocalized trait values.
pub fn resolve_localized_field<T>(
    localizations: &[CapabilityLocalization],
    locale: Option<&str>,
    field: impl Fn(&CapabilityLocalization) -> Option<T>,
) -> Option<T> {
    let mut candidates: Vec<String> = Vec::new();
    if let Some(raw) = locale {
        let normalized = raw.trim().replace('_', "-").to_lowercase();
        if !normalized.is_empty() {
            if let Some((language, _)) = normalized.split_once('-') {
                let language = language.to_string();
                candidates.push(normalized);
                candidates.push(language);
            } else {
                candidates.push(normalized);
            }
        }
    }
    candidates.push("en".to_string());

    for candidate in candidates {
        let hit = localizations
            .iter()
            .find(|entry| entry.locale.eq_ignore_ascii_case(&candidate))
            .and_then(&field);
        if hit.is_some() {
            return hit;
        }
    }
    None
}

#[async_trait]
pub trait Capability: Send + Sync {
    /// Explicit native asynchronous tool selection. Providers without support
    /// retain the ordinary synchronous definitions and scheduler.
    fn native_async_tools(
        &self,
        _config: &serde_json::Value,
    ) -> Option<std::collections::BTreeMap<String, Option<serde_json::Value>>> {
        None
    }
    /// Returns the unique capability identifier as a string
    fn id(&self) -> &str;

    /// Returns legacy identifiers that resolve to this capability.
    ///
    /// Aliases exist so a capability can be renamed without breaking persisted
    /// agent configs: registry lookups (`get`, `has`) and dependency resolution
    /// treat an alias exactly like the canonical `id()`. Resolution always
    /// normalizes aliases to the canonical ID, so an alias and its canonical
    /// ID never activate the capability twice. New code must use `id()`;
    /// aliases are a compatibility surface only.
    fn aliases(&self) -> Vec<&'static str> {
        vec![]
    }

    /// Returns the display name
    fn name(&self) -> &str;

    /// Returns a description of what this capability provides
    fn description(&self) -> &str;

    /// Returns localization overlays for this capability's display strings.
    ///
    /// Include an `"en"` entry when providing `config_description` for the
    /// base locale. Lookup follows `resolve_localized_field` fallback rules.
    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![]
    }

    /// Display name resolved for `locale`; `None` or unknown locales fall
    /// back to `name()`.
    fn localized_name(&self, locale: Option<&str>) -> String {
        resolve_localized_field(&self.localizations(), locale, |entry| entry.name)
            .unwrap_or_else(|| self.name())
            .to_string()
    }

    /// Description resolved for `locale`; falls back to `description()`.
    fn localized_description(&self, locale: Option<&str>) -> String {
        resolve_localized_field(&self.localizations(), locale, |entry| entry.description)
            .unwrap_or_else(|| self.description())
            .to_string()
    }

    /// One-line human-readable summary of what this capability's config
    /// controls, resolved for `locale`. `None` when the capability exposes
    /// no per-agent config.
    fn describe_schema(&self, locale: Option<&str>) -> Option<String> {
        resolve_localized_field(&self.localizations(), locale, |entry| {
            entry.config_description
        })
        .map(str::to_string)
    }

    /// Returns the current status of this capability
    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    /// Returns the icon name for UI rendering (optional)
    fn icon(&self) -> Option<&str> {
        None
    }

    /// Returns the category for grouping in UI (optional)
    fn category(&self) -> Option<&str> {
        None
    }

    /// Host-owned annotations that core does not interpret.
    ///
    /// The typed accessors above (`category`, `status`, `is_guardrail`, …) are
    /// the vocabulary core itself reasons about. This is the escape hatch for
    /// everything a *host* wants to carry alongside a capability — a UI icon,
    /// an embedder's grouping key, deployment provenance — without adding a
    /// field to core for each one. Core reads nothing here.
    ///
    /// The schema belongs to whoever writes it. Never put credentials or other
    /// sensitive payload here: it is surfaced to clients alongside the rest of
    /// the capability descriptor.
    fn metadata(&self) -> Option<serde_json::Value> {
        None
    }

    /// Whether this capability is a guardrail — a constraint on agent
    /// behavior (content checks, tool restrictions) rather than a grant of
    /// new abilities. Structural marker for UI sections and catalog
    /// filtering; carries no runtime semantics. See knowledge/execution/guardrails.md.
    fn is_guardrail(&self) -> bool {
        false
    }

    /// Model-adaptive dispatch: delegate this capability's contributions to a
    /// different underlying capability based on the agent's model.
    ///
    /// Capability collection (which knows the model via
    /// [`SystemPromptContext::model`]) calls this and, when it returns `Some`,
    /// collects the returned capability's contributions in place of this one's.
    /// The default returns `None` (no delegation). `auto_tool_search` overrides
    /// it to pick hosted vs client-side tool search. `model` is `None` when not
    /// yet resolved; implementations should choose a safe provider-agnostic
    /// default in that case.
    fn resolve_for_model(&self, _model: Option<&str>) -> Option<&dyn Capability> {
        None
    }

    /// Returns static text to include in the agent's system prompt (optional).
    ///
    /// This is the simple sync path for capabilities with static prompts.
    /// For dynamic content that requires filesystem access, override
    /// `system_prompt_contribution()` instead.
    ///
    /// **Contract: no duplication with tool definitions.** System prompt
    /// additions must NOT repeat information already present in tool names,
    /// descriptions, or parameter schemas. Only include content that cannot
    /// be inferred from tool definitions alone:
    ///
    /// - High-level semantics (when to use which tool, behavioral guidance)
    /// - Constraints the model cannot discover from schemas (row limits,
    ///   naming rules, workspace root paths, scheduling limits)
    /// - Data layout (filesystem paths for state files)
    /// - Cross-tool relationships or ordering not evident from descriptions
    ///
    /// If every piece of information in the prompt is already covered by the
    /// tool definitions, return `None` instead.
    fn system_prompt_addition(&self) -> Option<&str> {
        None
    }

    /// Returns the system prompt contribution for this capability, with access
    /// to session context (filesystem, etc.).
    ///
    /// This is the primary method for contributing to the system prompt.
    /// The returned string is included as-is in the final prompt (the capability
    /// is responsible for its own XML wrapping).
    ///
    /// The default implementation wraps `system_prompt_addition()` in
    /// `<capability id="...">` XML tags. Capabilities with dynamic content
    /// (e.g., `agent_instructions`, `skills`) override this to read from the
    /// session filesystem.
    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        self.system_prompt_addition().map(|addition| {
            format!(
                "<capability id=\"{}\">\n{}\n</capability>",
                self.id(),
                addition
            )
        })
    }

    /// Returns a preview of the system prompt addition for UI display.
    ///
    /// For most capabilities this is identical to `system_prompt_addition()`.
    /// Capabilities with dynamic content (e.g. `agent_instructions` which reads
    /// AGENTS.md at runtime) override this to return a representative preview.
    fn system_prompt_preview(&self) -> Option<String> {
        self.system_prompt_addition().map(|s| s.to_string())
    }

    /// Returns tool implementations provided by this capability
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![]
    }

    /// Returns tool implementations configured by per-capability config.
    ///
    /// Called during capability collection with the per-agent config for this
    /// capability (from `AgentCapabilityConfig.config`). Capabilities that adapt
    /// their tools based on config override this method.
    ///
    /// Default delegates to `tools()`.
    fn tools_with_config(&self, _config: &serde_json::Value) -> Vec<Box<dyn Tool>> {
        self.tools()
    }

    /// Returns a provider for one target of the neutral `spawn_agent` router.
    ///
    /// Hosted delegation implementations use this seam so core can assemble a
    /// single model-facing tool without knowing capability IDs or product
    /// configuration. The provider tool receives the original call unchanged.
    fn delegation_target_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Option<DelegationTargetProvider> {
        None
    }

    /// Whether this capability should be activated from the tools collected so
    /// far. This generic hook supports cross-cutting adapters without teaching
    /// core their IDs or deployment ownership.
    fn auto_activates_for(&self, _tool_definitions: &[ToolDefinition]) -> bool {
        false
    }

    /// Returns system prompt contribution adapted to per-capability config.
    ///
    /// Called during capability collection. Capabilities whose system prompt
    /// content depends on config override this method.
    ///
    /// Default delegates to `system_prompt_contribution(ctx)`.
    async fn system_prompt_contribution_with_config(
        &self,
        ctx: &SystemPromptContext,
        _config: &serde_json::Value,
    ) -> Option<String> {
        self.system_prompt_contribution(ctx).await
    }

    /// User-visible conversation context contributed by this capability.
    ///
    /// Content returned here renders as the leading user-role message of every
    /// turn: model-visible and re-resolved alongside the system prompt, but
    /// never folded into the cached system prompt. This is the correct sink
    /// for untrusted workspace content (e.g. AGENTS.md hierarchies): it keeps
    /// file instructions below harness safety instructions in the instruction
    /// hierarchy and out of the cache-stable prefix.
    ///
    /// Default returns `None` (no conversation context).
    async fn conversation_context_contribution(
        &self,
        _ctx: &SystemPromptContext,
    ) -> Option<String> {
        None
    }

    /// Called during capability collection. Capabilities whose conversation
    /// context depends on config override this method.
    ///
    /// Default delegates to `conversation_context_contribution(ctx)`.
    async fn conversation_context_contribution_with_config(
        &self,
        ctx: &SystemPromptContext,
        _config: &serde_json::Value,
    ) -> Option<String> {
        self.conversation_context_contribution(ctx).await
    }

    /// Returns tool definitions for the agent config
    /// By default, converts tools() to definitions
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools().iter().map(|t| t.to_definition()).collect()
    }

    /// Returns mount points to populate in the session filesystem
    ///
    /// Mount points allow capabilities to provide files and directories
    /// that are automatically created when a session starts. This is useful
    /// for providing sample data, documentation, or configuration files.
    ///
    /// By default, returns an empty vector (no mounts).
    fn mounts(&self) -> Vec<MountPoint> {
        vec![]
    }

    /// Returns capability IDs that this capability depends on.
    ///
    /// Dependencies are automatically resolved at runtime when applying
    /// capabilities. If capability A depends on capability B, then B's
    /// contributions (tools, system prompt, mounts) will be included
    /// when A is selected, even if B is not explicitly selected.
    ///
    /// By default, returns an empty vector (no dependencies).
    fn dependencies(&self) -> Vec<&'static str> {
        vec![]
    }

    /// Returns UI feature strings that this capability contributes to.
    ///
    /// Features are open-ended strings indicating what user-facing functionality
    /// this capability enables. Multiple capabilities can contribute the same
    /// feature (e.g., both `session_schedule` and a future `signals` capability
    /// might contribute `"schedules"`).
    ///
    /// The UI uses the aggregated set of features from all active capabilities
    /// to decide which tabs/sections to render.
    ///
    /// Known features: `"file_system"`, `"schedules"`, `"secrets"`,
    /// `"key_value"`, `"sql_database"`, `"leased_resources"`.
    ///
    /// By default, returns an empty vector (no features).
    fn features(&self) -> Vec<&'static str> {
        vec![]
    }

    /// Returns the JSON Schema for this capability's per-agent config.
    ///
    /// The schema is exposed through `CapabilityInfo` so clients can render a
    /// generic settings editor for capabilities without hard-coding capability
    /// IDs. Capabilities without configurable settings return `None`.
    fn config_schema(&self) -> Option<serde_json::Value> {
        None
    }

    /// Returns UI hints for rendering `config_schema`.
    ///
    /// This follows the react-jsonschema-form `uiSchema` shape. The server owns
    /// durable config semantics; clients own the generic component implementation.
    fn config_ui_schema(&self) -> Option<serde_json::Value> {
        None
    }

    /// Validates per-capability config before it is persisted.
    ///
    /// Default accepts any config for backward compatibility. Capabilities with
    /// a `config_schema()` should reject invalid values here so HTTP, CLI, and
    /// MCP write paths share the same server-side guardrail.
    fn validate_config(&self, _config: &serde_json::Value) -> Result<(), String> {
        Ok(())
    }

    /// Returns remote MCP servers contributed by this capability.
    ///
    /// These are merged into harness/agent/session scoped MCP config at runtime.
    /// Explicit scoped MCP config overrides capability-contributed defaults by
    /// logical server name.
    fn mcp_servers(&self) -> CapabilityMcpServers {
        CapabilityMcpServers::default()
    }

    /// Returns config-aware remote MCP server contributions.
    fn mcp_servers_with_config(&self, _config: &serde_json::Value) -> CapabilityMcpServers {
        self.mcp_servers()
    }

    /// Returns a message filter provider if this capability modifies message retrieval.
    ///
    /// Capabilities can contribute filters that modify how messages are loaded
    /// from the database. This enables features like:
    /// - Time-based filtering (recent messages only)
    /// - Event type filtering
    /// - Tool result filtering by tool name
    /// - Ephemeral message injection (summaries, reminders)
    ///
    /// Filters are applied in capability priority order (by `MessageFilterProvider::priority()`).
    ///
    /// By default, returns None (no message filtering).
    fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
        None
    }

    /// Derive the config passed to this capability's message filter.
    ///
    /// `compaction_enabled` lets a filter coordinate with a separately selected
    /// compaction policy without core matching on either capability's ID.
    fn message_filter_config(
        &self,
        config: &serde_json::Value,
        _compaction_enabled: bool,
        _provider_managed_reduction: bool,
    ) -> serde_json::Value {
        config.clone()
    }

    /// Token budget contributed to a provider-managed history reducer.
    fn provider_managed_reduction_budget(&self, _config: &serde_json::Value) -> Option<usize> {
        None
    }

    /// Whether this capability can rewrite user input before provider
    /// serialization. Provider-managed replay is disabled in that case because
    /// rebuilding from raw audit history could resend removed content.
    fn requires_provider_history_rewrite(&self, config: &serde_json::Value) -> bool {
        self.user_hooks_with_config(config)
            .iter()
            .any(|hook| hook.event == crate::user_hook_types::HookEvent::UserPromptSubmit)
    }

    /// Returns a provider that can build a prompt-facing model view from
    /// lossless stored messages before provider serialization.
    ///
    /// This is for capability-owned context transformations such as compaction
    /// cost-control masking. Storage messages remain unchanged.
    ///
    /// By default, returns None (no model-view transformation).
    fn model_view_provider(&self) -> Option<Arc<dyn ModelViewProvider>> {
        None
    }

    /// Returns an in-process hook invoked when a turn fails with a *terminal*
    /// LLM error (one that will not be retried), before the user-facing error
    /// message is emitted. The hook may perform a side effect (e.g. schedule a
    /// continuation) and/or return extra fields to augment the user-facing error
    /// copy. This is the platform seam for capability-owned error recovery — the
    /// same in-process hook family as [`Self::tool_call_hooks`] and
    /// [`Self::message_filter_provider`]; the reason atom invokes it generically
    /// and knows nothing about any specific capability's behavior. See
    /// [`crate::llm_error_hook`].
    ///
    /// By default, returns None (no error hook).
    fn llm_error_hook(&self) -> Option<Arc<dyn crate::llm_error_hook::LlmErrorHook>> {
        None
    }

    /// Provider-facing deferred tool-loading configuration contributed by
    /// this capability. The execution engine consumes this generically and
    /// does not match on implementation-owned capability IDs.
    fn tool_search_config(
        &self,
        _config: &serde_json::Value,
    ) -> Option<crate::driver_registry::ToolSearchConfig> {
        None
    }

    /// Provider-facing prompt-cache configuration contributed by this
    /// capability.
    fn prompt_cache_config(
        &self,
        _config: &serde_json::Value,
    ) -> Option<crate::driver_registry::PromptCacheConfig> {
        None
    }

    /// Driver-namespaced opaque per-call options (`"<driver-id>/<option>"`)
    /// requested by this capability. Each entry's shape is owned by the driver
    /// crate named in the key; core transports the values untouched.
    fn driver_options(&self, _config: &serde_json::Value) -> Vec<(String, serde_json::Value)> {
        Vec::new()
    }

    /// Request-level parallel tool-call preference contributed by this
    /// capability. `None` leaves the runtime/provider default unchanged.
    fn parallel_tool_calls_preference(&self, _config: &serde_json::Value) -> Option<bool> {
        None
    }

    /// User-facing terminal-error disclosure selected by this capability.
    fn error_disclosure(
        &self,
        _config: &serde_json::Value,
    ) -> Option<crate::user_facing_error::ErrorDisclosure> {
        None
    }

    /// Filter assistant text before it is persisted or returned. This is a
    /// deterministic, config-aware seam for capability-owned annotations.
    fn filter_response_text(&self, text: String, _config: &serde_json::Value) -> String {
        text
    }

    /// Context-compaction policy configured by this capability. The reason
    /// atom owns orchestration and invokes the returned implementation without
    /// matching on a capability ID.
    fn compaction_policy(
        &self,
        _config: &serde_json::Value,
    ) -> Option<Arc<dyn crate::compaction_policy::CompactionPolicy>> {
        None
    }

    /// Returns key/value [`Fact`]s this capability contributes to the model.
    ///
    /// Facts are routed by their [`Volatility`] so prompt caching is preserved:
    /// [`Volatility::Static`] facts fold into the cached system-prompt prefix at
    /// build time; [`Volatility::Dynamic`] facts are appended at the
    /// conversation tail on every turn (outside the cached prefix). This is the
    /// generic seam for "changing facts" such as the current time — see
    /// [`crate::capabilities::facts`].
    ///
    /// Called both at prompt-assembly time (to fold static facts and detect
    /// whether any dynamic facts exist) and per request (to render the live
    /// tail block), so implementations must be cheap and side-effect free.
    ///
    /// By default, returns an empty vector (no facts).
    fn facts(&self, _config: &serde_json::Value, _ctx: &FactsContext) -> Vec<Fact> {
        vec![]
    }

    /// Returns pre-tool execution hooks provided by this capability.
    ///
    /// These hooks run before each individual tool is executed — for *every*
    /// tool the agent calls (built-in, MCP, or client-side), not just this
    /// capability's own tools. A hook can mutate the tool call or block it
    /// outright (returning [`crate::tool_hooks::PreToolUseDecision::Block`]), which
    /// makes this the seam for cross-cutting policy such as approval gating.
    /// The first hook to block wins.
    ///
    /// By default, returns an empty vector (no hooks).
    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn crate::tool_hooks::PreToolUseHook>> {
        vec![]
    }

    /// Returns pre-tool execution hooks adapted to per-capability config.
    ///
    /// Default delegates to `pre_tool_use_hooks()`. Capabilities whose hook
    /// behavior depends on config (e.g. `guardrails`) override this.
    fn pre_tool_use_hooks_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Vec<Arc<dyn crate::tool_hooks::PreToolUseHook>> {
        self.pre_tool_use_hooks()
    }

    /// Returns post-tool execution hooks provided by this capability.
    ///
    /// These hooks run after each individual tool completes execution.
    /// They can persist output, inject metadata, or transform results.
    /// Capability-contributed hooks run before infrastructure (final) hooks.
    ///
    /// By default, returns an empty vector (no hooks).
    fn post_tool_exec_hooks(&self) -> Vec<Arc<dyn crate::tool_hooks::PostToolExecHook>> {
        vec![]
    }

    /// Returns post-tool execution hooks adapted to per-capability config.
    ///
    /// Default delegates to `post_tool_exec_hooks()`. Capabilities whose hook
    /// behavior depends on config (e.g. `guardrails`) override this.
    fn post_tool_exec_hooks_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Vec<Arc<dyn crate::tool_hooks::PostToolExecHook>> {
        self.post_tool_exec_hooks()
    }

    /// Returns tool definition hooks provided by this capability.
    ///
    /// These hooks run after the runtime agent has merged and deduplicated its
    /// final tool list, before the tool schemas are sent to the LLM. They let
    /// capabilities apply cross-cutting schema changes to all active tools,
    /// including tools contributed by other capabilities, MCP, or clients.
    ///
    /// By default, returns an empty vector (no tool definition transforms).
    fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![]
    }

    /// Returns tool definition hooks adapted to per-capability config.
    ///
    /// Default delegates to `tool_definition_hooks()`. Capabilities whose
    /// schema transforms depend on config override this method.
    fn tool_definition_hooks_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        self.tool_definition_hooks()
    }

    /// Returns tool definition hooks adapted to per-capability config and the
    /// collection context (session id, model, ...).
    ///
    /// Default delegates to [`Self::tool_definition_hooks_with_config`], which
    /// ignores the context. Capabilities whose hooks carry session-scoped state
    /// override this to capture `ctx` — e.g. `tool_search` keys its
    /// progressive-disclosure reveal set by `ctx.session_id`, since the
    /// capability is a process-global singleton shared across sessions and a
    /// `ToolDefinitionHook::transform` has no session context of its own.
    fn tool_definition_hooks_with_context(
        &self,
        _ctx: &SystemPromptContext,
        config: &serde_json::Value,
    ) -> Vec<Arc<dyn ToolDefinitionHook>> {
        self.tool_definition_hooks_with_config(config)
    }

    /// Returns tool call hooks provided by this capability.
    ///
    /// These hooks run after the model has produced a tool call. They can read
    /// model-authored metadata for UI display and transform the tool call used
    /// for actual execution.
    ///
    /// By default, returns an empty vector (no tool call handling).
    fn tool_call_hooks(&self) -> Vec<Arc<dyn ToolCallHook>> {
        vec![]
    }

    /// Returns a configured hook over the finalized model tool-call batch.
    /// This later seam is suitable for policy that needs all calls plus their
    /// final schemas before the assistant message is persisted.
    fn finalized_tool_calls_hook(
        &self,
        _config: &serde_json::Value,
    ) -> Option<Arc<dyn crate::finalized_tool_calls::FinalizedToolCallsHook>> {
        None
    }

    /// Contribute human-readable narration for one of *this capability's* tool
    /// calls (e.g. "Read AGENTS.md", "Searched tools: router").
    ///
    /// The **default** dispatches to the matching tool's
    /// [`crate::tools::Tool::narrate`], so a capability narrates its tools for
    /// free — narration lives on the tool that owns it. Override this only when
    /// narration is config-driven or spans tools, or when the tools are dynamic
    /// (e.g. proxied MCP tools that have no local `Tool` struct).
    ///
    /// Returns `None` for tool names this capability does not provide, so other
    /// capabilities — or the generic fallback in [`crate::tool_narration`] —
    /// can handle them. The framework consults this for every applied
    /// capability (see `assemble`/`CapabilityNarrationHook`) on the act path.
    fn narrate(
        &self,
        _tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        self.tools()
            .iter()
            .find(|tool| tool.name() == tool_call.name)
            .and_then(|tool| tool.narrate(tool_call, phase, locale, ctx))
    }

    /// Returns user-defined hook specifications contributed by this capability.
    ///
    /// User hooks are JSON-serializable specs (see
    /// `crate::user_hook_types::UserHookSpec` and `knowledge/runtime-resources/user-hooks.md`) that
    /// the `HookAdapterBuilder` validates and turns into per-event
    /// `Arc<dyn …Hook>` adapters during capability collection. Capabilities
    /// that ship reusable hook bundles (formatters, security guards, audit
    /// commands) override this; the user-facing `user_hooks` capability also
    /// uses this hook to surface user-config-authored entries.
    ///
    /// Contributors return *data only* — the executor is constructed
    /// centrally by the core so global timeout/output/sandbox limits cannot
    /// be bypassed.
    ///
    /// By default, returns an empty vector (no contributed hooks).
    fn user_hooks(&self) -> Vec<crate::user_hook_types::UserHookSpec> {
        vec![]
    }

    /// Returns user-defined hook specifications adapted to per-capability
    /// config.
    ///
    /// Default delegates to `user_hooks()`. The `user_hooks` capability
    /// overrides this to parse hook entries out of its config.
    fn user_hooks_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Vec<crate::user_hook_types::UserHookSpec> {
        self.user_hooks()
    }

    /// Returns the risk level of this capability.
    ///
    /// TM-AGENT-005: High-risk capabilities (code execution, network access)
    /// require admin approval when assigned to agents/harnesses. Capabilities
    /// that combine execution + network access enable data exfiltration.
    ///
    /// By default, returns `RiskLevel::Low`.
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Low
    }

    /// Returns system commands this capability provides.
    ///
    /// System commands are user-invocable /slash commands that execute directly
    /// without involving the LLM. They are surfaced in the UI command palette
    /// alongside invocable skills.
    ///
    /// By default, returns an empty vector (no commands).
    fn commands(&self) -> Vec<CommandDescriptor> {
        vec![]
    }

    /// Execute a system command declared by [`Self::commands`].
    ///
    /// Capabilities that declare commands MUST override this. The default
    /// implementation returns an error so that misconfigurations surface at
    /// invocation time rather than silently succeeding. Capabilities should
    /// match on `request.name`, validate `request.arguments`, and use the
    /// references they captured at construction time to mutate any external
    /// state (provider store, file system, etc.).
    ///
    /// Commands that need the session's assembled context or an out-of-band
    /// LLM call (e.g. `/btw`) use the host facilities on
    /// [`CommandExecutionContext::host`] — see
    /// [`crate::command_host::CommandHost`] and knowledge/project/commands.md.
    async fn execute_command(
        &self,
        request: &ExecuteCommandRequest,
        _ctx: &CommandExecutionContext,
    ) -> crate::error::Result<CommandResult> {
        Err(crate::error::AgentLoopError::config(format!(
            "capability {} declared command /{} but does not implement execute_command",
            self.id(),
            request.name,
        )))
    }

    /// Returns agent blueprints contributed by this capability.
    ///
    /// Blueprints are pre-built agent definitions with private tools, baked-in prompts,
    /// and fixed/default models. They are spawned via `spawn_agent` with a subagent target
    /// and `blueprint`.
    /// Blueprint tools never appear in the host agent's tool list.
    ///
    /// By default, returns an empty vector (no blueprints).
    fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
        vec![]
    }

    /// Returns skills contributed by this capability in code.
    ///
    /// Contributions are normalized during capability collection into read-only
    /// mount points at `/.agents/skills/{name}/` so the built-in `skills`
    /// capability discovers them alongside user-uploaded and registry-based
    /// skills. This keeps discovery, prompt listing, and activation in one
    /// place rather than adding a parallel skill pipeline.
    ///
    /// By default, returns an empty vector (no contributed skills).
    fn contribute_skills(&self) -> Vec<SkillContribution> {
        vec![]
    }

    /// Returns streaming output guardrails contributed by this capability.
    ///
    /// Each provider is armed once per assistant message stream with the
    /// fully assembled system prompt and per-capability config; the returned
    /// per-stream `OutputGuardrailRun` is invoked after every batched delta
    /// in the streaming hot path. Returning `Block` aborts the stream and
    /// the client is told to replace the accumulated text with a canned
    /// message. See [`crate::output_guardrail`].
    ///
    /// Default: no guardrails.
    fn output_guardrails(&self) -> Vec<Arc<dyn crate::output_guardrail::OutputGuardrail>> {
        vec![]
    }

    /// Async, end-of-message output guardrails (EVE-573).
    ///
    /// Unlike [`Self::output_guardrails`] (synchronous, per-delta, hot path),
    /// these providers run **once** on the fully assembled assistant message
    /// after streaming completes and before the message is finalized into
    /// context. They receive an LLM-capable context and may perform I/O (e.g.
    /// a moderation classifier). The per-agent capability config is passed so a
    /// capability contributes nothing unless it has an applicable check
    /// configured — keeping the common (no-output-check) case free of work.
    ///
    /// Default: no guardrails.
    fn post_output_guardrails_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Vec<Arc<dyn crate::output_guardrail::PostGenerationOutputGuardrail>> {
        vec![]
    }

    /// Returns end-of-message citation annotation hooks contributed by this
    /// capability, adapted to per-agent config.
    ///
    /// Like [`Self::post_output_guardrails_with_config`], these run once on the
    /// fully assembled assistant message after streaming completes. But instead
    /// of a block/allow decision they attach citation [`crate::message::TextAnnotation`]s
    /// to the message text (optionally rewriting it first, e.g. to strip inline
    /// citation markers). This is the seam citation capabilities use to turn
    /// retrieved sources into claim-level provenance. See
    /// [`crate::annotation_hook`] and `knowledge/runtime-resources/citations.md`.
    ///
    /// A capability contributes nothing unless a citation feed is configured,
    /// keeping the common (no-citations) case free of work.
    ///
    /// Default: no annotation hooks.
    fn post_output_annotation_hooks_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Vec<Arc<dyn crate::annotation_hook::PostGenerationAnnotationHook>> {
        vec![]
    }

    /// Returns a citation verifier contributed by this capability, if any.
    ///
    /// Runs once after all citation feeds have attached annotations, over the
    /// collected set, stamping a [`crate::message::VerificationVerdict`] on each
    /// citation. Decoupled from the feeds so any feed can be paired with any
    /// verifier. The `citation_verification` capability implements this. See
    /// [`crate::annotation_hook::CitationVerifier`] and `knowledge/runtime-resources/citations.md`.
    ///
    /// Default: no verifier.
    fn citation_verifier_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Option<Arc<dyn crate::annotation_hook::CitationVerifier>> {
        None
    }
}

pub trait ToolDefinitionHook: Send + Sync {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition>;

    /// Whether this hook should still run when the agent's model uses native
    /// (hosted) tool_search. Client-side deferral hooks return `false` so they
    /// don't strip schemas the hosted tool_search index needs (the two are
    /// mutually exclusive). Defaults to `true`.
    fn applies_with_native_tool_search(&self) -> bool {
        true
    }
}

pub trait ToolCallHook: Send + Sync {
    fn narration(
        &self,
        _tool_def: Option<&ToolDefinition>,
        _tool_call: &ToolCall,
        _phase: crate::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        None
    }

    fn transform_for_execution(&self, tool_call: ToolCall) -> ToolCall {
        tool_call
    }
}

/// Adapts a [`Capability`]'s [`Capability::narrate`] into a [`ToolCallHook`] so
/// capability-owned narration flows through the same hook channel the act atom
/// already consults. One is registered per applied capability during
/// `assemble`, after every explicit tool-call hook, so model-authored
/// narration (e.g. `human_intent`) still takes precedence.
pub struct CapabilityNarrationHook(pub Arc<dyn Capability>);

impl ToolCallHook for CapabilityNarrationHook {
    fn narration(
        &self,
        tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        self.0.narrate(tool_def, tool_call, phase, locale, ctx)
    }
}

#[cfg(test)]
mod tests_collect_tests;
#[cfg(test)]
mod tests_registry_tests;
#[cfg(test)]
mod tests_support;
