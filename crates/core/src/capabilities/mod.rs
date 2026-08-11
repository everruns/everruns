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

use crate::capability_types::is_plugin_capability;
use crate::command::{
    CommandDescriptor, CommandExecutionContext, CommandResult, ExecuteCommandRequest,
};
use crate::deployment::DeploymentGrade;
use crate::events::TokenUsage;
use crate::mcp_server::{ScopedMcpServers, merge_scoped_mcp_servers};
use crate::message::Message;
use crate::message_filter::MessageFilterProvider;
use crate::runtime_agent::RuntimeAgent;
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::tools::{Tool, ToolExecutionResult, ToolRegistry};
use crate::traits::{SessionFileSystem, ToolContext};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Integration Plugin System
// ============================================================================

/// Plugin registration point for external integration crates.
///
/// Integration crates use `inventory::submit!` to register their capabilities
/// without requiring `everruns-core` to know about them at compile time.
/// The `CapabilityRegistry::with_builtins_for_grade()` method iterates all
/// registered plugins and includes those matching the current deployment grade.
///
/// # Example
///
/// ```ignore
/// // In integrations/daytona/src/lib.rs:
/// inventory::submit! {
///     everruns_core::capabilities::IntegrationPlugin {
///         experimental_only: false,
///         feature_flag: None,
///         factory: || Box::new(DaytonaCapability),
///     }
/// }
/// ```
pub struct IntegrationPlugin {
    /// If true, only registered when `DeploymentGrade::experimental_features_enabled()` is true.
    pub experimental_only: bool,
    /// If set, only registered when the named deployment feature flag is enabled.
    /// Resolved at registry build time via `ExecutionFeatureDecisions`: internal
    /// infrastructure flags first, otherwise the explicit `FEATURE_<NAME>` env
    /// var (fail-closed — no grade-based default for registration gates).
    pub feature_flag: Option<&'static str>,
    /// Factory function that creates the capability instance.
    pub factory: fn() -> Box<dyn Capability>,
}

inventory::collect!(IntegrationPlugin);

// Re-export capability types from capability_types module
pub use crate::capability_types::{
    AgentCapabilityConfig, CapabilityId, CapabilityStatus, MountAccess, MountDirectoryBuilder,
    MountEntry, MountPoint, MountSource,
};

// ============================================================================
// Capability Modules
// ============================================================================

#[cfg(feature = "a2a")]
mod a2a_delegation;
#[cfg(feature = "ui-capabilities")]
mod a2ui;
mod agent_handoff;
pub mod attach_skill;
mod background_execution;
mod citation_retrieval;
mod citation_verification;
mod data_knowledge;
mod declarative;
mod delegation_result;
pub mod facts;
mod human_intent;
mod infinity_context;
mod knowledge_base;
mod knowledge_index;
mod memory;
mod monitors;
mod openrouter_server_tools;
#[cfg(feature = "ui-capabilities")]
mod openui;
mod research;
mod session;
mod session_sandbox;
mod session_schedule;
mod session_sql_database;
mod session_storage;
mod session_tasks;
mod skills;
mod skills_scoped;
mod subagents;
mod tool_approval;
pub mod user_hooks;
pub mod util;

// Re-export capabilities
/// Capability ID for outbound A2A agent delegation. Defined ungated so session
/// attachment logic can reference it even when the `a2a` feature (and the
/// delegation implementation) is compiled out.
pub const A2A_AGENT_DELEGATION_CAPABILITY_ID: &str = "a2a_agent_delegation";
/// KV key prefix for A2A delegation run records. Defined ungated so the
/// session-storage internal-prefix reservation (a TM-TOOL/TM-AGENT mitigation
/// against forged attachments) holds even when the `a2a` feature is compiled out.
pub(crate) const AGENT_RUN_KEY_PREFIX: &str = "agent_run:";
#[cfg(feature = "a2a")]
pub use a2a_delegation::{A2aAgentDelegationCapability, SpawnAgentTool};
#[cfg(feature = "ui-capabilities")]
pub use a2ui::{A2UI_CAPABILITY_ID, A2UiCapability};
pub use agent_handoff::{
    AGENT_HANDOFF_CAPABILITY_ID, AgentHandoffCapability, SpawnAgentHandoffTool,
};
pub use attach_skill::{
    AttachSkillCapability, SKILL_CAPABILITY_PREFIX, SKILLS_DISCOVERY_PATH, SkillCapabilityIdExt,
    SkillContribution, SkillInstructions, SkillMeta, SkillSource, discover_skills_from_entries,
    is_skill_capability, parse_skill_capability_id, reconstruct_skill_md, skill_capability_id,
};
pub use background_execution::{BACKGROUND_EXECUTION_CAPABILITY_ID, BackgroundExecutionCapability};
pub use citation_retrieval::{
    CITATION_RETRIEVAL_CAPABILITY_ID, CitationRetrievalCapability, CitationRetrievalConfig,
};
pub use citation_verification::{
    CITATION_VERIFICATION_CAPABILITY_ID, CitationVerificationCapability,
    CitationVerificationConfig, VerificationMode,
};
pub use data_knowledge::{DATA_KNOWLEDGE_CAPABILITY_ID, DataKnowledgeCapability};
pub use declarative::{
    DECLARATIVE_CAPABILITY_PREFIX, DeclarativeCapabilityDefinition, DeclarativeCapabilityFile,
    DeclarativeCapabilitySkill, DeclarativeCapabilitySkillFile, declarative_capability_id,
    declarative_capability_info, hydrate_declarative_capability_config,
    hydrate_plugin_capability_config, is_declarative_capability, parse_declarative_capability_id,
    plugin_capability_info, validate_declarative_capability_definition,
};
pub use delegation_result::{
    ReportResultTool, ReportTaskProgressTool, report_result_tool_for_child_session,
    report_task_progress_tool_for_child_session,
};
pub use facts::{FACTS_DYNAMIC_NOTE, Fact, FactsContext, Volatility, render_facts_block};
pub use human_intent::{HUMAN_INTENT_CAPABILITY_ID, HumanIntentCapability};
pub use infinity_context::{
    INFINITY_CONTEXT_CAPABILITY_ID, InfinityContextCapability, InfinityContextFilterOnlyCapability,
    QueryHistoryTool,
};
pub use knowledge_base::{
    KNOWLEDGE_BASE_CAPABILITY_ID, KnowledgeBaseCapability, KnowledgeBaseConfig,
    validate_knowledge_base_config,
};
pub use knowledge_index::{
    KNOWLEDGE_INDEX_CAPABILITY_ID, KnowledgeIndexCapability, KnowledgeIndexConfig,
    validate_knowledge_index_config,
};
pub use memory::{MEMORY_CAPABILITY_ID, MemoryCapability};
pub use openrouter_server_tools::{
    OPENROUTER_SERVER_TOOLS_CAPABILITY_ID, OpenRouterServerToolsCapability,
};
#[cfg(feature = "ui-capabilities")]
pub use openui::{OPENUI_CAPABILITY_ID, OpenUiCapability};
pub use research::{RESEARCH_CAPABILITY_ID, ResearchCapability};
pub use session::{
    GetSessionInfoTool, SESSION_CAPABILITY_ID, SessionCapability, SessionCapabilityConfig,
    SessionTitleMutation, WriteSessionTitleTool, session_title_updated_event,
    update_session_title_with_event,
};
pub use session_sandbox::{
    SESSION_SANDBOX_CAPABILITY_ID, SandboxExecTool, SandboxManageTool, SandboxReadFileTool,
    SandboxStatusTool, SandboxWriteFileTool, SessionSandboxCapability,
};
pub use session_schedule::{
    CancelScheduleTool, CreateScheduleTool, ListSchedulesTool, SESSION_SCHEDULE_CAPABILITY_ID,
    SessionScheduleCapability,
};
pub use session_sql_database::{
    SESSION_SQL_DATABASE_CAPABILITY_ID, SessionSqlDatabaseCapability, SqlExecuteTool, SqlQueryTool,
    SqlSchemaTool,
};
pub use session_storage::{
    KvStoreTool, SESSION_STORAGE_CAPABILITY_ID, SecretStoreTool, SessionStorageCapability,
    is_internal_session_kv_key, is_internal_session_secret_name,
};
pub use session_tasks::{SESSION_TASKS_CAPABILITY_ID, SessionTasksCapability};
pub use skills::{SKILLS_CAPABILITY_ID, SkillsCapability};
pub use skills_scoped::{
    ScopedSkillsCapability, SkillDirResolver, SkillScope, SkillsConfig, VfsSkillDirResolver,
};
pub(crate) use subagents::SPAWN_AGENT_CONCURRENCY_CLASS;
pub use subagents::{SUBAGENTS_CAPABILITY_ID, SpawnSubagentAsAgentTool, SubagentCapability};
// Blueprint types are exported directly from the trait definitions above
pub use tool_approval::{
    ApprovalDecision, ApprovalMode, TOOL_APPROVAL_CAPABILITY_ID, ToolApprovalCapability,
    ToolApprover,
};
pub use user_hooks::{USER_HOOKS_CAPABILITY_ID, UserHooksCapability};

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
}

impl SystemPromptContext {
    /// Create context with no file store (for callers that don't need filesystem access)
    pub fn without_file_store(session_id: SessionId) -> Self {
        Self {
            session_id,
            locale: None,
            file_store: None,
            model: None,
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
    fn mcp_servers(&self) -> ScopedMcpServers {
        ScopedMcpServers::default()
    }

    /// Returns config-aware remote MCP server contributions.
    fn mcp_servers_with_config(&self, _config: &serde_json::Value) -> ScopedMcpServers {
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
    /// outright (returning [`crate::atoms::PreToolUseDecision::Block`]), which
    /// makes this the seam for cross-cutting policy such as approval gating.
    /// The first hook to block wins.
    ///
    /// By default, returns an empty vector (no hooks).
    fn pre_tool_use_hooks(&self) -> Vec<Arc<dyn crate::atoms::PreToolUseHook>> {
        vec![]
    }

    /// Returns pre-tool execution hooks adapted to per-capability config.
    ///
    /// Default delegates to `pre_tool_use_hooks()`. Capabilities whose hook
    /// behavior depends on config (e.g. `guardrails`) override this.
    fn pre_tool_use_hooks_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Vec<Arc<dyn crate::atoms::PreToolUseHook>> {
        self.pre_tool_use_hooks()
    }

    /// Returns post-tool execution hooks provided by this capability.
    ///
    /// These hooks run after each individual tool completes execution.
    /// They can persist output, inject metadata, or transform results.
    /// Capability-contributed hooks run before infrastructure (final) hooks.
    ///
    /// By default, returns an empty vector (no hooks).
    fn post_tool_exec_hooks(&self) -> Vec<Arc<dyn crate::atoms::PostToolExecHook>> {
        vec![]
    }

    /// Returns post-tool execution hooks adapted to per-capability config.
    ///
    /// Default delegates to `post_tool_exec_hooks()`. Capabilities whose hook
    /// behavior depends on config (e.g. `guardrails`) override this.
    fn post_tool_exec_hooks_with_config(
        &self,
        _config: &serde_json::Value,
    ) -> Vec<Arc<dyn crate::atoms::PostToolExecHook>> {
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

/// Risk classification for capabilities (TM-AGENT-005).
///
/// Used to enforce approval requirements when assigning capabilities.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "low"))]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    /// No special approval needed
    Low,
    /// Logged but allowed for org members
    Medium,
    /// Requires org admin role to assign
    High,
}

// ============================================================================
// Agent Blueprints
// ============================================================================

/// Model selection strategy for agent blueprints.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintModel {
    /// Always use this model. Host cannot override.
    Fixed(String),
    /// Use this model unless host provides override via config.
    Default(String),
    /// Use whatever model the host agent uses.
    Inherit,
}

/// Pre-built agent definition with private tools, baked-in prompt, and model selection.
///
/// Contributed by capabilities via `agent_blueprints()`. Spawned via
/// `spawn_agent` with a subagent target and `blueprint`. Blueprint tools never appear in the
/// host agent's tool list — they exist only inside the spawned child session.
pub struct AgentBlueprint {
    /// Unique identifier (e.g. `"github_scout"`)
    pub id: &'static str,
    /// Human-readable display name
    pub name: &'static str,
    /// When to use this blueprint (LLM reads this for delegation decisions)
    pub description: &'static str,
    /// Model selection strategy
    pub model: BlueprintModel,
    /// Baked-in system prompt for the child agent
    pub system_prompt: &'static str,
    /// Private tools — only available inside the blueprint's session
    pub tools: Vec<Box<dyn Tool>>,
    /// Iteration limit (default: 20)
    pub max_turns: Option<usize>,
    /// JSON Schema for allowed host-provided config. `None` = no config accepted.
    pub config_schema: Option<serde_json::Value>,
}

impl AgentBlueprint {
    /// Convert blueprint tools to tool definitions (for RuntimeAgent building).
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.iter().map(|t| t.to_definition()).collect()
    }
}

impl std::fmt::Debug for AgentBlueprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentBlueprint")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("model", &self.model)
            .field("tool_count", &self.tools.len())
            .field("max_turns", &self.max_turns)
            .finish()
    }
}

// ============================================================================
// Capability Registry
// ============================================================================

/// Registry that holds all available capability implementations.
///
/// The registry provides access to capabilities by ID and allows
/// applying multiple capabilities to build a RuntimeAgent.
///
/// # Example
///
/// ```
/// use everruns_core::capabilities::CapabilityRegistry;
///
/// let registry = CapabilityRegistry::with_builtins();
///
/// // Core presets contain only effect-neutral capabilities. Applications add
/// // policy and integration bundles through their owning composition crates.
/// if let Some(cap) = registry.get("human_intent") {
///     println!("Capability: {}", cap.name());
/// }
///
/// // List all available capabilities
/// for cap in registry.list() {
///     println!("{}: {}", cap.id(), cap.name());
/// }
/// ```
#[derive(Clone)]
pub struct CapabilityRegistry {
    capabilities: HashMap<String, Arc<dyn Capability>>,
    /// Canonical-id/alias bookkeeping delegated to the neutral capability
    /// contract so the Framework and product resolve identity identically
    /// (see [`Capability::aliases`]).
    index: everruns_capability::CapabilityIdIndex,
}

impl CapabilityRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            capabilities: HashMap::new(),
            index: everruns_capability::CapabilityIdIndex::new(),
        }
    }

    /// Create a registry with the broad effect-neutral core preset registered.
    ///
    /// Portable policy implementations live in `everruns-builtins`, and
    /// environment/product implementations live in their owning composition
    /// crates. Uses `DeploymentGrade::from_env()` to select grade-gated core
    /// capabilities. For explicit control, use `with_builtins_for_grade()`.
    pub fn with_builtins() -> Self {
        Self::with_builtins_for_grade(DeploymentGrade::from_env())
    }

    /// Create a registry with capabilities that are usable in the public
    /// in-process runtime with its default host services.
    ///
    /// This intentionally excludes hosted Everruns product capabilities,
    /// demos/tests, and capabilities whose tools require optional host backends
    /// such as `platform_store`, `session_task_registry`, `schedule_store`, SQL
    /// databases, provider credentials, or knowledge stores. Embedders can
    /// still opt into those capabilities by supplying an explicit
    /// [`PlatformDefinition`](crate::PlatformDefinition) with the required
    /// backends.
    pub fn runtime_builtins() -> Self {
        let mut registry = Self::new();

        registry.register(HumanIntentCapability);
        registry.register(SessionStorageCapability);
        registry.register(SessionCapability);
        registry.register(InfinityContextCapability);
        registry.register(SkillsCapability);
        registry.register(user_hooks::UserHooksCapability);

        registry
    }

    /// Create the broad effect-neutral core preset for a deployment grade.
    ///
    /// Experimental capabilities are included via integration plugins in dev environments.
    /// Non-experimental integration plugins (like Daytona) are included in all environments.
    pub fn with_builtins_for_grade(grade: DeploymentGrade) -> Self {
        let mut registry = Self::new();

        // Core capabilities (all environments)
        registry.register(HumanIntentCapability);
        registry.register(ResearchCapability);
        registry.register(OpenRouterServerToolsCapability);
        registry.register(MemoryCapability);
        registry.register(SessionStorageCapability);
        registry.register(SessionCapability);
        registry.register(SessionSqlDatabaseCapability);
        registry.register(BackgroundExecutionCapability);
        registry.register(SessionScheduleCapability);
        registry.register(InfinityContextCapability);

        // Skills (filesystem-based discovery + activation, all environments)
        registry.register(SkillsCapability);

        // Subagents (spawn child agent sessions, all environments)
        registry.register(SubagentCapability);

        // Session tasks (inspect/steer background work, all environments)
        registry.register(SessionTasksCapability);

        // Deployment-level execution feature decisions (EVE-878): resolved once
        // from env + grade; never reads org feature-management records.
        let feature_decisions = crate::ExecutionFeatureDecisions::from_env(grade);

        // Outbound agent delegation — experimental (dev-only by default).
        // Risk: exfil, SSRF-adjacent reach, cost/recursion fan-out.
        // Gated by FEATURE_AGENT_DELEGATION; auto-enabled in dev, off in prod.
        if feature_decisions.agent_delegation {
            registry.register(AgentHandoffCapability);
            // Additionally compile-gated behind the `a2a` cargo feature: in
            // provider builds that disable core defaults the delegation
            // capability is absent even when FEATURE_AGENT_DELEGATION is set.
            #[cfg(feature = "a2a")]
            registry.register(A2aAgentDelegationCapability);
        }

        // User hooks (see knowledge/runtime-resources/user-hooks.md): user-authored shell commands
        // at lifecycle/tool events. Risk: High.
        registry.register(user_hooks::UserHooksCapability);

        // OpenUI/A2UI prompt helpers are product features, not required by embedders.
        #[cfg(feature = "ui-capabilities")]
        {
            registry.register(OpenUiCapability);
            registry.register(A2UiCapability);
        }

        // Data knowledge scaffold (all environments)
        registry.register(DataKnowledgeCapability);

        // Knowledge bases (curated org knowledge — see knowledge/runtime-resources/knowledge-bases.md)
        registry.register(KnowledgeBaseCapability);

        // Knowledge indexes (source-backed embedded collections — see knowledge/runtime-resources/knowledge-indexes.md)
        registry.register(KnowledgeIndexCapability);

        // Retrieval citations (claim-level provenance from search results — see knowledge/runtime-resources/citations.md)
        registry.register(CitationRetrievalCapability);

        // Citation verification (stamps faithfulness verdicts — see knowledge/runtime-resources/citations.md)
        registry.register(CitationVerificationCapability);

        // Demo/test fixture capabilities (fake_*, test_math/test_weather,
        // sample_data, noop) are NOT registered here. They live in the
        // `everruns-test-support` crate (EVE-875) and are registered
        // explicitly by tests and examples, never by product registries.

        // External integration plugins (registered via inventory::submit! in integration crates)
        let internal_flags = &feature_decisions.internal;
        if internal_flags.session_sandbox {
            registry.register(SessionSandboxCapability);
        }

        for plugin in inventory::iter::<IntegrationPlugin>() {
            if (!plugin.experimental_only || grade.experimental_features_enabled())
                && plugin
                    .feature_flag
                    .is_none_or(|f| feature_decisions.is_enabled(f))
            {
                registry.register_boxed((plugin.factory)());
            }
        }

        registry
    }

    /// Register a capability
    pub fn register(&mut self, capability: impl Capability + 'static) {
        self.register_arc(Arc::new(capability));
    }

    /// Register a boxed capability
    pub fn register_boxed(&mut self, capability: Box<dyn Capability>) {
        self.register_arc(Arc::from(capability));
    }

    /// Register an Arc-wrapped capability.
    ///
    /// Re-registering the same canonical ID replaces the previous
    /// implementation (legacy override semantics); use
    /// [`CapabilityRegistry::try_register_arc`] to reject collisions instead.
    pub fn register_arc(&mut self, capability: Arc<dyn Capability>) {
        let canonical = capability.id().to_string();
        self.index
            .insert_or_replace(canonical.clone(), &capability.aliases());
        self.capabilities.insert(canonical, capability);
    }

    /// Register an Arc-wrapped capability, rejecting duplicate IDs and alias
    /// collisions via the neutral contract's registry rules.
    pub fn try_register_arc(
        &mut self,
        capability: Arc<dyn Capability>,
    ) -> Result<(), everruns_capability::CapabilityError> {
        let canonical = capability.id().to_string();
        self.index
            .insert(canonical.clone(), &capability.aliases())?;
        self.capabilities.insert(canonical, capability);
        Ok(())
    }

    /// Get a capability by ID or alias
    pub fn get(&self, id: &str) -> Option<&Arc<dyn Capability>> {
        self.capabilities.get(self.index.canonical_of(id)?)
    }

    /// Resolve an ID or alias to the canonical capability ID.
    ///
    /// Returns `None` for IDs that are neither registered nor an alias of a
    /// registered capability (e.g. declarative or MCP refs).
    pub fn canonical_id<'a>(&'a self, id: &'a str) -> Option<&'a str> {
        self.index.canonical_of(id)
    }

    /// Remove a capability from the registry by ID or alias.
    pub fn unregister(&mut self, id: &str) -> Option<Arc<dyn Capability>> {
        let canonical = self.index.remove(id)?;
        self.capabilities.remove(&canonical)
    }

    /// Check if a capability is registered (by ID or alias)
    pub fn has(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// Get all registered capabilities
    pub fn list(&self) -> Vec<&Arc<dyn Capability>> {
        self.capabilities.values().collect()
    }

    /// Get the number of registered capabilities
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Check if the registry is empty
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Create a builder for fluent capability registration
    pub fn builder() -> CapabilityRegistryBuilder {
        CapabilityRegistryBuilder::new()
    }

    /// Find a blueprint by ID across all registered capabilities.
    ///
    /// Returns a fresh `AgentBlueprint` (with new tool instances) each time.
    pub fn blueprint(&self, id: &str) -> Option<AgentBlueprint> {
        for cap in self.capabilities.values() {
            for bp in cap.agent_blueprints() {
                if bp.id == id {
                    return Some(bp);
                }
            }
        }
        None
    }

    /// Find a blueprint and the capability that registered it.
    ///
    /// Returns `(capability_id, blueprint)` with fresh tool instances.
    pub fn blueprint_with_capability(&self, id: &str) -> Option<(String, AgentBlueprint)> {
        for (capability_id, cap) in &self.capabilities {
            for bp in cap.agent_blueprints() {
                if bp.id == id {
                    return Some((capability_id.clone(), bp));
                }
            }
        }
        None
    }

    /// Collect all blueprints from all registered capabilities.
    pub fn all_blueprints(&self) -> Vec<AgentBlueprint> {
        self.capabilities
            .values()
            .flat_map(|cap| cap.agent_blueprints())
            .collect()
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

impl std::fmt::Debug for CapabilityRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<_> = self.capabilities.keys().collect();
        f.debug_struct("CapabilityRegistry")
            .field("capabilities", &ids)
            .finish()
    }
}

/// Builder for creating a CapabilityRegistry with a fluent API
pub struct CapabilityRegistryBuilder {
    registry: CapabilityRegistry,
}

impl CapabilityRegistryBuilder {
    /// Create a new builder with an empty registry
    pub fn new() -> Self {
        Self {
            registry: CapabilityRegistry::new(),
        }
    }

    /// Create a new builder with the broad effect-neutral core preset.
    pub fn with_builtins() -> Self {
        Self {
            registry: CapabilityRegistry::with_builtins(),
        }
    }

    /// Add a capability
    pub fn capability(mut self, capability: impl Capability + 'static) -> Self {
        self.registry.register(capability);
        self
    }

    /// Build the registry
    pub fn build(self) -> CapabilityRegistry {
        self.registry
    }
}

impl Default for CapabilityRegistryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Collect Capabilities Helper
// ============================================================================

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
    /// OpenRouter routing controls (set when the `openrouter_server_tools`
    /// capability is present). Carries provider-executed server tools.
    pub openrouter_routing: Option<crate::driver_registry::OpenRouterRoutingConfig>,
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

struct SpawnAgentTargetProvider {
    target_type: &'static str,
    tool: Box<dyn Tool>,
}

/// Shared execution mode accepted natively by every `spawn_agent` provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SpawnMode {
    Background,
    Foreground,
}

impl SpawnMode {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "background" => Some(Self::Background),
            "foreground" => Some(Self::Foreground),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Background => "background",
            Self::Foreground => "foreground",
        }
    }
}

struct UnifiedSpawnAgentTool {
    providers: Vec<SpawnAgentTargetProvider>,
}

impl UnifiedSpawnAgentTool {
    fn new(providers: Vec<SpawnAgentTargetProvider>) -> Self {
        Self { providers }
    }

    fn provider_for(&self, target_type: &str) -> Option<&dyn Tool> {
        self.providers
            .iter()
            .find(|provider| provider.target_type == target_type)
            .map(|provider| provider.tool.as_ref())
    }

    fn target_types(&self) -> Vec<&'static str> {
        ["subagent", "agent", "external_a2a"]
            .into_iter()
            .filter(|target_type| {
                self.providers
                    .iter()
                    .any(|provider| provider.target_type == *target_type)
            })
            .collect()
    }

    /// Per-`target.type` constraint branches, nested inside the `target`
    /// property. Anthropic rejects `oneOf`/`allOf`/`anyOf` at the top level
    /// of a tool `input_schema`, so provider-specific requirements must live
    /// below the root (nested composition is accepted).
    fn target_constraint_branches(&self) -> Vec<serde_json::Value> {
        self.target_types()
            .into_iter()
            .filter_map(|target_type| match target_type {
                "subagent" => Some(serde_json::json!({
                    "properties": {
                        "type": {"const": "subagent"}
                    }
                })),
                "agent" => Some(serde_json::json!({
                    "properties": {
                        "type": {"const": "agent"}
                    },
                    "required": ["type", "id"]
                })),
                "external_a2a" => Some(serde_json::json!({
                    "properties": {
                        "type": {"const": "external_a2a"}
                    },
                    "anyOf": [
                        {"required": ["id"]},
                        {"required": ["external_agent_id"]}
                    ]
                })),
                _ => None,
            })
            .collect()
    }

    // NOTE: subagent and agent providers require `name` at execution
    // (`require_str`), while external_a2a ignores it. A schema that required
    // `name` only for the local targets would need a top-level
    // `oneOf`/`if`/`allOf`, which Anthropic rejects in a tool `input_schema`.
    // `name` is therefore required at the root unconditionally: requiring a
    // field external_a2a merely ignores is safe (the schema never permits a
    // call execution would reject), whereas omitting it would let a
    // `name`-less subagent call pass validation and then fail at dispatch —
    // exactly the mismatch #2787 set out to close.
}

#[async_trait]
impl Tool for UnifiedSpawnAgentTool {
    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let target_type = tool_call
            .arguments
            .get("target")
            .and_then(|target| target.get("type"))
            .and_then(serde_json::Value::as_str)?;
        self.provider_for(target_type)
            .and_then(|tool| tool.narrate(tool_call, phase, locale, ctx))
    }

    fn name(&self) -> &str {
        "spawn_agent"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Spawn Agent")
    }

    fn description(&self) -> &str {
        "Delegate work to another agent target. Set target.type to one of the advertised target types; background returns a task_id for generic task tools, and foreground waits for the result."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Human-readable name for the delegated run (subagent, first-party handoff, or external delegation). Used as the task label."
                },
                "instructions": {
                    "type": "string",
                    "description": "Instructions for the delegated agent. Do not include credentials or bearer tokens."
                },
                "goal": {
                    "type": "string",
                    "description": "Optional objective stored on the spawned session and made visible at system-prompt level."
                },
                "lifetime": {
                    "type": "string",
                    "enum": ["linked", "detached"],
                    "default": "linked",
                    "description": "linked creates a lifecycle child; detached creates an independent top-level peer session. Not valid for external_a2a."
                },
                "seed": {
                    "type": "string",
                    "enum": ["fresh", "fork", "workspace"],
                    "default": "fresh",
                    "description": "Detached-session seed mode: fresh starts blank, fork copies history/workspace/session storage, workspace copies workspace files only."
                },
                "target": {
                    "type": "object",
                    "properties": {
                        "type": {
                            "type": "string",
                            "enum": self.target_types(),
                            "description": "Delegation target type. Use subagent for same-agent child sessions, agent for configured first-party handoffs, or external_a2a for configured remote A2A agents."
                        },
                        "id": {
                            "type": "string",
                            "description": "Configured target id for first-party handoffs or external A2A agents."
                        },
                        "external_agent_id": {
                            "type": "string",
                            "description": "Configured external A2A agent id."
                        }
                    },
                    "required": ["type"],
                    "oneOf": self.target_constraint_branches(),
                    "additionalProperties": false
                },
                "mode": {
                    "type": "string",
                    "enum": ["background", "foreground"],
                    "description": "Execution mode. Use background to return immediately with a task_id, or foreground to block until the delegated work reaches a terminal state or timeout."
                },
                "blueprint": {
                    "type": "string",
                    "description": "Subagent-only blueprint ID to spawn a specialist agent with its own tools and model."
                },
                "config": {
                    "type": "object",
                    "description": "Subagent-only blueprint configuration. Only valid when blueprint is set."
                },
                "result_schema": {
                    "type": "object",
                    "description": "JSON Schema for a required final structured result. Local child agents must call report_result; external A2A agents must return a structured data artifact."
                },
                "message_schema": {
                    "type": "object",
                    "description": "JSON Schema for structured progress messages from local child agents. When set, the child receives report_task_progress. External A2A targets reject this option explicitly."
                },
                "public_context": {
                    "type": "object",
                    "description": "Agent-handoff-only non-secret structured context to include with the instructions."
                },
                "wait_timeout_secs": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 86400,
                    "description": "External-A2A-only foreground timeout."
                },
                "wake_on_completion": {
                    "type": "boolean",
                    "description": "External-A2A-only control for background completion wake-ups."
                }
            },
            "required": ["name", "instructions", "target"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> crate::tool_types::ToolHints {
        let mut hints = crate::tool_types::ToolHints::default()
            .with_long_running(true)
            .with_concurrency_class(SPAWN_AGENT_CONCURRENCY_CLASS);
        if self.provider_for("external_a2a").is_some() {
            hints = hints.with_open_world(true);
        }
        hints
    }

    async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "spawn_agent requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: serde_json::Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let target_type = match arguments
            .get("target")
            .and_then(|target| target.get("type"))
            .and_then(serde_json::Value::as_str)
        {
            Some(target_type) => target_type,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: target.type");
            }
        };

        let Some(provider) = self.provider_for(target_type) else {
            let supported = self.target_types().join(", ");
            return ToolExecutionResult::tool_error(format!(
                "Unsupported spawn_agent target.type: \"{target_type}\". Supported target types: {supported}"
            ));
        };
        if target_type == "external_a2a"
            && arguments
                .get("lifetime")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value == "detached")
        {
            return ToolExecutionResult::tool_error(
                "lifetime=\"detached\" is only valid for local session targets (subagent or agent), not external_a2a.",
            );
        }
        if target_type == "external_a2a"
            && arguments
                .get("message_schema")
                .is_some_and(|schema| !schema.is_null())
        {
            return ToolExecutionResult::tool_error(
                "message_schema is not supported for external_a2a targets because remote agents cannot receive report_task_progress.",
            );
        }

        provider.execute_with_context(arguments, context).await
    }

    fn requires_context(&self) -> bool {
        true
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
fn compaction_is_enabled(
    capability_configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
) -> bool {
    capability_configs.iter().any(|cap_config| {
        registry.get(cap_config.capability_id()).is_some_and(|cap| {
            cap.status() == CapabilityStatus::Available
                && cap.compaction_policy(cap_config.config_value()).is_some()
        })
    })
}

/// Per-agent message-filter config for a capability, injecting the derived
/// `compaction_active` signal into infinity context when compaction is enabled.
///
/// This is the one place capability composition is encoded: infinity context and
/// compaction are otherwise independent, but if infinity context evicts history
/// before compaction can summarize it, compaction only ever sees the recent
/// window. The flag tells infinity context to anchor + provide `query_history`
/// and let compaction own reduction.
fn message_filter_config_for(
    cap_id: &str,
    base: &serde_json::Value,
    compaction_on: bool,
) -> serde_json::Value {
    if cap_id != INFINITY_CONTEXT_CAPABILITY_ID || !compaction_on {
        return base.clone();
    }
    let mut config = base.clone();
    match config.as_object_mut() {
        Some(map) => {
            map.insert(
                "compaction_active".to_string(),
                serde_json::Value::Bool(true),
            );
        }
        None => {
            config = serde_json::json!({ "compaction_active": true });
        }
    }
    config
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
            if capability.status() != CapabilityStatus::Available {
                continue;
            }
            // Resolve against None: no model is known at message-filter collection
            // time, so fall back to the model-agnostic variant if present.
            let effective: &dyn Capability = capability
                .resolve_for_model(None)
                .unwrap_or_else(|| capability.as_ref());
            if let Some(provider) = effective.message_filter_provider() {
                let config =
                    message_filter_config_for(cap_id, cap_config.config_value(), compaction_on);
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
/// ReasonAtom already holds `model_with_provider`). Pass `None` only when the
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
            if capability.status() != CapabilityStatus::Available {
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
            if capability.status() != CapabilityStatus::Available {
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
                if definition.status != CapabilityStatus::Available {
                    continue;
                }
                if let Some(contributed) = definition.mcp_servers {
                    servers = merge_scoped_mcp_servers(&servers, &contributed);
                }
            }
            continue;
        }
        if let Some(capability) = registry.get(cap_id) {
            if capability.status() != CapabilityStatus::Available {
                continue;
            }
            servers = merge_scoped_mcp_servers(
                &servers,
                &capability.mcp_servers_with_config(cap_config.config_value()),
            );
        }
    }

    servers
}

// ============================================================================
// Dependency Resolution
// ============================================================================

/// Maximum number of capabilities after dependency resolution.
/// This prevents runaway dependency chains and resource exhaustion.
pub const MAX_RESOLVED_CAPABILITIES: usize = 100;

/// Error type for dependency resolution failures
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyError {
    /// Circular dependency detected in the capability graph
    CircularDependency {
        /// The capability where the cycle was detected
        capability_id: String,
        /// The dependency chain leading to the cycle
        chain: Vec<String>,
    },
    /// Too many capabilities after resolution
    TooManyCapabilities {
        /// Number of capabilities requested
        count: usize,
        /// Maximum allowed
        max: usize,
    },
}

impl std::fmt::Display for DependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencyError::CircularDependency {
                capability_id,
                chain,
            } => {
                write!(
                    f,
                    "Circular dependency detected: {} depends on itself via chain: {} -> {}",
                    capability_id,
                    chain.join(" -> "),
                    capability_id
                )
            }
            DependencyError::TooManyCapabilities { count, max } => {
                write!(
                    f,
                    "Too many capabilities after resolution: {} (max: {})",
                    count, max
                )
            }
        }
    }
}

impl std::error::Error for DependencyError {}

/// Result of resolving capability dependencies
#[derive(Debug, Clone)]
pub struct ResolvedCapabilities {
    /// All capability IDs after resolving dependencies (in topological order)
    /// Dependencies come before dependents.
    pub resolved_ids: Vec<String>,
    /// IDs that were added as dependencies (not in the original selection)
    pub added_as_dependencies: Vec<String>,
    /// Original user-selected capability IDs
    pub user_selected: Vec<String>,
}

/// Resolve capability dependencies, returning all required capability IDs.
///
/// This function:
/// 1. Takes the user-selected capability IDs
/// 2. Recursively collects all dependencies
/// 3. Returns them in topological order (dependencies before dependents)
/// 4. Detects circular dependencies and returns an error
/// 5. Enforces a maximum capability limit
///
/// # Arguments
///
/// * `selected_ids` - User-selected capability IDs
/// * `registry` - The capability registry to look up dependencies
///
/// # Returns
///
/// `Ok(ResolvedCapabilities)` with all required capabilities in order,
/// or `Err(DependencyError)` if circular dependencies are detected or
/// the limit is exceeded.
pub fn resolve_dependencies(
    selected_ids: &[String],
    registry: &CapabilityRegistry,
) -> Result<ResolvedCapabilities, DependencyError> {
    use std::collections::HashSet;

    // Canonicalize so capabilities selected via alias match their resolved IDs.
    let user_selected: HashSet<String> = selected_ids
        .iter()
        .map(|id| registry.canonical_id(id).unwrap_or(id).to_string())
        .collect();
    let mut resolved: Vec<String> = Vec::new();
    let mut resolved_set: HashSet<String> = HashSet::new();
    let mut added_as_dependencies: Vec<String> = Vec::new();

    // Process each selected capability and its dependencies using DFS
    for cap_id in selected_ids {
        resolve_single_capability(
            cap_id,
            registry,
            &mut resolved,
            &mut resolved_set,
            &mut added_as_dependencies,
            &user_selected,
            &mut Vec::new(), // visiting chain for cycle detection
        )?;
    }

    // Check max limit
    if resolved.len() > MAX_RESOLVED_CAPABILITIES {
        return Err(DependencyError::TooManyCapabilities {
            count: resolved.len(),
            max: MAX_RESOLVED_CAPABILITIES,
        });
    }

    Ok(ResolvedCapabilities {
        resolved_ids: resolved,
        added_as_dependencies,
        user_selected: selected_ids.to_vec(),
    })
}

/// Resolve dependency-expanded capability configs, preserving explicit config on selected IDs.
///
/// Dependencies are inserted with empty configs. If the same capability is provided more than
/// once, the last explicit config wins.
pub fn resolve_capability_configs(
    selected_configs: &[AgentCapabilityConfig],
    registry: &CapabilityRegistry,
) -> Result<Vec<AgentCapabilityConfig>, DependencyError> {
    let mut selected_ids: Vec<String> = Vec::new();
    for config in selected_configs {
        // Both `declarative:` and `plugin:` carry a `DeclarativeCapabilityDefinition`
        // config that may declare dependencies.
        if (is_declarative_capability(config.capability_id())
            || is_plugin_capability(config.capability_id()))
            && let Ok(definition) = serde_json::from_value::<DeclarativeCapabilityDefinition>(
                config.config_value().clone(),
            )
        {
            selected_ids.extend(definition.dependencies);
        }
        selected_ids.push(config.capability_id().to_string());
    }
    let resolved = resolve_dependencies(&selected_ids, registry)?;

    // Key explicit configs by canonical ID so config supplied under an alias
    // still attaches to the (canonical) resolved capability ID.
    let explicit_configs: std::collections::HashMap<String, serde_json::Value> = selected_configs
        .iter()
        .map(|config| {
            let id = config.capability_id();
            let id = registry.canonical_id(id).unwrap_or(id);
            (id.to_string(), config.config_value().clone())
        })
        .collect();

    Ok(resolved
        .resolved_ids
        .into_iter()
        .map(|capability_id| {
            explicit_configs
                .get(&capability_id)
                .cloned()
                .map(|config| AgentCapabilityConfig::with_config(capability_id.clone(), config))
                .unwrap_or_else(|| AgentCapabilityConfig::new(capability_id))
        })
        .collect())
}

/// Helper function to resolve a single capability and its dependencies recursively.
fn resolve_single_capability(
    cap_id: &str,
    registry: &CapabilityRegistry,
    resolved: &mut Vec<String>,
    resolved_set: &mut std::collections::HashSet<String>,
    added_as_dependencies: &mut Vec<String>,
    user_selected: &std::collections::HashSet<String>,
    visiting: &mut Vec<String>,
) -> Result<(), DependencyError> {
    // Normalize aliases to the canonical ID so an alias and its canonical ID
    // resolve (and dedupe) to the same capability. Unknown IDs (declarative,
    // MCP, skill refs) pass through unchanged.
    let cap_id = registry.canonical_id(cap_id).unwrap_or(cap_id);

    // Already resolved
    if resolved_set.contains(cap_id) {
        return Ok(());
    }

    // Check for circular dependency
    if visiting.contains(&cap_id.to_string()) {
        return Err(DependencyError::CircularDependency {
            capability_id: cap_id.to_string(),
            chain: visiting.clone(),
        });
    }

    // Get capability from registry
    let capability = match registry.get(cap_id) {
        Some(cap) => cap,
        None => {
            // `declarative:` and `plugin:` refs carry their full definition in
            // the config payload — they don't need a registry entry. Pass them
            // through so `collect_capabilities_with_configs` can process them.
            if (is_declarative_capability(cap_id) || is_plugin_capability(cap_id))
                && !resolved_set.contains(cap_id)
            {
                resolved.push(cap_id.to_string());
                resolved_set.insert(cap_id.to_string());
                if !user_selected.contains(cap_id) {
                    added_as_dependencies.push(cap_id.to_string());
                }
            }
            return Ok(());
        }
    };

    // Mark as visiting
    visiting.push(cap_id.to_string());

    // Resolve dependencies first (depth-first)
    for dep_id in capability.dependencies() {
        resolve_single_capability(
            dep_id,
            registry,
            resolved,
            resolved_set,
            added_as_dependencies,
            user_selected,
            visiting,
        )?;
    }

    // Remove from visiting
    visiting.pop();

    // Add to resolved
    if !resolved_set.contains(cap_id) {
        resolved.push(cap_id.to_string());
        resolved_set.insert(cap_id.to_string());

        // Track if this was added as a dependency (not user-selected)
        if !user_selected.contains(cap_id) {
            added_as_dependencies.push(cap_id.to_string());
        }
    }

    Ok(())
}

/// Compute the aggregated set of UI features from a list of capability IDs.
///
/// Resolves dependencies, collects features from all resolved capabilities,
/// and returns deduplicated feature strings.
pub fn compute_features(capability_ids: &[String], registry: &CapabilityRegistry) -> Vec<String> {
    use std::collections::HashSet;

    let resolved_ids = match resolve_dependencies(capability_ids, registry) {
        Ok(resolved) => resolved.resolved_ids,
        Err(_) => capability_ids.to_vec(),
    };

    let mut seen = HashSet::new();
    let mut features = Vec::new();
    for cap_id in &resolved_ids {
        if let Some(cap) = registry.get(cap_id) {
            for feature in cap.features() {
                if seen.insert(feature) {
                    features.push(feature.to_string());
                }
            }
        }
    }
    features
}

/// Get direct dependencies for a capability ID.
/// Returns empty vec if capability not found.
pub fn get_dependencies(cap_id: &str, registry: &CapabilityRegistry) -> Vec<String> {
    registry
        .get(cap_id)
        .map(|cap| cap.dependencies().iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

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
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut tool_definitions: Vec<ToolDefinition> = Vec::new();
    let mut mounts: Vec<MountPoint> = Vec::new();
    let mut message_filter_providers: Vec<(Arc<dyn MessageFilterProvider>, serde_json::Value)> =
        Vec::new();
    let mut applied_ids: Vec<String> = Vec::new();
    let mut tool_search: Option<crate::driver_registry::ToolSearchConfig> = None;
    let mut prompt_cache: Option<crate::driver_registry::PromptCacheConfig> = None;
    let mut openrouter_routing: Option<crate::driver_registry::OpenRouterRoutingConfig> = None;
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
    let mut agent_handoff_spawn_config: Option<serde_json::Value> = None;
    let mut spawn_agent_providers: Vec<SpawnAgentTargetProvider> = Vec::new();

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
                    if definition.status != CapabilityStatus::Available {
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
                        mcp_servers = merge_scoped_mcp_servers(&mcp_servers, servers);
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
            // Only collect from available capabilities
            if capability.status() != CapabilityStatus::Available {
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
            if cap_id == AGENT_HANDOFF_CAPABILITY_ID {
                agent_handoff_spawn_config = Some(cap_config.config_value().clone());
            }

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
            for tool in effective.tools_with_config(cap_config.config_value()) {
                if cap_id == A2A_AGENT_DELEGATION_CAPABILITY_ID && tool.name() == "spawn_agent" {
                    spawn_agent_providers.push(SpawnAgentTargetProvider {
                        target_type: "external_a2a",
                        tool,
                    });
                } else {
                    tools.push(tool);
                }
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
                if cap_id == A2A_AGENT_DELEGATION_CAPABILITY_ID && def.name() == "spawn_agent" {
                    continue;
                }
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

            if cap_id == OPENROUTER_SERVER_TOOLS_CAPABILITY_ID {
                let server_tools =
                    openrouter_server_tools::server_tools_from_config(cap_config.config_value());
                if !server_tools.is_empty() {
                    openrouter_routing = Some(crate::driver_registry::OpenRouterRoutingConfig {
                        server_tools,
                        ..Default::default()
                    });
                }
            }

            // Collect mount points
            mounts.extend(effective.mounts());

            mcp_servers = merge_scoped_mcp_servers(
                &mcp_servers,
                &effective.mcp_servers_with_config(cap_config.config_value()),
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
                    message_filter_config_for(cap_id, cap_config.config_value(), compaction_on);
                message_filter_providers.push((provider, config));
            }

            applied_ids.push(cap_id.to_string());
        }
    }

    // EVE-677 migration: known delegation providers now share one model-facing
    // `spawn_agent` dispatcher so subagents, first-party handoffs, and external
    // A2A agents can coexist in the same session. Unknown third-party
    // `spawn_agent` owners still win to avoid changing their contract.
    if applied_ids.iter().any(|id| id == SUBAGENTS_CAPABILITY_ID) {
        spawn_agent_providers.push(SpawnAgentTargetProvider {
            target_type: "subagent",
            tool: Box::new(SpawnSubagentAsAgentTool),
        });
    }
    if let Some(config) = agent_handoff_spawn_config.as_ref() {
        spawn_agent_providers.push(SpawnAgentTargetProvider {
            target_type: "agent",
            tool: Box::new(SpawnAgentHandoffTool::new(config)),
        });
    }
    if !tools.iter().any(|tool| tool.name() == "spawn_agent") && !spawn_agent_providers.is_empty() {
        let tool = UnifiedSpawnAgentTool::new(spawn_agent_providers);
        let def = tool
            .to_definition()
            .with_category("Orchestration")
            .with_capability_attribution("agent_delegation", Some("Agent Delegation"));
        tools.push(Box::new(tool));
        tool_definitions.push(def);
    }

    // Auto-activate `background_execution` whenever any collected tool
    // declares background support via `ToolHints::supports_background`.
    //
    // This is the generic cross-cutting capability contract — meta-tools that
    // wrap other tools based on hints should hook in here, not attach to a
    // single owner capability (e.g. `bashkit_shell`).
    //
    // Lockstep: we extend both `tools` (execution registry) and
    // `tool_definitions` (model-visible) so the model can see and the worker
    // can dispatch `spawn_background` from the same activation event. See
    // `knowledge/execution/background-execution.md`.
    if !applied_ids
        .iter()
        .any(|id| id == BACKGROUND_EXECUTION_CAPABILITY_ID)
        && tool_definitions
            .iter()
            .any(|def| def.hints().supports_background == Some(true))
        && let Some(bg_cap) = registry.get(BACKGROUND_EXECUTION_CAPABILITY_ID)
        && bg_cap.status() == CapabilityStatus::Available
    {
        tools.extend(bg_cap.tools());
        let cap_category = bg_cap.category();
        for def in bg_cap.tool_definitions() {
            let def = match (def.category(), cap_category) {
                (None, Some(cat)) => def.with_category(cat),
                _ => def,
            }
            .with_capability_attribution(BACKGROUND_EXECUTION_CAPABILITY_ID, Some(bg_cap.name()));
            tool_definitions.push(def);
        }
        narration_hooks.push(Arc::new(CapabilityNarrationHook(bg_cap.clone())));
        applied_ids.push(BACKGROUND_EXECUTION_CAPABILITY_ID.to_string());
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
        tools,
        tool_definitions,
        mounts,
        message_filter_providers,
        applied_ids,
        tool_search,
        prompt_cache,
        openrouter_routing,
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
/// let registry = CapabilityRegistry::with_builtins();
/// let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");
/// let ctx = SystemPromptContext::without_file_store(SessionId::new());
///
/// let capability_ids = vec!["human_intent".to_string()];
/// let applied = apply_capabilities(base_runtime_agent, &capability_ids, &registry, &ctx).await;
///
/// assert_eq!(applied.applied_ids, vec!["human_intent"]);
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
        openrouter_routing: collected.openrouter_routing,
        network_access: base_runtime_agent.network_access,
        // Explicit request-level preference (escape hatch) wins; otherwise the
        // `parallel_tool_calls` capability supplies the preference.
        parallel_tool_calls: base_runtime_agent
            .parallel_tool_calls
            .or(collected.parallel_tool_calls),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_id::SessionId;
    use std::collections::BTreeSet;
    use uuid::Uuid;

    // Env-var-mutating tests must not run in parallel.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_env() -> std::sync::MutexGuard<'static, ()> {
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Test helper: dummy context with no file store
    fn test_ctx() -> SystemPromptContext {
        SystemPromptContext::without_file_store(SessionId::new())
    }

    // -------------------------------------------------------------------------
    // Local stand-ins for the fixture capabilities that moved to the
    // `everruns-test-support` crate (EVE-875). The registry/apply/dependency
    // mechanics tested here only need capabilities with these shapes: one
    // that contributes nothing, one that contributes plain tools, and one
    // that carries mounts plus a dependency.
    // -------------------------------------------------------------------------

    /// Contributes nothing: no tools, no prompt, no dependencies.
    struct NoopFixture;

    impl Capability for NoopFixture {
        fn id(&self) -> &str {
            "noop"
        }
        fn name(&self) -> &str {
            "No-Op"
        }
        fn description(&self) -> &str {
            "Contributes nothing."
        }
    }

    struct FixtureTool(&'static str);

    #[async_trait]
    impl Tool for FixtureTool {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "Fixture tool."
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            })
        }
        async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
            ToolExecutionResult::success(serde_json::json!({ "ok": true }))
        }
    }

    struct BackgroundFixtureTool;

    #[async_trait]
    impl Tool for BackgroundFixtureTool {
        fn name(&self) -> &str {
            "bash"
        }
        fn description(&self) -> &str {
            "Fixture background-capable shell tool."
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _arguments: serde_json::Value) -> ToolExecutionResult {
            ToolExecutionResult::success(serde_json::json!({"ok": true}))
        }
        fn hints(&self) -> crate::tool_types::ToolHints {
            crate::tool_types::ToolHints {
                supports_background: Some(true),
                ..Default::default()
            }
        }
    }

    struct FileSystemFixture;

    impl Capability for FileSystemFixture {
        fn id(&self) -> &str {
            "session_file_system"
        }
        fn name(&self) -> &str {
            "Fixture Filesystem"
        }
        fn description(&self) -> &str {
            "Fixture filesystem capability."
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            vec![
                Box::new(FixtureTool("read_file")),
                Box::new(FixtureTool("write_file")),
            ]
        }
        fn features(&self) -> Vec<&'static str> {
            vec!["file_system"]
        }
    }

    struct BashFixture;

    impl Capability for BashFixture {
        fn id(&self) -> &str {
            "bashkit_shell"
        }
        fn aliases(&self) -> Vec<&'static str> {
            vec!["virtual_bash"]
        }
        fn name(&self) -> &str {
            "Fixture Bash"
        }
        fn description(&self) -> &str {
            "Fixture shell capability."
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            vec![Box::new(BackgroundFixtureTool)]
        }
        fn dependencies(&self) -> Vec<&'static str> {
            vec!["session_file_system"]
        }
        fn features(&self) -> Vec<&'static str> {
            vec!["file_system"]
        }
        fn risk_level(&self) -> RiskLevel {
            RiskLevel::High
        }
    }

    struct WebFetchFixture;

    impl Capability for WebFetchFixture {
        fn id(&self) -> &str {
            "web_fetch"
        }
        fn name(&self) -> &str {
            "Fixture Web Fetch"
        }
        fn description(&self) -> &str {
            "Fixture web capability."
        }
        fn risk_level(&self) -> RiskLevel {
            RiskLevel::High
        }
    }

    /// Portable-policy-shaped stand-ins used only to exercise neutral core
    /// collection mechanics after policy implementations moved out of core.
    struct DynamicFactFixture;

    impl Capability for DynamicFactFixture {
        fn id(&self) -> &str {
            "current_time"
        }
        fn name(&self) -> &str {
            "Dynamic Fact Fixture"
        }
        fn description(&self) -> &str {
            "Fixture with one dynamic fact and one tool."
        }
        fn icon(&self) -> Option<&str> {
            Some("clock")
        }
        fn category(&self) -> Option<&str> {
            Some("Core")
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            vec![Box::new(FixtureTool("get_current_time"))]
        }
        fn facts(&self, _config: &serde_json::Value, _ctx: &FactsContext) -> Vec<Fact> {
            vec![Fact::dynamic("current_time", "fixture-now")]
        }
    }

    struct PromptToolFixture;

    impl Capability for PromptToolFixture {
        fn id(&self) -> &str {
            "stateless_todo_list"
        }
        fn name(&self) -> &str {
            "Prompt Tool Fixture"
        }
        fn description(&self) -> &str {
            "Fixture with a static prompt and tool."
        }
        fn system_prompt_addition(&self) -> Option<&str> {
            Some("Task Management uses the write_todos tool.")
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            vec![Box::new(FixtureTool("write_todos"))]
        }
    }

    struct DynamicPreviewFixture;

    impl Capability for DynamicPreviewFixture {
        fn id(&self) -> &str {
            "agent_instructions"
        }
        fn name(&self) -> &str {
            "Dynamic Preview Fixture"
        }
        fn description(&self) -> &str {
            "Fixture whose runtime prompt is dynamic."
        }
        fn system_prompt_preview(&self) -> Option<String> {
            Some("Reads AGENTS.md dynamically.".to_string())
        }
    }

    /// Contributes four plain calculator-style tools and no prompt addition.
    struct MathFixture;

    impl Capability for MathFixture {
        fn id(&self) -> &str {
            "test_math"
        }
        fn name(&self) -> &str {
            "Test Math"
        }
        fn description(&self) -> &str {
            "Fixture: calculator tools."
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            vec![
                Box::new(FixtureTool("add")),
                Box::new(FixtureTool("subtract")),
                Box::new(FixtureTool("multiply")),
                Box::new(FixtureTool("divide")),
            ]
        }
    }

    /// Contributes two plain tools.
    struct WeatherFixture;

    impl Capability for WeatherFixture {
        fn id(&self) -> &str {
            "test_weather"
        }
        fn name(&self) -> &str {
            "Test Weather"
        }
        fn description(&self) -> &str {
            "Fixture: weather tools."
        }
        fn tools(&self) -> Vec<Box<dyn Tool>> {
            vec![
                Box::new(FixtureTool("get_weather")),
                Box::new(FixtureTool("get_forecast")),
            ]
        }
    }

    /// Carries a read-only mount, a prompt addition, a feature, and a
    /// dependency on `session_file_system`.
    struct SampleDataFixture;

    impl Capability for SampleDataFixture {
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
        fn mounts(&self) -> Vec<MountPoint> {
            let samples_dir = MountDirectoryBuilder::new()
                .file("users.json", "[]")
                .build();
            vec![MountPoint::readonly("/samples", samples_dir, self.id())]
        }
        fn dependencies(&self) -> Vec<&'static str> {
            vec!["session_file_system"]
        }
        fn features(&self) -> Vec<&'static str> {
            vec!["file_system"]
        }
    }

    /// Built-in registry plus the local fixture stand-ins above.
    fn fixture_registry() -> CapabilityRegistry {
        let mut registry = CapabilityRegistry::with_builtins();
        registry.register(NoopFixture);
        registry.register(MathFixture);
        registry.register(WeatherFixture);
        registry.register(SampleDataFixture);
        registry.register(FileSystemFixture);
        registry.register(BashFixture);
        registry.register(WebFetchFixture);
        registry.register(DynamicFactFixture);
        registry.register(PromptToolFixture);
        registry.register(DynamicPreviewFixture);
        registry
    }

    /// A host-defined capability carrying annotations core knows nothing about.
    struct HostAnnotatedCapability;

    #[async_trait]
    impl Capability for HostAnnotatedCapability {
        fn id(&self) -> &str {
            "host_annotated"
        }
        fn name(&self) -> &str {
            "Host Annotated"
        }
        fn description(&self) -> &str {
            "Test capability with host-owned metadata."
        }
        fn metadata(&self) -> Option<serde_json::Value> {
            Some(serde_json::json!({"icon": "sparkles", "group": "host"}))
        }
    }

    #[test]
    fn capability_metadata_is_an_opt_in_host_hatch() {
        // Core capabilities carry none, so nothing changes for them.
        assert!(HumanIntentCapability.metadata().is_none());

        let metadata = HostAnnotatedCapability.metadata().expect("metadata");
        assert_eq!(metadata["icon"], "sparkles");
        assert_eq!(metadata["group"], "host");
    }

    /// Base set of built-in capabilities present in all environments (no experimental delegation).
    fn expected_core_builtin_ids() -> BTreeSet<&'static str> {
        let mut ids = [
            "human_intent",
            "research",
            "session_storage",
            "session",
            "session_sql_database",
            "background_execution",
            "session_schedule",
            "infinity_context",
            "memory",
            "session_tasks",
            "skills",
            "subagents",
            "data_knowledge",
            "knowledge_base",
            "knowledge_index",
            "citation_retrieval",
            "citation_verification",
            "user_hooks",
            "openrouter_server_tools",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        if cfg!(feature = "ui-capabilities") {
            ids.insert("openui");
            ids.insert("a2ui");
        }
        ids
    }

    /// Capabilities present in the default in-process runtime registry.
    fn expected_runtime_builtin_ids() -> BTreeSet<&'static str> {
        [
            "human_intent",
            "session_storage",
            "session",
            "infinity_context",
            "skills",
            "user_hooks",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    }

    /// Full set for dev: base + experimental delegation capabilities.
    fn expected_dev_builtin_ids() -> BTreeSet<&'static str> {
        let mut ids = expected_core_builtin_ids();
        ids.insert("agent_handoff");
        if cfg!(feature = "a2a") {
            ids.insert("a2a_agent_delegation");
        }
        ids
    }

    fn registry_ids(registry: &CapabilityRegistry) -> BTreeSet<&str> {
        registry.capabilities.keys().map(String::as_str).collect()
    }

    // =========================================================================
    // CapabilityRegistry tests
    // =========================================================================

    // Note: Integration plugins (docker, daytona, etc.) are registered via inventory::submit!
    // in external crates. They only appear in the registry when the integration crate is
    // linked into the final binary. Core tests verify only built-in capabilities.
    // Integration crates have their own tests for plugin registration.

    #[test]
    fn test_capability_registry_with_builtins_dev() {
        // Dev mode includes all built-in capabilities including experimental delegation
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
        let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
        assert_eq!(registry_ids(&registry), expected_dev_builtin_ids());
        assert!(registry.has("agent_handoff"));
        assert_eq!(registry.has("a2a_agent_delegation"), cfg!(feature = "a2a"));
    }

    #[test]
    fn test_capability_registry_with_builtins_prod() {
        // Prod mode excludes experimental capabilities including delegation
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
        let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
        assert_eq!(registry_ids(&registry), expected_core_builtin_ids());
        // Experimental capabilities NOT included in prod
        assert!(!registry.has("docker_container"));
        assert!(!registry.has("agent_handoff"));
        assert!(!registry.has("a2a_agent_delegation"));
    }

    #[test]
    fn test_capability_registry_runtime_builtins() {
        let _lock = lock_env();
        unsafe { std::env::remove_var("FEATURE_LUA") };
        let registry = CapabilityRegistry::runtime_builtins();
        assert_eq!(registry_ids(&registry), expected_runtime_builtin_ids());
        for environment_backed in [
            "session_file_system",
            "bashkit_shell",
            "web_fetch",
            "lua",
            "lua_code_mode",
            "model_scout",
            "openrouter_workspace",
        ] {
            assert!(
                !registry.has(environment_backed),
                "`{environment_backed}` must be composed outside everruns-core"
            );
        }

        for platform_only in [
            "model_scout",
            "openrouter_workspace",
            "openrouter_server_tools",
            "session_tasks",
            "session_schedule",
            "subagents",
            "background_execution",
            "session_sql_database",
            "knowledge_base",
            "knowledge_index",
            "data_knowledge",
            "research",
        ] {
            assert!(
                !registry.has(platform_only),
                "`{platform_only}` should not be in the runtime default registry"
            );
        }
    }

    #[test]
    fn test_agent_delegation_enabled_by_env_in_prod() {
        // FEATURE_AGENT_DELEGATION=true enables delegation caps even in prod
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_AGENT_DELEGATION", "true") };
        let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
        assert!(registry.has("agent_handoff"));
        assert_eq!(registry.has("a2a_agent_delegation"), cfg!(feature = "a2a"));
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
    }

    #[test]
    fn test_agent_delegation_disabled_by_env_in_dev() {
        // FEATURE_AGENT_DELEGATION=false disables delegation caps even in dev
        let _lock = lock_env();
        unsafe { std::env::set_var("FEATURE_AGENT_DELEGATION", "false") };
        let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
        assert!(!registry.has("agent_handoff"));
        assert!(!registry.has("a2a_agent_delegation"));
        unsafe { std::env::remove_var("FEATURE_AGENT_DELEGATION") };
    }

    #[test]
    fn test_capability_registry_get() {
        let registry = CapabilityRegistry::with_builtins();

        let human_intent = registry.get("human_intent").unwrap();
        assert_eq!(human_intent.id(), "human_intent");
        assert_eq!(human_intent.status(), CapabilityStatus::Available);
    }

    /// Registry-wide invariants for every built-in capability. This replaces the
    /// per-capability `test_capability_metadata` / `has_tools` / `in_registry`
    /// boilerplate that only restated hardcoded constants: instead of pinning
    /// each id/name/tool-list literal, it enforces the properties that actually
    /// matter across the whole set and would catch a real defect (a blank id, a
    /// duplicate or dangling dependency, colliding tool names) that constant
    /// mirrors never could.
    #[test]
    fn builtin_capabilities_satisfy_registry_invariants() {
        let registry = CapabilityRegistry::with_builtins();

        for cap in registry.list() {
            let id = cap.id();
            assert!(!id.is_empty(), "capability has an empty id");
            assert!(
                !cap.name().trim().is_empty(),
                "capability `{id}` has an empty name"
            );

            // The registration key is `id()`, so every capability must resolve
            // by its own id (guards against an id()/registration mismatch).
            assert!(
                registry.get(id).is_some(),
                "capability `{id}` does not resolve by its own id"
            );

            // Core may declare a dependency on an implementation supplied by
            // the host composition layer. All other dependencies must resolve
            // inside the core registry.
            for dep in cap.dependencies() {
                assert!(
                    dep == "session_file_system" || registry.get(dep).is_some(),
                    "capability `{id}` depends on `{dep}`, which is not registered"
                );
            }

            // Tool names must be non-empty and unique within a capability, so
            // dispatch by name is unambiguous.
            let mut seen = std::collections::HashSet::new();
            for tool in cap.tools() {
                let name = tool.name().to_string();
                assert!(
                    !name.is_empty(),
                    "capability `{id}` exposes a tool with an empty name"
                );
                assert!(
                    seen.insert(name.clone()),
                    "capability `{id}` exposes duplicate tool name `{name}`"
                );
            }

            // Advertised tool definitions must likewise carry non-empty, unique
            // names so the tool schema a client sees is unambiguous.
            let mut def_seen = std::collections::HashSet::new();
            for def in cap.tool_definitions() {
                let name = def.name().to_string();
                assert!(
                    !name.is_empty(),
                    "capability `{id}` advertises a tool definition with an empty name"
                );
                assert!(
                    def_seen.insert(name.clone()),
                    "capability `{id}` advertises duplicate tool definition name `{name}`"
                );
            }
        }
    }

    /// Every built-in production tool must carry backend-authored narration so
    /// downstream clients (e.g. Yolop) render a concise status line instead of
    /// the raw tool-call presentation. A tool is considered covered when its
    /// owning capability's `narrate()` returns `Some` for a representative call,
    /// or when it opts into data-driven CRUD narration via a `narration_noun`
    /// hint. Capabilities whose generic display-name presentation is intentional
    /// are listed in `GENERIC_NARRATION_ALLOWLIST` with a documented reason.
    ///
    /// This is the ratchet the tool-narration audit installs: a newly added
    /// built-in tool that neither narrates nor is allowlisted fails here rather
    /// than silently falling back to raw presentation.
    #[test]
    fn builtin_tools_have_narration_or_documented_generic_fallback() {
        use crate::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
        use crate::tool_types::ToolCall;

        // (capability_id, reason) — whole capabilities whose tools intentionally
        // use the generic display-name presentation. Keep the reason specific.
        const GENERIC_NARRATION_ALLOWLIST: &[(&str, &str)] = &[
            // Demo / eval fixtures — not a production surface.
            (
                "data_knowledge",
                "demo knowledge scaffold; fixture data only",
            ),
            // Platform-admin surface: the mutating `manage_*` tools narrate via
            // `narration_noun` hints; the read/query/messaging tools are a
            // low-frequency operator surface where the display-name presentation
            // ("Read Agents", "Read Sessions") is already clear.
            (
                "platform",
                "operator command surface; tool display names are the intended presentation",
            ),
            (
                "platform_management",
                "operator admin surface; mutations narrate via narration_noun, reads use display names",
            ),
            // Operator model-routing / provider-inspection tooling: specialized,
            // low-frequency, and the display names read clearly on their own.
            (
                "model_scout",
                "operator model-routing tools; display-name presentation is adequate",
            ),
            (
                "openrouter_workspace",
                "operator OpenRouter inspection tools; display-name presentation is adequate",
            ),
            // Arbitrary sandboxed code execution — there is no bounded, non-secret
            // argument worth surfacing; "Run Lua" is the honest status.
            (
                "lua",
                "arbitrary sandboxed code execution; display-name presentation is adequate",
            ),
        ];

        // Exercise the fullest production registry so platform tools, session
        // tasks/schedules, and the SQL/knowledge surfaces are all covered.
        let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
        let ctx = ToolNarrationContext::default();
        let mut missing: Vec<String> = Vec::new();

        for cap in registry.list() {
            let cap_id = cap.id().to_string();
            if GENERIC_NARRATION_ALLOWLIST
                .iter()
                .any(|(id, _)| *id == cap_id)
            {
                continue;
            }

            for tool in cap.tools() {
                let def = tool.to_definition();
                // Data-driven CRUD narration (operation + narration_noun) yields a
                // meaningful line via the generic fallback path.
                if def.hints().narration_noun.is_some() {
                    continue;
                }

                let call = ToolCall {
                    id: "call_narration_audit".to_string(),
                    name: tool.name().to_string(),
                    arguments: serde_json::json!({}),
                };
                // Only the Started phase needs checking: `narrate()` returns
                // `Some`/`None` uniformly across phases for a given tool.
                if cap
                    .narrate(Some(&def), &call, ToolNarrationPhase::Started, None, ctx)
                    .is_none()
                {
                    missing.push(format!("{cap_id}::{}", tool.name()));
                }
            }
        }

        assert!(
            missing.is_empty(),
            "These built-in tools fall back to raw tool-call presentation. Implement \
             `Tool::narrate` (see knowledge/execution/tool-narration.md), set a `narration_noun` hint, \
             or add a documented entry to GENERIC_NARRATION_ALLOWLIST: {missing:?}"
        );
    }

    #[test]
    fn test_capability_registry_blueprint_with_capability() {
        struct BlueprintProviderCapability;

        impl Capability for BlueprintProviderCapability {
            fn id(&self) -> &str {
                "blueprint_provider"
            }
            fn name(&self) -> &str {
                "Blueprint Provider"
            }
            fn description(&self) -> &str {
                "Capability that provides a blueprint for tests"
            }
            fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
                vec![AgentBlueprint {
                    id: "test_blueprint",
                    name: "Test Blueprint",
                    description: "Blueprint for capability registry tests",
                    model: BlueprintModel::Inherit,
                    system_prompt: "Test prompt",
                    tools: vec![],
                    max_turns: None,
                    config_schema: None,
                }]
            }
        }

        let mut registry = CapabilityRegistry::new();
        registry.register(BlueprintProviderCapability);

        let (capability_id, blueprint) = registry
            .blueprint_with_capability("test_blueprint")
            .expect("blueprint should resolve with capability id");
        assert_eq!(capability_id, "blueprint_provider");
        assert_eq!(blueprint.id, "test_blueprint");
    }

    #[test]
    fn test_capability_registry_builder() {
        let registry = CapabilityRegistry::builder()
            .capability(HumanIntentCapability)
            .build();

        assert!(registry.has("human_intent"));
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_capability_status() {
        let registry = CapabilityRegistry::with_builtins();

        let human_intent = registry.get("human_intent").unwrap();
        assert_eq!(human_intent.status(), CapabilityStatus::Available);

        let research = registry.get("research").unwrap();
        assert_eq!(research.status(), CapabilityStatus::ComingSoon);
    }

    #[test]
    fn test_capability_icons_and_categories() {
        let registry = CapabilityRegistry::with_builtins();

        let session = registry.get("session").unwrap();
        assert_eq!(session.icon(), Some("panel-left"));
        assert_eq!(session.category(), Some("Session"));
    }

    #[test]
    fn test_system_prompt_preview_default_delegates_to_addition() {
        // A capability with a static system_prompt_addition — preview should
        // match the addition by default.
        struct StaticPromptCapability;
        impl Capability for StaticPromptCapability {
            fn id(&self) -> &str {
                "static_prompt"
            }
            fn name(&self) -> &str {
                "Static Prompt"
            }
            fn description(&self) -> &str {
                "Static prompt addition."
            }
            fn system_prompt_addition(&self) -> Option<&str> {
                Some("Use the static prompt.")
            }
        }

        let cap = StaticPromptCapability;
        assert_eq!(
            cap.system_prompt_preview().as_deref(),
            cap.system_prompt_addition()
        );

        // current_time has no system_prompt_addition — preview should be None
        let registry = fixture_registry();
        let current_time = registry.get("current_time").unwrap();
        assert!(current_time.system_prompt_preview().is_none());
        assert!(current_time.system_prompt_addition().is_none());
    }

    #[test]
    fn test_system_prompt_preview_dynamic_capability() {
        let registry = fixture_registry();
        let cap = registry.get("agent_instructions").unwrap();

        // No static addition, but preview exists
        assert!(cap.system_prompt_addition().is_none());
        assert!(cap.system_prompt_preview().is_some());
        assert!(cap.system_prompt_preview().unwrap().contains("AGENTS.md"));
    }

    // =========================================================================
    // apply_capabilities tests
    // =========================================================================

    #[tokio::test]
    async fn test_apply_capabilities_empty() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied =
            apply_capabilities(base_runtime_agent.clone(), &[], &registry, &test_ctx()).await;

        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
        );
        assert!(applied.tool_registry.is_empty());
        assert!(applied.applied_ids.is_empty());
    }

    #[tokio::test]
    async fn test_apply_capabilities_noop() {
        let registry = fixture_registry();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["noop".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // Noop has no system prompt addition or tools
        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
        );
        assert!(applied.tool_registry.is_empty());
        assert_eq!(applied.applied_ids, vec!["noop"]);
    }

    #[tokio::test]
    async fn test_apply_capabilities_current_time() {
        let registry = fixture_registry();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["current_time".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // CurrentTime contributes a dynamic `current_time` fact, so the cached
        // prompt gains the explanatory facts note (the live value is appended at
        // the conversation tail per request). It also keeps its tool.
        assert!(
            applied
                .runtime_agent
                .system_prompt
                .contains(FACTS_DYNAMIC_NOTE),
            "current_time should contribute the dynamic-facts note"
        );
        assert!(
            applied
                .runtime_agent
                .system_prompt
                .contains(&base_runtime_agent.system_prompt),
            "base prompt is preserved"
        );
        assert!(applied.tool_registry.has("get_current_time"));
        assert_eq!(applied.tool_registry.len(), 1);
        assert_eq!(applied.applied_ids, vec!["current_time"]);
    }

    #[tokio::test]
    async fn test_apply_capabilities_skips_coming_soon() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        // Research is ComingSoon, so it should be skipped
        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["research".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // System prompt should not have the research addition
        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
        );
        assert!(applied.applied_ids.is_empty()); // Research was not applied
    }

    #[tokio::test]
    async fn test_apply_capabilities_multiple() {
        let registry = fixture_registry();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["noop".to_string(), "current_time".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        assert!(applied.tool_registry.has("get_current_time"));
        assert_eq!(applied.applied_ids, vec!["noop", "current_time"]);
    }

    #[tokio::test]
    async fn test_apply_capabilities_preserves_order() {
        let registry = fixture_registry();
        let base_runtime_agent = RuntimeAgent::new("Base prompt.", "gpt-5.2");

        // Order should be preserved in applied_ids
        let applied = apply_capabilities(
            base_runtime_agent,
            &["current_time".to_string(), "noop".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        assert_eq!(applied.applied_ids, vec!["current_time", "noop"]);
    }

    #[tokio::test]
    async fn test_apply_capabilities_test_math() {
        let registry = fixture_registry();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["test_math".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // TestMath has no system prompt addition (tool defs are sufficient)
        assert!(
            !applied
                .runtime_agent
                .system_prompt
                .contains("<capability id=\"test_math\">")
        );
        // No capability prompt prefix, so base prompt is used as-is (no XML wrapping)
        assert!(
            applied
                .runtime_agent
                .system_prompt
                .contains("You are a helpful assistant.")
        );
        assert!(applied.tool_registry.has("add"));
        assert!(applied.tool_registry.has("subtract"));
        assert!(applied.tool_registry.has("multiply"));
        assert!(applied.tool_registry.has("divide"));
        assert_eq!(applied.tool_registry.len(), 4);
    }

    #[tokio::test]
    async fn test_apply_capabilities_test_weather() {
        let registry = fixture_registry();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["test_weather".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // TestWeather has no system prompt addition (tool defs are sufficient)
        assert!(
            !applied
                .runtime_agent
                .system_prompt
                .contains("<capability id=\"test_weather\">")
        );
        assert!(applied.tool_registry.has("get_weather"));
        assert!(applied.tool_registry.has("get_forecast"));
        assert_eq!(applied.tool_registry.len(), 2);
    }

    #[tokio::test]
    async fn test_apply_capabilities_test_math_and_test_weather() {
        let registry = fixture_registry();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["test_math".to_string(), "test_weather".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // Should have both sets of tools
        assert_eq!(applied.tool_registry.len(), 6); // 4 math + 2 weather
        assert!(applied.tool_registry.has("add"));
        assert!(applied.tool_registry.has("get_weather"));
    }

    #[tokio::test]
    async fn test_apply_capabilities_stateless_todo_list() {
        let registry = fixture_registry();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["stateless_todo_list".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // StatelessTodoList has system prompt addition and 1 tool
        assert!(
            applied
                .runtime_agent
                .system_prompt
                .contains("Task Management")
        );
        assert!(applied.runtime_agent.system_prompt.contains("write_todos"));
        assert!(applied.tool_registry.has("write_todos"));
        assert_eq!(applied.tool_registry.len(), 1);
    }

    // =========================================================================
    // XML prompt formatting tests
    // =========================================================================

    #[tokio::test]
    async fn test_xml_tags_wrap_capability_prompts() {
        let registry = fixture_registry();
        let collected =
            collect_capabilities(&["stateless_todo_list".to_string()], &registry, &test_ctx())
                .await;

        assert_eq!(collected.system_prompt_parts.len(), 1);
        let part = &collected.system_prompt_parts[0];
        assert!(part.starts_with("<capability id=\"stateless_todo_list\">"));
        assert!(part.ends_with("</capability>"));
        assert!(part.contains("Task Management"));
    }

    #[tokio::test]
    async fn test_xml_tags_multiple_capabilities() {
        let registry = fixture_registry();
        let collected = collect_capabilities(
            &[
                "stateless_todo_list".to_string(),
                "session_schedule".to_string(),
            ],
            &registry,
            &test_ctx(),
        )
        .await;

        assert_eq!(collected.system_prompt_parts.len(), 2);
        assert!(
            collected.system_prompt_parts[0].starts_with("<capability id=\"stateless_todo_list\">")
        );
        assert!(
            collected.system_prompt_parts[1].starts_with("<capability id=\"session_schedule\">")
        );

        let prefix = collected.system_prompt_prefix().unwrap();
        // Both capability sections separated by double newline
        assert!(prefix.contains("</capability>\n\n<capability"));
    }

    #[tokio::test]
    async fn test_xml_tags_system_prompt_wrapping() {
        let registry = fixture_registry();
        let base = RuntimeAgent::new("You are helpful.", "gpt-5.2");

        let applied = apply_capabilities(
            base,
            &["stateless_todo_list".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        let prompt = &applied.runtime_agent.system_prompt;
        assert!(prompt.starts_with("<system-prompt>\nYou are helpful.\n</system-prompt>"));
        // Capability wrapped
        assert!(prompt.contains("<capability id=\"stateless_todo_list\">"));
        assert!(prompt.contains("</capability>"));
        // Base prompt wrapped
        assert!(prompt.contains("<system-prompt>\nYou are helpful.\n</system-prompt>"));
    }

    #[tokio::test]
    async fn test_no_xml_wrapping_without_capabilities() {
        let registry = CapabilityRegistry::with_builtins();
        let base = RuntimeAgent::new("You are helpful.", "gpt-5.2");

        let applied = apply_capabilities(base, &[], &registry, &test_ctx()).await;

        // No capabilities = no XML wrapping (plain base prompt)
        assert_eq!(applied.runtime_agent.system_prompt, "You are helpful.");
        assert!(
            !applied
                .runtime_agent
                .system_prompt
                .contains("<system-prompt>")
        );
    }

    #[tokio::test]
    async fn test_no_xml_wrapping_for_noop_capability() {
        let registry = fixture_registry();
        let base = RuntimeAgent::new("You are helpful.", "gpt-5.2");

        // Noop has no system_prompt_addition, so no XML wrapping should occur
        let applied = apply_capabilities(base, &["noop".to_string()], &registry, &test_ctx()).await;

        assert_eq!(applied.runtime_agent.system_prompt, "You are helpful.");
        assert!(
            !applied
                .runtime_agent
                .system_prompt
                .contains("<system-prompt>")
        );
    }

    // =========================================================================
    // Mount collection tests
    // =========================================================================

    #[tokio::test]
    async fn test_collect_capabilities_includes_mounts() {
        let registry = fixture_registry();

        let collected =
            collect_capabilities(&["sample_data".to_string()], &registry, &test_ctx()).await;

        assert!(!collected.mounts.is_empty());
        assert_eq!(collected.mounts.len(), 1);
        assert_eq!(collected.mounts[0].path, "/samples");
        assert!(collected.mounts[0].is_readonly());
    }

    #[tokio::test]
    async fn test_collect_capabilities_empty_mounts_by_default() {
        let registry = fixture_registry();

        // Most capabilities don't have mounts
        let collected =
            collect_capabilities(&["current_time".to_string()], &registry, &test_ctx()).await;

        assert!(collected.mounts.is_empty());
    }

    #[tokio::test]
    async fn test_dynamic_facts_add_note_without_static_block() {
        // `current_time` contributes a Dynamic fact, so the cached prompt gets
        // the explanatory note but NOT a static `<facts>` block (the live value
        // is appended at the conversation tail per request instead).
        let registry = fixture_registry();
        let configs = vec![AgentCapabilityConfig::new("current_time".to_string())];
        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;
        let prompt = collected.system_prompt_parts.join("\n");
        assert!(
            prompt.contains(FACTS_DYNAMIC_NOTE),
            "dynamic-facts note should be in the cached prompt"
        );
        assert!(
            !prompt.contains("<facts>\n"),
            "no static <facts> block for a purely-dynamic fact; got: {prompt}"
        );
    }

    #[tokio::test]
    async fn test_static_facts_fold_into_prompt() {
        struct StaticFactCap;
        impl Capability for StaticFactCap {
            fn id(&self) -> &str {
                "test_static_fact"
            }
            fn name(&self) -> &str {
                "Static Fact"
            }
            fn description(&self) -> &str {
                "test"
            }
            fn status(&self) -> CapabilityStatus {
                CapabilityStatus::Available
            }
            fn facts(&self, _config: &serde_json::Value, _ctx: &FactsContext) -> Vec<Fact> {
                vec![Fact::stat("workspace_root", "/workspace")]
            }
        }
        let mut registry = CapabilityRegistry::new();
        registry.register(StaticFactCap);
        let configs = vec![AgentCapabilityConfig::new("test_static_fact".to_string())];
        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;
        let prompt = collected.system_prompt_parts.join("\n");
        assert!(
            prompt.contains("<facts>\n- workspace_root: /workspace\n</facts>"),
            "static fact should fold into the cached prompt; got: {prompt}"
        );
        assert!(
            !prompt.contains(FACTS_DYNAMIC_NOTE),
            "no dynamic note when only static facts exist"
        );
    }

    #[test]
    fn test_collect_dynamic_facts_returns_current_time() {
        let registry = fixture_registry();
        let configs = vec![AgentCapabilityConfig::new("current_time".to_string())];
        let facts = collect_dynamic_facts(
            &configs,
            &registry,
            None,
            &FactsContext::new(SessionId::new()),
        );
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].key, "current_time");
        assert_eq!(facts[0].volatility, Volatility::Dynamic);
    }

    #[tokio::test]
    async fn test_collect_capabilities_combines_mounts() {
        let registry = fixture_registry();

        // Collect from multiple capabilities - only sample_data has mounts.
        // sample_data depends on session_file_system, which is auto-resolved.
        let collected = collect_capabilities(
            &["sample_data".to_string(), "current_time".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        assert_eq!(collected.mounts.len(), 1);
        // Verify expected capabilities were applied (including auto-resolved dependency)
        assert!(
            collected
                .applied_ids
                .iter()
                .any(|id| id == "session_file_system")
        );
        assert!(collected.applied_ids.iter().any(|id| id == "sample_data"));
        assert!(collected.applied_ids.iter().any(|id| id == "current_time"));
    }

    #[test]
    fn test_sample_data_capability() {
        let registry = fixture_registry();
        let cap = registry.get("sample_data").unwrap();

        assert_eq!(cap.id(), "sample_data");
        assert_eq!(cap.name(), "Sample Data");
        assert_eq!(cap.status(), CapabilityStatus::Available);

        // Has system prompt but no tools
        assert!(cap.system_prompt_addition().is_some());
        assert!(cap.tools().is_empty());

        // Has mounts
        assert!(!cap.mounts().is_empty());
    }

    // =========================================================================
    // Dependency resolution tests
    // =========================================================================

    #[test]
    fn test_resolve_dependencies_empty() {
        let registry = CapabilityRegistry::with_builtins();

        let resolved = resolve_dependencies(&[], &registry).unwrap();

        assert!(resolved.resolved_ids.is_empty());
        assert!(resolved.added_as_dependencies.is_empty());
        assert!(resolved.user_selected.is_empty());
    }

    #[test]
    fn test_resolve_dependencies_no_deps() {
        let registry = fixture_registry();

        // CurrentTime has no dependencies
        let resolved = resolve_dependencies(&["current_time".to_string()], &registry).unwrap();

        assert_eq!(resolved.resolved_ids, vec!["current_time"]);
        assert!(resolved.added_as_dependencies.is_empty());
    }

    #[test]
    fn test_resolve_dependencies_with_deps() {
        let registry = fixture_registry();

        // SampleData depends on FileSystem
        let resolved = resolve_dependencies(&["sample_data".to_string()], &registry).unwrap();

        // FileSystem should be resolved before SampleData
        assert_eq!(resolved.resolved_ids.len(), 2);
        let fs_pos = resolved
            .resolved_ids
            .iter()
            .position(|id| id == "session_file_system")
            .unwrap();
        let sd_pos = resolved
            .resolved_ids
            .iter()
            .position(|id| id == "sample_data")
            .unwrap();
        assert!(fs_pos < sd_pos, "FileSystem should come before SampleData");

        // FileSystem was added as a dependency
        assert_eq!(resolved.added_as_dependencies, vec!["session_file_system"]);
    }

    #[test]
    fn test_resolve_dependencies_already_selected() {
        let registry = fixture_registry();

        // If dependency is already selected, it shouldn't be duplicated
        let resolved = resolve_dependencies(
            &["session_file_system".to_string(), "sample_data".to_string()],
            &registry,
        )
        .unwrap();

        assert_eq!(resolved.resolved_ids.len(), 2);
        // FileSystem was user-selected, not added as dependency
        assert!(resolved.added_as_dependencies.is_empty());
    }

    #[test]
    fn test_resolve_dependencies_preserves_order() {
        let registry = fixture_registry();

        // Multiple independent capabilities should maintain their relative order
        let resolved =
            resolve_dependencies(&["current_time".to_string(), "noop".to_string()], &registry)
                .unwrap();

        assert_eq!(resolved.resolved_ids, vec!["current_time", "noop"]);
    }

    #[test]
    fn test_resolve_dependencies_unknown_capability() {
        let registry = CapabilityRegistry::with_builtins();

        // Unknown capabilities are silently skipped
        let resolved =
            resolve_dependencies(&["unknown_capability".to_string()], &registry).unwrap();

        assert!(resolved.resolved_ids.is_empty());
    }

    #[test]
    fn test_get_dependencies() {
        let registry = fixture_registry();

        // SampleData depends on FileSystem
        let deps = get_dependencies("sample_data", &registry);
        assert_eq!(deps, vec!["session_file_system"]);

        // CurrentTime has no dependencies
        let deps = get_dependencies("current_time", &registry);
        assert!(deps.is_empty());

        // Unknown capability
        let deps = get_dependencies("unknown", &registry);
        assert!(deps.is_empty());
    }

    #[test]
    fn test_sample_data_has_dependency() {
        let registry = fixture_registry();
        let cap = registry.get("sample_data").unwrap();

        let deps = cap.dependencies();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], "session_file_system");
    }

    #[test]
    fn test_noop_has_no_dependencies() {
        let registry = fixture_registry();
        let cap = registry.get("noop").unwrap();

        assert!(cap.dependencies().is_empty());
    }

    // Test for circular dependency detection
    // Note: We can't easily test this with built-in capabilities since they don't have cycles.
    // This test uses a custom registry to create a cycle.
    #[test]
    fn test_circular_dependency_error() {
        // Create capabilities that form a cycle: A -> B -> A
        struct CapA;
        struct CapB;

        impl Capability for CapA {
            fn id(&self) -> &str {
                "test_cap_a"
            }
            fn name(&self) -> &str {
                "Test A"
            }
            fn description(&self) -> &str {
                "Test capability A"
            }
            fn dependencies(&self) -> Vec<&'static str> {
                vec!["test_cap_b"]
            }
        }

        impl Capability for CapB {
            fn id(&self) -> &str {
                "test_cap_b"
            }
            fn name(&self) -> &str {
                "Test B"
            }
            fn description(&self) -> &str {
                "Test capability B"
            }
            fn dependencies(&self) -> Vec<&'static str> {
                vec!["test_cap_a"]
            }
        }

        let mut registry = CapabilityRegistry::new();
        registry.register(CapA);
        registry.register(CapB);

        let result = resolve_dependencies(&["test_cap_a".to_string()], &registry);

        assert!(result.is_err());
        match result.unwrap_err() {
            DependencyError::CircularDependency { capability_id, .. } => {
                assert_eq!(capability_id, "test_cap_a");
            }
            _ => panic!("Expected CircularDependency error"),
        }
    }

    // =========================================================================
    // Message filter provider tests
    // =========================================================================

    use crate::message_filter::{MessageFilter, MessageFilterProvider, MessageQuery};

    /// Test capability that provides a message filter
    struct FilterTestCapability {
        priority: i32,
    }

    impl Capability for FilterTestCapability {
        fn id(&self) -> &str {
            "filter_test"
        }
        fn name(&self) -> &str {
            "Filter Test"
        }
        fn description(&self) -> &str {
            "Test capability with message filter"
        }
        fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
            Some(Arc::new(FilterTestProvider {
                priority: self.priority,
            }))
        }
    }

    struct FilterTestProvider {
        priority: i32,
    }

    impl MessageFilterProvider for FilterTestProvider {
        fn apply_filters(&self, query: &mut MessageQuery, config: &serde_json::Value) {
            // Add a search filter based on config
            if let Some(search) = config.get("search").and_then(|v| v.as_str()) {
                query
                    .filters
                    .push(MessageFilter::Search(search.to_string()));
            }
        }

        fn priority(&self) -> i32 {
            self.priority
        }
    }

    #[tokio::test]
    async fn test_collect_capabilities_with_configs_no_filter_providers() {
        let registry = fixture_registry();
        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("current_time"),
            serde_json::json!({}),
        )];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        assert!(collected.message_filter_providers.is_empty());
        assert!(!collected.has_message_filters());
    }

    #[tokio::test]
    async fn test_collect_capabilities_with_configs_with_filter_provider() {
        let mut registry = CapabilityRegistry::new();
        registry.register(FilterTestCapability { priority: 0 });

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("filter_test"),
            serde_json::json!({ "search": "hello" }),
        )];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        assert_eq!(collected.message_filter_providers.len(), 1);
        assert!(collected.has_message_filters());
    }

    #[tokio::test]
    async fn test_collect_capabilities_with_configs_filter_priority_order() {
        // Create capabilities with different priorities
        struct HighPriorityCapability;
        struct LowPriorityCapability;

        impl Capability for HighPriorityCapability {
            fn id(&self) -> &str {
                "high_priority"
            }
            fn name(&self) -> &str {
                "High Priority"
            }
            fn description(&self) -> &str {
                "Test"
            }
            fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
                Some(Arc::new(FilterTestProvider { priority: 10 }))
            }
        }

        impl Capability for LowPriorityCapability {
            fn id(&self) -> &str {
                "low_priority"
            }
            fn name(&self) -> &str {
                "Low Priority"
            }
            fn description(&self) -> &str {
                "Test"
            }
            fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
                Some(Arc::new(FilterTestProvider { priority: -5 }))
            }
        }

        let mut registry = CapabilityRegistry::new();
        registry.register(HighPriorityCapability);
        registry.register(LowPriorityCapability);

        // Add in order: high priority first, low priority second
        let configs = vec![
            AgentCapabilityConfig::with_config(
                CapabilityId::new("high_priority"),
                serde_json::json!({}),
            ),
            AgentCapabilityConfig::with_config(
                CapabilityId::new("low_priority"),
                serde_json::json!({}),
            ),
        ];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        // Should be sorted by priority (lower first)
        assert_eq!(collected.message_filter_providers.len(), 2);
        assert_eq!(collected.message_filter_providers[0].0.priority(), -5);
        assert_eq!(collected.message_filter_providers[1].0.priority(), 10);
    }

    #[tokio::test]
    async fn test_collected_capabilities_apply_message_filters() {
        let mut registry = CapabilityRegistry::new();
        registry.register(FilterTestCapability { priority: 0 });

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("filter_test"),
            serde_json::json!({ "search": "test_query" }),
        )];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        // Apply filters to a query
        let session_id: SessionId = Uuid::now_v7().into();
        let mut query = MessageQuery::new(session_id);

        collected.apply_message_filters(&mut query);

        // Should have added the search filter
        assert_eq!(query.filters.len(), 1);
        assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "test_query"));
    }

    #[tokio::test]
    async fn test_collected_capabilities_apply_multiple_filters_in_priority_order() {
        struct SearchCapability {
            id: &'static str,
            search_term: &'static str,
            priority: i32,
        }

        struct SearchProvider {
            search_term: &'static str,
            priority: i32,
        }

        impl MessageFilterProvider for SearchProvider {
            fn apply_filters(&self, query: &mut MessageQuery, _config: &serde_json::Value) {
                query
                    .filters
                    .push(MessageFilter::Search(self.search_term.to_string()));
            }

            fn priority(&self) -> i32 {
                self.priority
            }
        }

        impl Capability for SearchCapability {
            fn id(&self) -> &str {
                self.id
            }
            fn name(&self) -> &str {
                "Search"
            }
            fn description(&self) -> &str {
                "Test"
            }
            fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
                Some(Arc::new(SearchProvider {
                    search_term: self.search_term,
                    priority: self.priority,
                }))
            }
        }

        let mut registry = CapabilityRegistry::new();
        registry.register(SearchCapability {
            id: "cap_a",
            search_term: "alpha",
            priority: 5,
        });
        registry.register(SearchCapability {
            id: "cap_b",
            search_term: "beta",
            priority: 1,
        });
        registry.register(SearchCapability {
            id: "cap_c",
            search_term: "gamma",
            priority: 10,
        });

        let configs = vec![
            AgentCapabilityConfig::with_config(CapabilityId::new("cap_a"), serde_json::json!({})),
            AgentCapabilityConfig::with_config(CapabilityId::new("cap_b"), serde_json::json!({})),
            AgentCapabilityConfig::with_config(CapabilityId::new("cap_c"), serde_json::json!({})),
        ];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        let session_id: SessionId = Uuid::now_v7().into();
        let mut query = MessageQuery::new(session_id);

        collected.apply_message_filters(&mut query);

        // Filters should be applied in priority order: beta (1), alpha (5), gamma (10)
        assert_eq!(query.filters.len(), 3);
        assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "beta"));
        assert!(matches!(&query.filters[1], MessageFilter::Search(s) if s == "alpha"));
        assert!(matches!(&query.filters[2], MessageFilter::Search(s) if s == "gamma"));
    }

    #[test]
    fn test_capability_without_message_filter_returns_none() {
        let registry = fixture_registry();

        let noop = registry.get("noop").unwrap();
        assert!(noop.message_filter_provider().is_none());

        let current_time = registry.get("current_time").unwrap();
        assert!(current_time.message_filter_provider().is_none());
    }

    #[tokio::test]
    async fn test_collect_capabilities_preserves_config_for_filter_provider() {
        let mut registry = CapabilityRegistry::new();
        registry.register(FilterTestCapability { priority: 0 });

        let test_config = serde_json::json!({
            "search": "custom_search",
            "extra_field": 42
        });

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("filter_test"),
            test_config.clone(),
        )];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        // Verify the config is preserved
        assert_eq!(collected.message_filter_providers.len(), 1);
        let (_, stored_config) = &collected.message_filter_providers[0];
        assert_eq!(*stored_config, test_config);
    }

    // =========================================================================
    // collect_message_filters_only tests
    // =========================================================================

    #[test]
    fn test_collect_message_filters_only_collects_filters() {
        let mut registry = CapabilityRegistry::new();
        registry.register(FilterTestCapability { priority: 0 });

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("filter_test"),
            serde_json::json!({ "search": "test_query" }),
        )];

        let collected = collect_message_filters_only(&configs, &registry);

        let session_id: SessionId = Uuid::now_v7().into();
        let mut query = MessageQuery::new(session_id);
        collected.apply_message_filters(&mut query);

        assert_eq!(query.filters.len(), 1);
        assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "test_query"));
    }

    #[test]
    fn test_message_filter_config_injects_compaction_active_for_infinity_context() {
        let base = serde_json::json!({ "context_budget_tokens": 1000 });

        // Infinity context gets the derived flag only when compaction is enabled.
        let with = message_filter_config_for(INFINITY_CONTEXT_CAPABILITY_ID, &base, true);
        assert_eq!(with["compaction_active"], serde_json::json!(true));
        assert_eq!(with["context_budget_tokens"], serde_json::json!(1000));

        let without = message_filter_config_for(INFINITY_CONTEXT_CAPABILITY_ID, &base, false);
        assert!(without.get("compaction_active").is_none());

        // Other capabilities are never touched.
        let other = message_filter_config_for("other", &base, true);
        assert!(other.get("compaction_active").is_none());

        // A null base is upgraded to an object carrying the flag.
        let null_base = message_filter_config_for(
            INFINITY_CONTEXT_CAPABILITY_ID,
            &serde_json::Value::Null,
            true,
        );
        assert_eq!(null_base["compaction_active"], serde_json::json!(true));
    }

    #[test]
    fn test_collect_message_filters_only_skips_unknown_capabilities() {
        let registry = CapabilityRegistry::new();

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("nonexistent"),
            serde_json::json!({}),
        )];

        let collected = collect_message_filters_only(&configs, &registry);
        assert!(collected.message_filter_providers.is_empty());
    }

    #[test]
    fn test_collect_message_filters_only_preserves_priority_order() {
        struct PriorityFilterCap {
            id: &'static str,
            search_term: &'static str,
            priority: i32,
        }

        struct PriorityFilterProvider {
            search_term: &'static str,
            priority: i32,
        }

        impl Capability for PriorityFilterCap {
            fn id(&self) -> &str {
                self.id
            }
            fn name(&self) -> &str {
                self.id
            }
            fn description(&self) -> &str {
                "priority test"
            }
            fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
                Some(Arc::new(PriorityFilterProvider {
                    search_term: self.search_term,
                    priority: self.priority,
                }))
            }
        }

        impl MessageFilterProvider for PriorityFilterProvider {
            fn apply_filters(&self, query: &mut MessageQuery, _config: &serde_json::Value) {
                query
                    .filters
                    .push(MessageFilter::Search(self.search_term.to_string()));
            }
            fn priority(&self) -> i32 {
                self.priority
            }
        }

        let mut registry = CapabilityRegistry::new();
        registry.register(PriorityFilterCap {
            id: "gamma",
            search_term: "gamma",
            priority: 10,
        });
        registry.register(PriorityFilterCap {
            id: "alpha",
            search_term: "alpha",
            priority: 5,
        });
        registry.register(PriorityFilterCap {
            id: "beta",
            search_term: "beta",
            priority: 1,
        });

        let configs = vec![
            AgentCapabilityConfig::with_config(CapabilityId::new("gamma"), serde_json::json!({})),
            AgentCapabilityConfig::with_config(CapabilityId::new("alpha"), serde_json::json!({})),
            AgentCapabilityConfig::with_config(CapabilityId::new("beta"), serde_json::json!({})),
        ];

        let collected = collect_message_filters_only(&configs, &registry);

        let session_id: SessionId = Uuid::now_v7().into();
        let mut query = MessageQuery::new(session_id);
        collected.apply_message_filters(&mut query);

        // Filters should be applied in priority order: beta (1), alpha (5), gamma (10)
        assert_eq!(query.filters.len(), 3);
        assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "beta"));
        assert!(matches!(&query.filters[1], MessageFilter::Search(s) if s == "alpha"));
        assert!(matches!(&query.filters[2], MessageFilter::Search(s) if s == "gamma"));
    }

    #[test]
    fn test_collect_message_filters_only_post_load_invoked() {
        use crate::message::Message;

        struct PostLoadCap;
        struct PostLoadProvider;

        impl Capability for PostLoadCap {
            fn id(&self) -> &str {
                "post_load_test"
            }
            fn name(&self) -> &str {
                "PostLoad Test"
            }
            fn description(&self) -> &str {
                "test"
            }
            fn message_filter_provider(&self) -> Option<Arc<dyn MessageFilterProvider>> {
                Some(Arc::new(PostLoadProvider))
            }
        }

        impl MessageFilterProvider for PostLoadProvider {
            fn apply_filters(&self, _query: &mut MessageQuery, _config: &serde_json::Value) {}
            fn priority(&self) -> i32 {
                0
            }
            fn post_load(&self, messages: &mut Vec<Message>, _config: &serde_json::Value) {
                // Reverse messages to prove post_load was called
                messages.reverse();
            }
        }

        let mut registry = CapabilityRegistry::new();
        registry.register(PostLoadCap);

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("post_load_test"),
            serde_json::json!({}),
        )];

        let collected = collect_message_filters_only(&configs, &registry);

        let mut messages = vec![Message::user("first"), Message::user("second")];
        collected.apply_post_load_filters(&mut messages);

        // post_load reversed the messages
        assert_eq!(messages[0].text(), Some("second"));
        assert_eq!(messages[1].text(), Some("first"));
    }

    // Tests for resolve_for_model delegation in fast-path collectors

    struct DelegatingFilterCap {
        id: &'static str,
        inner: std::sync::Arc<InnerFilterCap>,
    }
    struct InnerFilterCap;

    impl Capability for InnerFilterCap {
        fn id(&self) -> &str {
            "inner_filter"
        }
        fn name(&self) -> &str {
            "Inner Filter"
        }
        fn description(&self) -> &str {
            "inner"
        }
        fn message_filter_provider(&self) -> Option<std::sync::Arc<dyn MessageFilterProvider>> {
            Some(std::sync::Arc::new(SentinelFilter))
        }
    }
    struct SentinelFilter;
    impl MessageFilterProvider for SentinelFilter {
        fn apply_filters(&self, _query: &mut MessageQuery, _config: &serde_json::Value) {}
    }
    impl Capability for DelegatingFilterCap {
        fn id(&self) -> &str {
            self.id
        }
        fn name(&self) -> &str {
            "Delegating Filter"
        }
        fn description(&self) -> &str {
            "delegating"
        }
        fn message_filter_provider(&self) -> Option<std::sync::Arc<dyn MessageFilterProvider>> {
            None // outer provides nothing
        }
        fn resolve_for_model(&self, _model: Option<&str>) -> Option<&dyn Capability> {
            Some(&*self.inner)
        }
    }

    #[test]
    fn test_collect_message_filters_only_honors_resolve_for_model_delegation() {
        let inner = std::sync::Arc::new(InnerFilterCap);
        let outer = DelegatingFilterCap {
            id: "delegating_filter",
            inner: inner.clone(),
        };

        let mut registry = CapabilityRegistry::new();
        registry.register(outer);

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("delegating_filter"),
            serde_json::json!({}),
        )];

        // Outer has no message_filter_provider; inner does. resolve_for_model
        // delegates to inner so the provider should be collected.
        let collected = collect_message_filters_only(&configs, &registry);
        assert_eq!(
            collected.message_filter_providers.len(),
            1,
            "provider from resolved inner capability must be collected"
        );
    }

    struct DelegatingMvpCap {
        id: &'static str,
        inner: std::sync::Arc<InnerMvpCap>,
    }
    struct InnerMvpCap;

    impl Capability for InnerMvpCap {
        fn id(&self) -> &str {
            "inner_mvp"
        }
        fn name(&self) -> &str {
            "Inner MVP"
        }
        fn description(&self) -> &str {
            "inner"
        }
        fn model_view_provider(
            &self,
        ) -> Option<std::sync::Arc<dyn crate::capabilities::ModelViewProvider>> {
            // Return a no-op provider to prove delegation reached here.
            struct NoopMvp;
            impl crate::capabilities::ModelViewProvider for NoopMvp {
                fn apply_model_view(
                    &self,
                    messages: Vec<Message>,
                    _config: &serde_json::Value,
                    _context: &ModelViewContext<'_>,
                ) -> Vec<Message> {
                    messages
                }
            }
            Some(std::sync::Arc::new(NoopMvp))
        }
    }
    impl Capability for DelegatingMvpCap {
        fn id(&self) -> &str {
            self.id
        }
        fn name(&self) -> &str {
            "Delegating MVP"
        }
        fn description(&self) -> &str {
            "delegating"
        }
        fn model_view_provider(
            &self,
        ) -> Option<std::sync::Arc<dyn crate::capabilities::ModelViewProvider>> {
            None // outer provides nothing
        }
        fn resolve_for_model(&self, _model: Option<&str>) -> Option<&dyn Capability> {
            Some(&*self.inner)
        }
    }

    #[test]
    fn test_collect_model_view_providers_honors_resolve_for_model_delegation() {
        let inner = std::sync::Arc::new(InnerMvpCap);
        let outer = DelegatingMvpCap {
            id: "delegating_mvp",
            inner: inner.clone(),
        };

        let mut registry = CapabilityRegistry::new();
        registry.register(outer);

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("delegating_mvp"),
            serde_json::json!({}),
        )];

        // Outer has no model_view_provider; inner does. resolve_for_model
        // delegates to inner so the provider should be collected.
        let collected = collect_model_view_providers(&configs, &registry, None);
        assert_eq!(
            collected.model_view_providers.len(),
            1,
            "provider from resolved inner capability must be collected"
        );
    }

    // =========================================================================
    // Harness capability tool registration tests
    //
    // Regression tests for the "Tool not found: bash" bug where harness
    // capabilities were not used for tool registration when agent_id was absent.
    // These tests verify that capability-provided tools (especially bash) are
    // correctly produced by collect_capabilities.
    // =========================================================================

    #[tokio::test]
    async fn test_bashkit_shell_capability_produces_bash_tool() {
        let registry = fixture_registry();
        let collected =
            collect_capabilities(&["bashkit_shell".to_string()], &registry, &test_ctx()).await;

        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();
        assert!(
            tool_names.contains(&"bash"),
            "bashkit_shell capability must produce 'bash' tool, got: {:?}",
            tool_names
        );
        assert!(
            !collected.tools.is_empty(),
            "bashkit_shell must provide tool implementations"
        );
    }

    #[tokio::test]
    async fn test_generic_harness_capability_set_produces_bash_tool() {
        // These are the exact capability IDs from the Generic Harness seed data.
        // If any are renamed or removed, this test catches the regression.
        let generic_harness_caps = vec![
            "session_file_system".to_string(),
            "bashkit_shell".to_string(),
            "web_fetch".to_string(),
            "session_storage".to_string(),
            "session".to_string(),
            "agent_instructions".to_string(),
            "skills".to_string(),
            "infinity_context".to_string(),
            "auto_tool_search".to_string(),
        ];

        let registry = fixture_registry();
        let collected = collect_capabilities(&generic_harness_caps, &registry, &test_ctx()).await;

        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();
        assert!(
            tool_names.contains(&"bash"),
            "Generic Harness capabilities must produce 'bash' tool, got: {:?}",
            tool_names
        );
    }

    #[tokio::test]
    async fn test_collect_capabilities_tool_count_matches_definitions() {
        // Ensure collected tools (implementations) match tool_definitions count.
        // A mismatch means some tools won't be executable at runtime.
        let registry = fixture_registry();
        let collected =
            collect_capabilities(&["bashkit_shell".to_string()], &registry, &test_ctx()).await;

        assert_eq!(
            collected.tools.len(),
            collected.tool_definitions.len(),
            "tool implementations ({}) must match tool definitions ({})",
            collected.tools.len(),
            collected.tool_definitions.len(),
        );
    }

    /// Regression test for EVE-189: collect_capabilities must resolve dependencies
    /// so that transitive capabilities register their tools even when not explicitly
    /// listed. Uses sample_data (depends on session_file_system) as the test case.
    #[tokio::test]
    async fn test_collect_capabilities_resolves_dependencies() {
        // sample_data depends on session_file_system
        // Passing only sample_data should still include session_file_system tools
        let registry = fixture_registry();
        let collected =
            collect_capabilities(&["sample_data".to_string()], &registry, &test_ctx()).await;

        // Verify the transitive dependency capability itself was applied
        assert!(
            collected
                .applied_ids
                .iter()
                .any(|id| id == "session_file_system"),
            "collect_capabilities must apply session_file_system as a dependency; applied_ids: {:?}",
            collected.applied_ids
        );

        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();

        // session_file_system provides these tools; both should be present
        assert!(
            tool_names.contains(&"read_file") && tool_names.contains(&"write_file"),
            "collect_capabilities must resolve dependencies and include dependency tools, got: {:?}",
            tool_names
        );

        // Also verify tool implementations match definitions (dependency tools are executable)
        assert_eq!(
            collected.tools.len(),
            collected.tool_definitions.len(),
            "dependency-added tools must have implementations, not just definitions"
        );
    }

    #[test]
    fn test_defaults_do_not_include_bash() {
        // ToolRegistry::with_defaults() must NOT include bash — it comes from
        // capabilities only. This documents the invariant that the bug violated.
        let registry = crate::ToolRegistry::with_defaults();
        assert!(
            !registry.has("bash"),
            "with_defaults() must not include 'bash' — it comes from bashkit_shell capability"
        );
    }

    // =========================================================================
    // EVE-501: background_execution auto-activation
    // =========================================================================

    /// Auto-activation: any collected tool with `supports_background=true`
    /// causes `spawn_background` to appear in both tool_definitions and tools.
    #[tokio::test]
    async fn test_background_execution_auto_activates_with_bashkit_shell() {
        let registry = fixture_registry();
        let collected =
            collect_capabilities(&["bashkit_shell".to_string()], &registry, &test_ctx()).await;

        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();
        assert!(
            tool_names.contains(&"spawn_background"),
            "spawn_background must be auto-activated when bashkit_shell (a \
             background-capable tool) is in the agent's capability set; got: {:?}",
            tool_names
        );
        assert!(
            collected
                .applied_ids
                .iter()
                .any(|id| id == BACKGROUND_EXECUTION_CAPABILITY_ID),
            "background_execution must be in applied_ids when auto-activated; \
             got: {:?}",
            collected.applied_ids
        );

        // Lockstep: implementations match definitions (executable in the worker).
        assert!(
            collected
                .tools
                .iter()
                .any(|t| t.name() == "spawn_background"),
            "spawn_background tool implementation must be present alongside the \
             definition (lockstep contract)"
        );
    }

    /// Negative: when no collected tool declares background support, the
    /// capability must NOT auto-activate.
    #[tokio::test]
    async fn test_background_execution_does_not_auto_activate_without_hint() {
        let registry = fixture_registry();
        // current_time has no background-capable tool.
        let collected =
            collect_capabilities(&["current_time".to_string()], &registry, &test_ctx()).await;

        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();
        assert!(
            !tool_names.contains(&"spawn_background"),
            "spawn_background must NOT be activated without a background-capable \
             tool; got: {:?}",
            tool_names
        );
        assert!(
            !collected
                .applied_ids
                .iter()
                .any(|id| id == BACKGROUND_EXECUTION_CAPABILITY_ID),
            "background_execution must not appear in applied_ids when no \
             background-capable tool is present; got: {:?}",
            collected.applied_ids
        );
    }

    #[tokio::test]
    async fn test_subagents_collect_unified_spawn_agent_adapter() {
        let registry = CapabilityRegistry::with_builtins();
        let collected = collect_capabilities(
            &[SUBAGENTS_CAPABILITY_ID.to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        assert!(
            collected
                .tools
                .iter()
                .any(|tool| tool.name() == "spawn_agent"),
            "subagent-only sessions should get the unified spawn_agent adapter"
        );
        let spawn_agent = collected
            .tool_definitions
            .iter()
            .find(|tool| tool.name() == "spawn_agent")
            .expect("spawn_agent definition");
        assert_eq!(
            spawn_agent.parameters()["properties"]["target"]["properties"]["type"]["enum"],
            serde_json::json!(["subagent"])
        );
        assert_eq!(
            spawn_agent.concurrency_class(),
            Some(SPAWN_AGENT_CONCURRENCY_CLASS),
            "unified spawn_agent must serialize same-batch spawns before cap checks"
        );
    }

    #[tokio::test]
    async fn test_agent_handoff_collects_unified_spawn_agent_adapter() {
        let mut registry = CapabilityRegistry::new();
        registry.register(AgentHandoffCapability);
        let agent_id = crate::typed_id::AgentId::new();
        let harness_id = crate::typed_id::HarnessId::new();
        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new(AGENT_HANDOFF_CAPABILITY_ID),
            serde_json::json!({
                "targets": [{
                    "id": "aws_operator",
                    "name": "AWS Operator",
                    "agent_id": agent_id,
                    "harness_id": harness_id
                }]
            }),
        )];
        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        assert!(
            collected
                .tools
                .iter()
                .any(|tool| tool.name() == "spawn_agent"),
            "agent_handoff-only sessions should get the unified spawn_agent adapter"
        );
        let spawn_agent = collected
            .tool_definitions
            .iter()
            .find(|tool| tool.name() == "spawn_agent")
            .expect("spawn_agent definition");
        assert_eq!(
            spawn_agent.parameters()["properties"]["target"]["properties"]["type"]["enum"],
            serde_json::json!(["agent"])
        );
    }

    #[tokio::test]
    async fn test_spawn_agent_dispatcher_combines_known_target_providers() {
        let mut registry = CapabilityRegistry::new();
        registry.register(SubagentCapability);
        registry.register(AgentHandoffCapability);

        let agent_id = crate::typed_id::AgentId::new();
        let harness_id = crate::typed_id::HarnessId::new();
        let configs = vec![
            AgentCapabilityConfig::with_config(
                CapabilityId::new(SUBAGENTS_CAPABILITY_ID),
                serde_json::json!({}),
            ),
            AgentCapabilityConfig::with_config(
                CapabilityId::new(AGENT_HANDOFF_CAPABILITY_ID),
                serde_json::json!({
                    "targets": [{
                        "id": "aws_operator",
                        "name": "AWS Operator",
                        "agent_id": agent_id,
                        "harness_id": harness_id
                    }]
                }),
            ),
        ];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;
        let spawn_agent_defs: Vec<_> = collected
            .tool_definitions
            .iter()
            .filter(|tool| tool.name() == "spawn_agent")
            .collect();

        assert_eq!(spawn_agent_defs.len(), 1);
        let schema = spawn_agent_defs[0].parameters();
        assert_eq!(
            schema["properties"]["target"]["properties"]["type"]["enum"],
            serde_json::json!(["subagent", "agent"])
        );
        // Anthropic rejects top-level oneOf/allOf/anyOf in input_schema, so
        // the per-target constraints must live inside the target property.
        assert!(schema.get("oneOf").is_none());
        assert!(schema.get("anyOf").is_none());
        assert!(schema.get("allOf").is_none());
        assert_eq!(
            schema["required"],
            serde_json::json!(["name", "instructions", "target"])
        );
        assert_eq!(
            schema["properties"]["target"]["oneOf"],
            serde_json::json!([
                {
                    "properties": {"type": {"const": "subagent"}}
                },
                {
                    "properties": {"type": {"const": "agent"}},
                    "required": ["type", "id"]
                }
            ])
        );
    }

    #[cfg(feature = "a2a")]
    #[tokio::test]
    async fn test_spawn_agent_dispatcher_includes_external_a2a_provider() {
        let mut registry = CapabilityRegistry::new();
        registry.register(SubagentCapability);
        registry.register(A2aAgentDelegationCapability);

        let configs = vec![
            AgentCapabilityConfig::with_config(
                CapabilityId::new(SUBAGENTS_CAPABILITY_ID),
                serde_json::json!({}),
            ),
            AgentCapabilityConfig::with_config(
                CapabilityId::new(A2A_AGENT_DELEGATION_CAPABILITY_ID),
                serde_json::json!({
                    "agents": [{
                        "id": "local_app",
                        "name": "Local App",
                        "base_url": "https://example.com"
                    }]
                }),
            ),
        ];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;
        let spawn_agent_defs: Vec<_> = collected
            .tool_definitions
            .iter()
            .filter(|tool| tool.name() == "spawn_agent")
            .collect();

        assert_eq!(spawn_agent_defs.len(), 1);
        assert_eq!(
            spawn_agent_defs[0].parameters()["properties"]["target"]["properties"]["type"]["enum"],
            serde_json::json!(["subagent", "external_a2a"])
        );
        assert_eq!(
            spawn_agent_defs[0].parameters()["properties"]["mode"]["enum"],
            serde_json::json!(["background", "foreground"])
        );
        assert!(
            !spawn_agent_defs[0].parameters()["properties"]["mode"]["description"]
                .as_str()
                .expect("mode description")
                .contains("wait")
        );
        let schema = spawn_agent_defs[0].parameters();
        assert!(schema.get("oneOf").is_none());
        // name is required at the root even with external_a2a present: the
        // local providers demand it and requiring a field external_a2a ignores
        // is safe, whereas top-level conditional requirements are rejected.
        assert_eq!(
            schema["required"],
            serde_json::json!(["name", "instructions", "target"])
        );
        assert_eq!(
            schema["properties"]["target"]["oneOf"],
            serde_json::json!([
                {
                    "properties": {"type": {"const": "subagent"}}
                },
                {
                    "properties": {"type": {"const": "external_a2a"}},
                    "anyOf": [
                        {"required": ["id"]},
                        {"required": ["external_agent_id"]}
                    ]
                }
            ])
        );
    }

    struct ExistingSpawnAgentCapability;

    impl Capability for ExistingSpawnAgentCapability {
        fn id(&self) -> &str {
            "existing_spawn_agent"
        }

        fn name(&self) -> &str {
            "Existing Spawn Agent"
        }

        fn description(&self) -> &str {
            "Test capability that already owns spawn_agent"
        }

        fn tools(&self) -> Vec<Box<dyn Tool>> {
            vec![Box::new(ExistingSpawnAgentTool)]
        }
    }

    struct ExistingSpawnAgentTool;

    #[async_trait]
    impl Tool for ExistingSpawnAgentTool {
        fn name(&self) -> &str {
            "spawn_agent"
        }

        fn description(&self) -> &str {
            "Existing spawn_agent test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "object",
                        "properties": {
                            "type": {"type": "string", "enum": ["external_a2a"]}
                        },
                        "required": ["type"]
                    }
                },
                "required": ["target"]
            })
        }

        async fn execute(
            &self,
            _arguments: serde_json::Value,
        ) -> crate::tools::ToolExecutionResult {
            crate::tools::ToolExecutionResult::success(serde_json::json!({"ok": true}))
        }
    }

    #[tokio::test]
    async fn test_subagents_do_not_shadow_existing_spawn_agent_provider() {
        let mut registry = CapabilityRegistry::new();
        registry.register(SubagentCapability);
        registry.register(ExistingSpawnAgentCapability);

        let collected = collect_capabilities(
            &[
                SUBAGENTS_CAPABILITY_ID.to_string(),
                "existing_spawn_agent".to_string(),
            ],
            &registry,
            &test_ctx(),
        )
        .await;

        let spawn_agent_defs: Vec<_> = collected
            .tool_definitions
            .iter()
            .filter(|tool| tool.name() == "spawn_agent")
            .collect();
        assert_eq!(spawn_agent_defs.len(), 1);
        assert_eq!(
            spawn_agent_defs[0].parameters()["properties"]["target"]["properties"]["type"]["enum"],
            serde_json::json!(["external_a2a"])
        );
    }

    #[tokio::test]
    async fn test_agent_handoff_does_not_shadow_existing_spawn_agent_provider() {
        let mut registry = CapabilityRegistry::new();
        registry.register(AgentHandoffCapability);
        registry.register(ExistingSpawnAgentCapability);

        let agent_id = crate::typed_id::AgentId::new();
        let harness_id = crate::typed_id::HarnessId::new();
        let configs = vec![
            AgentCapabilityConfig::with_config(
                CapabilityId::new(AGENT_HANDOFF_CAPABILITY_ID),
                serde_json::json!({
                    "targets": [{
                        "id": "aws_operator",
                        "name": "AWS Operator",
                        "agent_id": agent_id,
                        "harness_id": harness_id
                    }]
                }),
            ),
            AgentCapabilityConfig::with_config(
                CapabilityId::new("existing_spawn_agent"),
                serde_json::json!({}),
            ),
        ];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        let spawn_agent_defs: Vec<_> = collected
            .tool_definitions
            .iter()
            .filter(|tool| tool.name() == "spawn_agent")
            .collect();
        assert_eq!(spawn_agent_defs.len(), 1);
        assert_eq!(
            spawn_agent_defs[0].parameters()["properties"]["target"]["properties"]["type"]["enum"],
            serde_json::json!(["external_a2a"])
        );
    }

    /// Idempotence: explicitly selecting `background_execution` plus a
    /// background-capable tool must not produce duplicate spawn_background
    /// entries.
    #[tokio::test]
    async fn test_background_execution_explicit_selection_is_idempotent() {
        let registry = CapabilityRegistry::with_builtins();
        let collected = collect_capabilities(
            &[
                "bashkit_shell".to_string(),
                BACKGROUND_EXECUTION_CAPABILITY_ID.to_string(),
            ],
            &registry,
            &test_ctx(),
        )
        .await;

        let spawn_background_count = collected
            .tool_definitions
            .iter()
            .filter(|t| t.name() == "spawn_background")
            .count();
        assert_eq!(
            spawn_background_count, 1,
            "spawn_background must appear exactly once even when \
             background_execution is selected explicitly alongside a \
             background-capable tool"
        );
        let applied_count = collected
            .applied_ids
            .iter()
            .filter(|id| id.as_str() == BACKGROUND_EXECUTION_CAPABILITY_ID)
            .count();
        assert_eq!(
            applied_count, 1,
            "background_execution must appear exactly once in applied_ids"
        );
    }

    /// Lockstep: with_defaults() must NOT include spawn_background — it only
    /// reaches the worker registry through the auto-activated capability.
    /// This proves the executor cannot dispatch spawn_background without the
    /// model having seen it.
    #[test]
    fn test_defaults_do_not_include_spawn_background() {
        let registry = crate::ToolRegistry::with_defaults();
        assert!(
            !registry.has("spawn_background"),
            "with_defaults() must not include 'spawn_background' — it comes \
             from the background_execution capability (EVE-501)"
        );
    }

    // =========================================================================
    // Feature tests
    // =========================================================================

    #[test]
    fn test_capability_features_default_empty() {
        let registry = fixture_registry();

        // Most capabilities have no features
        let noop = registry.get("noop").unwrap();
        assert!(noop.features().is_empty());

        let current_time = registry.get("current_time").unwrap();
        assert!(current_time.features().is_empty());
    }

    #[test]
    fn test_file_system_capability_features() {
        let registry = fixture_registry();

        let fs = registry.get("session_file_system").unwrap();
        assert_eq!(fs.features(), vec!["file_system"]);
    }

    #[test]
    fn test_bashkit_shell_capability_features() {
        let registry = fixture_registry();

        let bash = registry.get("bashkit_shell").unwrap();
        assert_eq!(bash.features(), vec!["file_system"]);
    }

    #[test]
    fn test_alias_resolves_to_canonical_capability() {
        let registry = fixture_registry();

        // Legacy `virtual_bash` ID (persisted agent configs) must keep working.
        let via_alias = registry.get("virtual_bash").unwrap();
        assert_eq!(via_alias.id(), "bashkit_shell");
        assert!(registry.has("virtual_bash"));
        assert_eq!(registry.canonical_id("virtual_bash"), Some("bashkit_shell"));
        assert_eq!(
            registry.canonical_id("bashkit_shell"),
            Some("bashkit_shell")
        );
        assert_eq!(registry.canonical_id("nonexistent"), None);
    }

    #[test]
    fn test_alias_dedupes_with_canonical_in_dependency_resolution() {
        let registry = fixture_registry();

        // Selecting both the alias and the canonical ID must resolve to a
        // single activation under the canonical ID.
        let resolved = resolve_dependencies(
            &["virtual_bash".to_string(), "bashkit_shell".to_string()],
            &registry,
        )
        .unwrap();
        let bash_ids: Vec<_> = resolved
            .resolved_ids
            .iter()
            .filter(|id| id.as_str() == "bashkit_shell" || id.as_str() == "virtual_bash")
            .collect();
        assert_eq!(bash_ids, vec!["bashkit_shell"]);
        // Selected via alias => not reported as "added as dependency".
        assert!(
            !resolved
                .added_as_dependencies
                .contains(&"bashkit_shell".to_string())
        );
    }

    #[test]
    fn test_alias_preserves_explicit_config_in_resolution() {
        let registry = fixture_registry();

        let configs = vec![AgentCapabilityConfig::with_config(
            "virtual_bash".to_string(),
            serde_json::json!({"key": "value"}),
        )];
        let resolved = resolve_capability_configs(&configs, &registry).unwrap();
        let bash = resolved
            .iter()
            .find(|c| c.capability_id() == "bashkit_shell")
            .expect("alias must resolve to canonical bashkit_shell config");
        assert_eq!(
            bash.config_value().clone(),
            serde_json::json!({"key": "value"})
        );
    }

    #[test]
    fn test_unregister_by_alias_removes_capability_and_aliases() {
        let mut registry = fixture_registry();

        assert!(registry.unregister("virtual_bash").is_some());
        assert!(!registry.has("bashkit_shell"));
        assert!(!registry.has("virtual_bash"));
    }

    #[test]
    fn test_session_storage_capability_features() {
        let registry = CapabilityRegistry::with_builtins();

        let storage = registry.get("session_storage").unwrap();
        let features = storage.features();
        assert!(features.contains(&"secrets"));
        assert!(features.contains(&"key_value"));
    }

    #[test]
    fn test_session_schedule_capability_features() {
        let registry = CapabilityRegistry::with_builtins();

        let schedule = registry.get("session_schedule").unwrap();
        assert_eq!(schedule.features(), vec!["schedules"]);
    }

    #[test]
    fn test_session_sql_database_capability_features() {
        let registry = CapabilityRegistry::with_builtins();

        let sql = registry.get("session_sql_database").unwrap();
        assert_eq!(sql.features(), vec!["sql_database"]);
    }

    #[test]
    fn test_sample_data_capability_features() {
        let registry = fixture_registry();

        let sample = registry.get("sample_data").unwrap();
        assert_eq!(sample.features(), vec!["file_system"]);
    }

    #[test]
    fn test_compute_features_empty() {
        let registry = CapabilityRegistry::with_builtins();

        let features = compute_features(&[], &registry);
        assert!(features.is_empty());
    }

    #[test]
    fn test_compute_features_single_capability() {
        let registry = CapabilityRegistry::with_builtins();

        let features = compute_features(&["session_schedule".to_string()], &registry);
        assert_eq!(features, vec!["schedules"]);
    }

    #[test]
    fn test_compute_features_multiple_capabilities() {
        let registry = fixture_registry();

        let features = compute_features(
            &[
                "session_file_system".to_string(),
                "session_storage".to_string(),
                "session_schedule".to_string(),
            ],
            &registry,
        );
        assert!(features.contains(&"file_system".to_string()));
        assert!(features.contains(&"secrets".to_string()));
        assert!(features.contains(&"key_value".to_string()));
        assert!(features.contains(&"schedules".to_string()));
    }

    #[test]
    fn test_compute_features_deduplicates() {
        let registry = fixture_registry();

        // Both session_file_system and bashkit_shell contribute "file_system"
        let features = compute_features(
            &[
                "session_file_system".to_string(),
                "bashkit_shell".to_string(),
            ],
            &registry,
        );
        let file_system_count = features.iter().filter(|f| *f == "file_system").count();
        assert_eq!(file_system_count, 1, "file_system should appear only once");
    }

    #[test]
    fn test_compute_features_includes_dependency_features() {
        let registry = fixture_registry();

        // bashkit_shell depends on session_file_system; both contribute "file_system"
        let features = compute_features(&["bashkit_shell".to_string()], &registry);
        assert!(features.contains(&"file_system".to_string()));
    }

    #[test]
    fn test_compute_features_generic_harness_set() {
        let registry = fixture_registry();

        // Typical Generic Harness capabilities
        let features = compute_features(
            &[
                "session_file_system".to_string(),
                "bashkit_shell".to_string(),
                "session_storage".to_string(),
                "session".to_string(),
                "session_schedule".to_string(),
            ],
            &registry,
        );
        assert!(features.contains(&"file_system".to_string()));
        assert!(features.contains(&"secrets".to_string()));
        assert!(features.contains(&"key_value".to_string()));
        assert!(features.contains(&"schedules".to_string()));
    }

    #[test]
    fn test_compute_features_unknown_capability_ignored() {
        let registry = CapabilityRegistry::with_builtins();

        let features = compute_features(
            &["unknown_cap".to_string(), "session_schedule".to_string()],
            &registry,
        );
        assert_eq!(features, vec!["schedules"]);
    }

    #[test]
    fn test_risk_level_ordering() {
        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
    }

    #[test]
    fn test_risk_level_serde_roundtrip() {
        let high = RiskLevel::High;
        let json = serde_json::to_string(&high).unwrap();
        assert_eq!(json, "\"high\"");
        let back: RiskLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, RiskLevel::High);
    }

    #[test]
    fn test_capability_risk_levels() {
        let registry = fixture_registry();

        // bashkit_shell is High (code execution requires admin gating)
        let bash = registry.get("bashkit_shell").unwrap();
        assert_eq!(bash.risk_level(), RiskLevel::High);

        // web_fetch is High (network access requires admin gating)
        let fetch = registry.get("web_fetch").unwrap();
        assert_eq!(fetch.risk_level(), RiskLevel::High);

        // Default capabilities should be Low
        let noop = registry.get("noop").unwrap();
        assert_eq!(noop.risk_level(), RiskLevel::Low);
    }

    // ========================================================================
    // contribute_skills() collection — EVE-311
    // ========================================================================

    struct SkillContributingCapability;

    impl Capability for SkillContributingCapability {
        fn id(&self) -> &str {
            "contributes_skills"
        }
        fn name(&self) -> &str {
            "Contributes Skills"
        }
        fn description(&self) -> &str {
            "Test capability that contributes skills."
        }
        fn contribute_skills(&self) -> Vec<SkillContribution> {
            vec![
                SkillContribution::new("alpha-skill", "Alpha skill desc", "# Alpha\nDo alpha.")
                    .with_files(vec![(
                        "scripts/a.sh".to_string(),
                        "#!/bin/sh\necho a\n".to_string(),
                    )]),
                SkillContribution::new("beta-skill", "Beta skill desc", "# Beta\nDo beta.")
                    .with_user_invocable(false),
            ]
        }
    }

    fn skill_md_from_entries(entries: &HashMap<String, MountEntry>) -> &str {
        match &entries.get("SKILL.md").expect("SKILL.md missing").source {
            MountSource::InlineFile { content, .. } => content.as_str(),
            _ => panic!("Expected InlineFile for SKILL.md"),
        }
    }

    #[tokio::test]
    async fn test_contribute_skills_normalized_to_mounts() {
        let mut registry = CapabilityRegistry::new();
        registry.register(SkillContributingCapability);

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("contributes_skills"),
            serde_json::json!({}),
        )];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        let skill_mounts: Vec<_> = collected
            .mounts
            .iter()
            .filter(|m| m.path.starts_with("/.agents/skills/"))
            .collect();
        assert_eq!(skill_mounts.len(), 2);

        // Every contributed skill mount is read-only and owned by the contributing
        // capability so the VFS layer can attribute skill files correctly.
        for m in &skill_mounts {
            assert!(m.is_readonly());
            assert_eq!(m.capability_id, "contributes_skills");
        }

        let alpha = skill_mounts
            .iter()
            .find(|m| m.path == "/.agents/skills/alpha-skill")
            .expect("alpha-skill mount missing");
        match &alpha.source {
            MountSource::InlineDirectory { entries } => {
                assert!(entries.contains_key("SKILL.md"));
                assert!(entries.contains_key("scripts/a.sh"));
                let parsed = crate::skill::parse_skill_md(skill_md_from_entries(entries)).unwrap();
                assert_eq!(parsed.name, "alpha-skill");
                assert!(parsed.user_invocable);
            }
            _ => panic!("Expected InlineDirectory"),
        }

        let beta = skill_mounts
            .iter()
            .find(|m| m.path == "/.agents/skills/beta-skill")
            .expect("beta-skill mount missing");
        match &beta.source {
            MountSource::InlineDirectory { entries } => {
                let parsed = crate::skill::parse_skill_md(skill_md_from_entries(entries)).unwrap();
                assert!(!parsed.user_invocable);
            }
            _ => panic!("Expected InlineDirectory"),
        }
    }

    #[tokio::test]
    async fn test_contribute_skills_default_empty() {
        // Registry-resident capability without a contribute_skills override
        // must not add skill mounts.
        let mut registry = CapabilityRegistry::new();
        registry.register(FilterTestCapability { priority: 0 });

        let configs = vec![AgentCapabilityConfig::with_config(
            CapabilityId::new("filter_test"),
            serde_json::json!({}),
        )];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;
        assert!(
            collected
                .mounts
                .iter()
                .all(|m| !m.path.starts_with("/.agents/skills/"))
        );
    }

    struct LocalizedCapability;

    impl Capability for LocalizedCapability {
        fn id(&self) -> &str {
            "localized"
        }
        fn name(&self) -> &str {
            "Localized"
        }
        fn description(&self) -> &str {
            "English description"
        }
        fn localizations(&self) -> Vec<CapabilityLocalization> {
            vec![
                CapabilityLocalization {
                    locale: "en",
                    name: None,
                    description: None,
                    config_description: Some("Controls things."),
                    config_overlay: None,
                },
                CapabilityLocalization {
                    locale: "uk",
                    name: Some("Локалізована"),
                    description: Some("Український опис"),
                    config_description: Some("Керує налаштуваннями."),
                    config_overlay: None,
                },
            ]
        }
    }

    #[test]
    fn localized_name_falls_back_exact_language_then_base() {
        let cap = LocalizedCapability;
        // Region tag resolves through the language family.
        assert_eq!(cap.localized_name(Some("uk-UA")), "Локалізована");
        assert_eq!(cap.localized_name(Some("uk")), "Локалізована");
        // Underscore-separated tags are normalized.
        assert_eq!(cap.localized_name(Some("uk_UA")), "Локалізована");
        // Unsupported locales and None fall back to the base name.
        assert_eq!(cap.localized_name(Some("fr-FR")), "Localized");
        assert_eq!(cap.localized_name(None), "Localized");
        assert_eq!(cap.localized_description(Some("uk")), "Український опис");
        assert_eq!(cap.localized_description(Some("de")), "English description");
    }

    #[test]
    fn describe_schema_resolves_config_description_per_locale() {
        let cap = LocalizedCapability;
        assert_eq!(
            cap.describe_schema(Some("uk-UA")).as_deref(),
            Some("Керує налаштуваннями.")
        );
        // Unsupported locales fall back to the "en" entry.
        assert_eq!(
            cap.describe_schema(Some("pl")).as_deref(),
            Some("Controls things.")
        );
        assert_eq!(
            cap.describe_schema(None).as_deref(),
            Some("Controls things.")
        );
        // Capabilities without localizations have no config description.
        assert_eq!(HostAnnotatedCapability.describe_schema(Some("uk")), None);
    }
}
