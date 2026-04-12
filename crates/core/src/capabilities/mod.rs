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
//!   agent's base system prompt. See specs/xml-prompt-formatting.md for rationale.
//!
//! Each capability is in its own file with collocated tools.

use crate::command::CommandDescriptor;
use crate::deployment::DeploymentGrade;
use crate::message_filter::MessageFilterProvider;
use crate::runtime_agent::RuntimeAgent;
use crate::tool_types::ToolDefinition;
use crate::tools::{Tool, ToolRegistry};
use crate::traits::SessionFileStore;
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
    /// If set, only registered when the named internal feature flag is enabled.
    /// Checked via `InternalFeatureFlags::is_enabled()` at registry build time.
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

mod agent_instructions;
pub mod attach_skill;
mod budgeting;
pub mod compaction;
mod current_time;
mod fake_aws;
mod fake_crm;
mod fake_financial;
mod fake_warehouse;
mod file_system;
mod infinity_context;
mod loop_detection;
pub mod mcp;
mod noop;
mod openai_tool_search;
mod openui;
pub mod persistent_memory;
mod platform_docs;
mod platform_management;
mod research;
mod sample_data;
mod session;
mod session_schedule;
mod session_sql_database;
mod session_storage;
mod skills;
mod stateless_todo_list;
mod subagents;
mod system_commands;
mod test_math;
mod test_weather;
mod tool_output_persistence;
mod virtual_bash;
mod web_fetch;

// Re-export capabilities
pub use agent_instructions::{
    AGENT_INSTRUCTIONS_CAPABILITY_ID, AGENTS_MD_PATH, AgentInstructionsCapability,
    MAX_AGENTS_MD_SIZE, format_agents_md_content,
};
pub use attach_skill::{
    AttachSkillCapability, SKILL_CAPABILITY_PREFIX, SKILLS_DISCOVERY_PATH, SkillInstructions,
    SkillMeta, SkillSource, discover_skills_from_entries, is_skill_capability,
    parse_skill_capability_id, skill_capability_id,
};
pub use compaction::{
    COMPACTION_CAPABILITY_ID, CompactionCapability, CompactionConfig, CompactionStep,
    CompactionStrategy, HierarchicalMemoryConfig, MaskingSummaryFormat, MemoryTier,
    ObservationMaskingConfig, ObservationMaskingResult, SessionCompactionMetrics,
    SummarizationConfig, aggressive_trim, apply_hierarchical_memory, apply_observation_masking,
    build_summarization_prompt, build_summary_message, classify_memory_tiers, estimate_tokens,
    estimate_total_tokens, format_messages_for_summarization, should_compact_proactively,
};
pub use current_time::{CurrentTimeCapability, GetCurrentTimeTool};
pub use fake_aws::{
    AwsCreateEc2InstanceTool, AwsCreateIamUserTool, AwsCreateRdsDatabaseTool,
    AwsCreateS3BucketTool, AwsGetCloudWatchMetricsTool, AwsListEc2InstancesTool,
    AwsListIamUsersTool, AwsListRdsDatabasesTool, AwsListS3BucketsTool, AwsListSecurityGroupsTool,
    AwsStopEc2InstanceTool, FakeAwsCapability,
};
pub use fake_crm::{
    CrmAddInteractionTool, CrmCreateCustomerTool, CrmCreateTicketTool, CrmGetCustomerTool,
    CrmListCustomersTool, CrmListTicketsTool, CrmSearchCustomersTool, CrmUpdateTicketTool,
    FakeCrmCapability,
};
pub use fake_financial::{
    FakeFinancialCapability, FinanceCreateBudgetTool, FinanceCreateTransactionTool,
    FinanceForecastCashFlowTool, FinanceGetBalanceTool, FinanceGetExpenseReportTool,
    FinanceGetRevenueReportTool, FinanceListBudgetsTool, FinanceListTransactionsTool,
};
pub use fake_warehouse::{
    FakeWarehouseCapability, WarehouseCreateInvoiceTool, WarehouseCreateOrderTool,
    WarehouseCreateShipmentTool, WarehouseGetInventoryTool, WarehouseInventoryReportTool,
    WarehouseListOrdersTool, WarehouseListShipmentsTool, WarehouseProcessReturnTool,
    WarehouseUpdateInventoryTool, WarehouseUpdateShipmentStatusTool,
};
pub use file_system::{
    DeleteFileTool, EditFileTool, FileSystemCapability, GrepFilesTool, ListDirectoryTool,
    ReadFileTool, StatFileTool, WriteFileTool,
};
pub use infinity_context::{
    INFINITY_CONTEXT_CAPABILITY_ID, InfinityContextCapability, QueryHistoryTool,
};
pub use loop_detection::LoopDetectionCapability;
pub use mcp::{
    MCP_CAPABILITY_PREFIX, McpCapability, is_mcp_capability, mcp_capability_id,
    parse_mcp_capability_id,
};
pub use noop::NoopCapability;
pub use openai_tool_search::{
    DEFAULT_TOOL_SEARCH_THRESHOLD, OPENAI_TOOL_SEARCH_CAPABILITY_ID, OpenAiToolSearchCapability,
};
pub use openui::{OPENUI_CAPABILITY_ID, OpenUiCapability};
pub use persistent_memory::{
    ForgetTool, MEMORY_CAPABILITY_ID, MemoryCapability, MemoryConfig, RecallTool, RememberTool,
};
pub use platform_docs::PlatformDocsCapability;
pub use platform_management::{
    ManageAgentsTool, ManageHarnessesTool, ManageSessionsTool, PlatformManagementCapability,
    ReadAgentsTool, ReadCapabilitiesTool, ReadHarnessesTool, ReadSessionsTool,
    SessionReadMessagesTool, SessionReadResponseTool, SessionSendMessageTool,
};
pub use research::ResearchCapability;
pub use sample_data::SampleDataCapability;
pub use session::{GetSessionInfoTool, SessionCapability, WriteSessionTitleTool};
pub use session_schedule::{
    CancelScheduleTool, CreateScheduleTool, ListSchedulesTool, SESSION_SCHEDULE_CAPABILITY_ID,
    SessionScheduleCapability,
};
pub use session_sql_database::{
    SessionSqlDatabaseCapability, SqlExecuteTool, SqlQueryTool, SqlSchemaTool,
};
pub use session_storage::{KvStoreTool, SecretStoreTool, SessionStorageCapability};
pub use skills::{SKILLS_CAPABILITY_ID, SkillsCapability};
pub use stateless_todo_list::{StatelessTodoListCapability, WriteTodosTool};
pub use subagents::SubagentCapability;
// Blueprint types are exported directly from the trait definitions above
pub use system_commands::{SYSTEM_COMMANDS_CAPABILITY_ID, SystemCommandsCapability};
pub use test_math::{AddTool, DivideTool, MultiplyTool, SubtractTool, TestMathCapability};
pub use test_weather::{GetForecastTool, GetWeatherTool, TestWeatherCapability};
pub use virtual_bash::{BashTool, VirtualBashCapability};
pub use web_fetch::{
    BotAuthPublicKey, WebFetchCapability, WebFetchTool, derive_bot_auth_public_key,
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
    pub file_store: Option<Arc<dyn SessionFileStore>>,
}

impl SystemPromptContext {
    /// Create context with no file store (for callers that don't need filesystem access)
    pub fn without_file_store(session_id: SessionId) -> Self {
        Self {
            session_id,
            locale: None,
            file_store: None,
        }
    }
}

// ============================================================================
// Capability Trait
// ============================================================================

/// Trait for implementing capabilities that extend agent functionality.
///
/// A capability can contribute:
/// - System prompt additions (prepended to agent's system prompt)
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
#[async_trait]
pub trait Capability: Send + Sync {
    /// Returns the unique capability identifier as a string
    fn id(&self) -> &str;

    /// Returns the display name
    fn name(&self) -> &str;

    /// Returns a description of what this capability provides
    fn description(&self) -> &str;

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

    /// Returns static text to prepend to the agent's system prompt (optional).
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

    /// Returns agent blueprints contributed by this capability.
    ///
    /// Blueprints are pre-built agent definitions with private tools, baked-in prompts,
    /// and fixed/default models. They are spawned via `spawn_subagent(blueprint: "<id>")`.
    /// Blueprint tools never appear in the host agent's tool list.
    ///
    /// By default, returns an empty vector (no blueprints).
    fn agent_blueprints(&self) -> Vec<AgentBlueprint> {
        vec![]
    }
}

/// Risk classification for capabilities (TM-AGENT-005).
///
/// Used to enforce approval requirements when assigning capabilities.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
/// `spawn_subagent(blueprint: "<id>")`. Blueprint tools never appear in the
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
/// // Get a capability by ID
/// if let Some(cap) = registry.get("current_time") {
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
}

impl CapabilityRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            capabilities: HashMap::new(),
        }
    }

    /// Create a registry with all built-in capabilities registered
    ///
    /// Uses `DeploymentGrade::from_env()` to determine which capabilities to include.
    /// For explicit control, use `with_builtins_for_grade()`.
    pub fn with_builtins() -> Self {
        Self::with_builtins_for_grade(DeploymentGrade::from_env())
    }

    /// Create a registry with built-in capabilities for a specific deployment grade
    ///
    /// Experimental capabilities are included via integration plugins in dev environments.
    /// Non-experimental integration plugins (like Daytona) are included in all environments.
    pub fn with_builtins_for_grade(grade: DeploymentGrade) -> Self {
        let mut registry = Self::new();

        // Core capabilities (all environments)
        registry.register(AgentInstructionsCapability);
        registry.register(NoopCapability);
        registry.register(CurrentTimeCapability);
        registry.register(ResearchCapability);
        registry.register(PlatformManagementCapability);
        registry.register(FileSystemCapability);
        registry.register(SessionStorageCapability);
        registry.register(SessionCapability);
        registry.register(SessionSqlDatabaseCapability);
        registry.register(TestMathCapability);
        registry.register(TestWeatherCapability);
        registry.register(StatelessTodoListCapability);
        registry.register(WebFetchCapability::from_env());
        registry.register(VirtualBashCapability);
        registry.register(SessionScheduleCapability);
        registry.register(InfinityContextCapability);
        registry.register(budgeting::BudgetingCapability);
        registry.register(CompactionCapability);
        registry.register(MemoryCapability);

        // OpenAI tool_search (deferred tool loading, all environments)
        registry.register(OpenAiToolSearchCapability::new());

        // Skills (filesystem-based discovery + activation, all environments)
        registry.register(SkillsCapability);

        // Subagents (spawn child agent sessions, all environments)
        registry.register(SubagentCapability);

        // System commands (/clear, /status, /compact, /model)
        registry.register(SystemCommandsCapability);

        // Tool output persistence (EVE-222: persist exec output to VFS)
        registry.register(tool_output_persistence::ToolOutputPersistenceCapability);

        // Loop detection (EVE-227: detect repeated identical tool calls)
        registry.register(LoopDetectionCapability);

        // OpenUI generative UI (all environments)
        registry.register(OpenUiCapability);

        // Platform documentation (virtual mount, all environments)
        registry.register(PlatformDocsCapability);

        // Demo capability with mount points (all environments)
        registry.register(SampleDataCapability);

        // Fake demo capabilities (all environments)
        registry.register(FakeWarehouseCapability);
        registry.register(FakeAwsCapability);
        registry.register(FakeCrmCapability);
        registry.register(FakeFinancialCapability);

        // External integration plugins (registered via inventory::submit! in integration crates)
        let internal_flags = crate::InternalFeatureFlags::from_env();
        for plugin in inventory::iter::<IntegrationPlugin>() {
            if (!plugin.experimental_only || grade.experimental_features_enabled())
                && plugin
                    .feature_flag
                    .is_none_or(|f| internal_flags.is_enabled(f))
            {
                registry.register_boxed((plugin.factory)());
            }
        }

        registry
    }

    /// Register a capability
    pub fn register(&mut self, capability: impl Capability + 'static) {
        self.capabilities
            .insert(capability.id().to_string(), Arc::new(capability));
    }

    /// Register a boxed capability
    pub fn register_boxed(&mut self, capability: Box<dyn Capability>) {
        self.capabilities
            .insert(capability.id().to_string(), Arc::from(capability));
    }

    /// Register an Arc-wrapped capability
    pub fn register_arc(&mut self, capability: Arc<dyn Capability>) {
        self.capabilities
            .insert(capability.id().to_string(), capability);
    }

    /// Get a capability by ID
    pub fn get(&self, id: &str) -> Option<&Arc<dyn Capability>> {
        self.capabilities.get(id)
    }

    /// Remove a capability from the registry.
    pub fn unregister(&mut self, id: &str) -> Option<Arc<dyn Capability>> {
        self.capabilities.remove(id)
    }

    /// Check if a capability is registered
    pub fn has(&self, id: &str) -> bool {
        self.capabilities.contains_key(id)
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

    /// Create a new builder with built-in capabilities
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

/// Collected data from capabilities before applying to config.
///
/// This intermediate struct allows sharing the capability collection logic
/// between `apply_capabilities` and `apply_capabilities_to_builder`.
pub struct CollectedCapabilities {
    /// System prompt additions (in order)
    pub system_prompt_parts: Vec<String>,
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
    pub tool_search: Option<crate::llm_driver_registry::ToolSearchConfig>,
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

    for cap_config in capability_configs {
        let cap_id = cap_config.capability_ref.as_str();
        if let Some(capability) = registry.get(cap_id) {
            if capability.status() != CapabilityStatus::Available {
                continue;
            }
            if let Some(provider) = capability.message_filter_provider() {
                message_filter_providers.push((provider, cap_config.config.clone()));
            }
        }
    }

    message_filter_providers.sort_by_key(|(p, _)| p.priority());

    CollectedMessageFilters {
        message_filter_providers,
    }
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

    let user_selected: HashSet<String> = selected_ids.iter().cloned().collect();
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
    let selected_ids: Vec<String> = selected_configs
        .iter()
        .map(|config| config.capability_id().to_string())
        .collect();
    let resolved = resolve_dependencies(&selected_ids, registry)?;

    let explicit_configs: std::collections::HashMap<String, serde_json::Value> = selected_configs
        .iter()
        .map(|config| (config.capability_id().to_string(), config.config.clone()))
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
            // Unknown capability - skip silently (will be caught later)
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
        .map(|id| AgentCapabilityConfig {
            capability_ref: CapabilityId::new(id),
            config: serde_json::Value::Object(serde_json::Map::new()),
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
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut tool_definitions: Vec<ToolDefinition> = Vec::new();
    let mut mounts: Vec<MountPoint> = Vec::new();
    let mut message_filter_providers: Vec<(Arc<dyn MessageFilterProvider>, serde_json::Value)> =
        Vec::new();
    let mut applied_ids: Vec<String> = Vec::new();
    let mut tool_search: Option<crate::llm_driver_registry::ToolSearchConfig> = None;

    for cap_config in capability_configs {
        let cap_id = cap_config.capability_ref.as_str();
        if let Some(capability) = registry.get(cap_id) {
            // Only collect from available capabilities
            if capability.status() != CapabilityStatus::Available {
                continue;
            }

            // Collect dynamic system prompt contribution (config-aware, may read from filesystem)
            if let Some(contribution) = capability
                .system_prompt_contribution_with_config(ctx, &cap_config.config)
                .await
            {
                system_prompt_parts.push(contribution);
            }

            // Collect tools (config-aware: capabilities can adapt based on per-agent config)
            tools.extend(capability.tools_with_config(&cap_config.config));

            // Collect tool definitions, propagating capability category if not already set
            let cap_category = capability.category();
            for def in capability.tool_definitions() {
                let def = match (def.category(), cap_category) {
                    (None, Some(cat)) => def.with_category(cat),
                    _ => def,
                };
                tool_definitions.push(def);
            }

            // Detect OpenAI tool_search capability
            if cap_id == OPENAI_TOOL_SEARCH_CAPABILITY_ID {
                // Parse threshold from config, fall back to default
                let threshold = cap_config
                    .config
                    .get("threshold")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .unwrap_or(DEFAULT_TOOL_SEARCH_THRESHOLD);
                tool_search = Some(crate::llm_driver_registry::ToolSearchConfig {
                    enabled: true,
                    threshold,
                });
            }

            // Collect mount points
            mounts.extend(capability.mounts());

            // Collect message filter provider
            if let Some(provider) = capability.message_filter_provider() {
                message_filter_providers.push((provider, cap_config.config.clone()));
            }

            applied_ids.push(cap_id.to_string());
        }
    }

    // Sort message filter providers by priority (lower = earlier)
    message_filter_providers.sort_by_key(|(p, _)| p.priority());

    CollectedCapabilities {
        system_prompt_parts,
        tools,
        tool_definitions,
        mounts,
        message_filter_providers,
        applied_ids,
        tool_search,
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
/// 2. Prepends them to the agent's base system prompt
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
/// let capability_ids = vec!["current_time".to_string()];
/// let applied = apply_capabilities(base_runtime_agent, &capability_ids, &registry, &ctx).await;
///
/// // The runtime agent now includes CurrentTime tool
/// assert!(!applied.tool_registry.is_empty());
/// ```
pub async fn apply_capabilities(
    base_runtime_agent: RuntimeAgent,
    capability_ids: &[String],
    registry: &CapabilityRegistry,
    ctx: &SystemPromptContext,
) -> AppliedCapabilities {
    let collected = collect_capabilities(capability_ids, registry, ctx).await;

    // Build final system prompt: capability additions + base prompt (wrapped in XML tags)
    let final_system_prompt = match collected.system_prompt_prefix() {
        Some(prefix) => format!(
            "{}\n\n<system-prompt>\n{}\n</system-prompt>",
            prefix, base_runtime_agent.system_prompt
        ),
        None => base_runtime_agent.system_prompt,
    };

    // Build tool registry from collected tools
    let mut tool_registry = ToolRegistry::new();
    for tool in collected.tools {
        tool_registry.register_boxed(tool);
    }

    // Create modified runtime agent
    let runtime_agent = RuntimeAgent {
        system_prompt: final_system_prompt,
        model: base_runtime_agent.model,
        tools: collected.tool_definitions,
        max_iterations: base_runtime_agent.max_iterations,
        temperature: base_runtime_agent.temperature,
        max_tokens: base_runtime_agent.max_tokens,
        tool_search: collected.tool_search,
        network_access: base_runtime_agent.network_access,
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

    /// Test helper: dummy context with no file store
    fn test_ctx() -> SystemPromptContext {
        SystemPromptContext::without_file_store(SessionId::new())
    }

    fn expected_core_builtin_ids() -> BTreeSet<&'static str> {
        [
            "agent_instructions",
            "budgeting",
            "noop",
            "current_time",
            "research",
            "platform_management",
            "session_file_system",
            "session_storage",
            "session",
            "session_sql_database",
            "test_math",
            "test_weather",
            "stateless_todo_list",
            "web_fetch",
            "virtual_bash",
            "session_schedule",
            "infinity_context",
            "compaction",
            "memory",
            "openai_tool_search",
            "skills",
            "subagents",
            "system_commands",
            "openui",
            "sample_data",
            "tool_output_persistence",
            "fake_warehouse",
            "fake_aws",
            "fake_crm",
            "fake_financial",
            "loop_detection",
        ]
        .into_iter()
        .collect()
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
        // Dev mode includes all built-in capabilities
        let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);

        assert_eq!(registry_ids(&registry), expected_core_builtin_ids());
    }

    #[test]
    fn test_capability_registry_with_builtins_prod() {
        // Prod mode excludes experimental capabilities
        let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
        assert_eq!(registry_ids(&registry), expected_core_builtin_ids());
        // Experimental capabilities NOT included in prod
        assert!(!registry.has("docker_container"));
    }

    #[test]
    fn test_capability_registry_get() {
        let registry = CapabilityRegistry::with_builtins();

        let noop = registry.get("noop").unwrap();
        assert_eq!(noop.id(), "noop");
        assert_eq!(noop.name(), "No-Op");
        assert_eq!(noop.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_capability_registry_builder() {
        let registry = CapabilityRegistry::builder()
            .capability(NoopCapability)
            .capability(CurrentTimeCapability)
            .build();

        assert!(registry.has("noop"));
        assert!(registry.has("current_time"));
        assert_eq!(registry.len(), 2);
    }

    #[test]
    fn test_capability_status() {
        let registry = CapabilityRegistry::with_builtins();

        let current_time = registry.get("current_time").unwrap();
        assert_eq!(current_time.status(), CapabilityStatus::Available);

        let research = registry.get("research").unwrap();
        assert_eq!(research.status(), CapabilityStatus::ComingSoon);
    }

    #[test]
    fn test_capability_icons_and_categories() {
        let registry = CapabilityRegistry::with_builtins();

        let noop = registry.get("noop").unwrap();
        assert_eq!(noop.icon(), Some("circle-off"));
        assert_eq!(noop.category(), Some("Testing"));

        let current_time = registry.get("current_time").unwrap();
        assert_eq!(current_time.icon(), Some("clock"));
        assert_eq!(current_time.category(), Some("Utilities"));
    }

    #[test]
    fn test_system_prompt_preview_default_delegates_to_addition() {
        let registry = CapabilityRegistry::with_builtins();

        // test_math has a static system_prompt_addition — preview should match
        let test_math = registry.get("test_math").unwrap();
        assert_eq!(
            test_math.system_prompt_preview().as_deref(),
            test_math.system_prompt_addition()
        );

        // current_time has no system_prompt_addition — preview should be None
        let current_time = registry.get("current_time").unwrap();
        assert!(current_time.system_prompt_preview().is_none());
        assert!(current_time.system_prompt_addition().is_none());
    }

    #[test]
    fn test_system_prompt_preview_dynamic_capability() {
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["current_time".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // CurrentTime has no system prompt addition but has a tool
        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
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
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
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

    #[tokio::test]
    async fn test_apply_capabilities_web_fetch() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.2");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["web_fetch".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // WebFetch has system prompt from fetchkit's TOOL_LLMTXT and 1 tool
        assert!(
            applied
                .runtime_agent
                .system_prompt
                .contains(&base_runtime_agent.system_prompt)
        );
        assert!(applied.runtime_agent.system_prompt.contains("web_fetch"));
        assert!(applied.tool_registry.has("web_fetch"));
        assert_eq!(applied.tool_registry.len(), 1);
    }

    // =========================================================================
    // XML prompt formatting tests
    // =========================================================================

    #[tokio::test]
    async fn test_xml_tags_wrap_capability_prompts() {
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
        let base = RuntimeAgent::new("You are helpful.", "gpt-5.2");

        let applied = apply_capabilities(
            base,
            &["stateless_todo_list".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        let prompt = &applied.runtime_agent.system_prompt;
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
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();

        let collected =
            collect_capabilities(&["sample_data".to_string()], &registry, &test_ctx()).await;

        assert!(!collected.mounts.is_empty());
        assert_eq!(collected.mounts.len(), 1);
        assert_eq!(collected.mounts[0].path, "/samples");
        assert!(collected.mounts[0].is_readonly());
    }

    #[tokio::test]
    async fn test_collect_capabilities_empty_mounts_by_default() {
        let registry = CapabilityRegistry::with_builtins();

        // Most capabilities don't have mounts
        let collected =
            collect_capabilities(&["current_time".to_string()], &registry, &test_ctx()).await;

        assert!(collected.mounts.is_empty());
    }

    #[tokio::test]
    async fn test_collect_capabilities_combines_mounts() {
        let registry = CapabilityRegistry::with_builtins();

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
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();

        // CurrentTime has no dependencies
        let resolved = resolve_dependencies(&["current_time".to_string()], &registry).unwrap();

        assert_eq!(resolved.resolved_ids, vec!["current_time"]);
        assert!(resolved.added_as_dependencies.is_empty());
    }

    #[test]
    fn test_resolve_dependencies_with_deps() {
        let registry = CapabilityRegistry::with_builtins();

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
        let registry = CapabilityRegistry::with_builtins();

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
        let registry = CapabilityRegistry::with_builtins();

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
        let registry = CapabilityRegistry::with_builtins();

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
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get("sample_data").unwrap();

        let deps = cap.dependencies();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], "session_file_system");
    }

    #[test]
    fn test_noop_has_no_dependencies() {
        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
        let configs = vec![AgentCapabilityConfig {
            capability_ref: CapabilityId::new("current_time"),
            config: serde_json::json!({}),
        }];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        assert!(collected.message_filter_providers.is_empty());
        assert!(!collected.has_message_filters());
    }

    #[tokio::test]
    async fn test_collect_capabilities_with_configs_with_filter_provider() {
        let mut registry = CapabilityRegistry::new();
        registry.register(FilterTestCapability { priority: 0 });

        let configs = vec![AgentCapabilityConfig {
            capability_ref: CapabilityId::new("filter_test"),
            config: serde_json::json!({ "search": "hello" }),
        }];

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
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("high_priority"),
                config: serde_json::json!({}),
            },
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("low_priority"),
                config: serde_json::json!({}),
            },
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

        let configs = vec![AgentCapabilityConfig {
            capability_ref: CapabilityId::new("filter_test"),
            config: serde_json::json!({ "search": "test_query" }),
        }];

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
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("cap_a"),
                config: serde_json::json!({}),
            },
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("cap_b"),
                config: serde_json::json!({}),
            },
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("cap_c"),
                config: serde_json::json!({}),
            },
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
        let registry = CapabilityRegistry::with_builtins();

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

        let configs = vec![AgentCapabilityConfig {
            capability_ref: CapabilityId::new("filter_test"),
            config: test_config.clone(),
        }];

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

        let configs = vec![AgentCapabilityConfig {
            capability_ref: CapabilityId::new("filter_test"),
            config: serde_json::json!({ "search": "test_query" }),
        }];

        let collected = collect_message_filters_only(&configs, &registry);

        let session_id: SessionId = Uuid::now_v7().into();
        let mut query = MessageQuery::new(session_id);
        collected.apply_message_filters(&mut query);

        assert_eq!(query.filters.len(), 1);
        assert!(matches!(&query.filters[0], MessageFilter::Search(s) if s == "test_query"));
    }

    #[test]
    fn test_collect_message_filters_only_skips_unknown_capabilities() {
        let registry = CapabilityRegistry::new();

        let configs = vec![AgentCapabilityConfig {
            capability_ref: CapabilityId::new("nonexistent"),
            config: serde_json::json!({}),
        }];

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
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("gamma"),
                config: serde_json::json!({}),
            },
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("alpha"),
                config: serde_json::json!({}),
            },
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("beta"),
                config: serde_json::json!({}),
            },
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

        let configs = vec![AgentCapabilityConfig {
            capability_ref: CapabilityId::new("post_load_test"),
            config: serde_json::json!({}),
        }];

        let collected = collect_message_filters_only(&configs, &registry);

        let mut messages = vec![Message::user("first"), Message::user("second")];
        collected.apply_post_load_filters(&mut messages);

        // post_load reversed the messages
        assert_eq!(messages[0].text(), Some("second"));
        assert_eq!(messages[1].text(), Some("first"));
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
    async fn test_virtual_bash_capability_produces_bash_tool() {
        let registry = CapabilityRegistry::with_builtins();
        let collected =
            collect_capabilities(&["virtual_bash".to_string()], &registry, &test_ctx()).await;

        let tool_names: Vec<&str> = collected
            .tool_definitions
            .iter()
            .map(|t| t.name())
            .collect();
        assert!(
            tool_names.contains(&"bash"),
            "virtual_bash capability must produce 'bash' tool, got: {:?}",
            tool_names
        );
        assert!(
            !collected.tools.is_empty(),
            "virtual_bash must provide tool implementations"
        );
    }

    #[tokio::test]
    async fn test_generic_harness_capability_set_produces_bash_tool() {
        // These are the exact capability IDs from the Generic Harness seed data.
        // If any are renamed or removed, this test catches the regression.
        let generic_harness_caps = vec![
            "session_file_system".to_string(),
            "virtual_bash".to_string(),
            "web_fetch".to_string(),
            "session_storage".to_string(),
            "session".to_string(),
            "agent_instructions".to_string(),
            "skills".to_string(),
            "infinity_context".to_string(),
            "openai_tool_search".to_string(),
        ];

        let registry = CapabilityRegistry::with_builtins();
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
        let registry = CapabilityRegistry::with_builtins();
        let collected =
            collect_capabilities(&["virtual_bash".to_string()], &registry, &test_ctx()).await;

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
        let registry = CapabilityRegistry::with_builtins();
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
            "with_defaults() must not include 'bash' — it comes from virtual_bash capability"
        );
    }

    // =========================================================================
    // Feature tests
    // =========================================================================

    #[test]
    fn test_capability_features_default_empty() {
        let registry = CapabilityRegistry::with_builtins();

        // Most capabilities have no features
        let noop = registry.get("noop").unwrap();
        assert!(noop.features().is_empty());

        let current_time = registry.get("current_time").unwrap();
        assert!(current_time.features().is_empty());
    }

    #[test]
    fn test_file_system_capability_features() {
        let registry = CapabilityRegistry::with_builtins();

        let fs = registry.get("session_file_system").unwrap();
        assert_eq!(fs.features(), vec!["file_system"]);
    }

    #[test]
    fn test_virtual_bash_capability_features() {
        let registry = CapabilityRegistry::with_builtins();

        let bash = registry.get("virtual_bash").unwrap();
        assert_eq!(bash.features(), vec!["file_system"]);
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
        let registry = CapabilityRegistry::with_builtins();

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
        let registry = CapabilityRegistry::with_builtins();

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
        let registry = CapabilityRegistry::with_builtins();

        // Both session_file_system and virtual_bash contribute "file_system"
        let features = compute_features(
            &[
                "session_file_system".to_string(),
                "virtual_bash".to_string(),
            ],
            &registry,
        );
        let file_system_count = features.iter().filter(|f| *f == "file_system").count();
        assert_eq!(file_system_count, 1, "file_system should appear only once");
    }

    #[test]
    fn test_compute_features_includes_dependency_features() {
        let registry = CapabilityRegistry::with_builtins();

        // virtual_bash depends on session_file_system; both contribute "file_system"
        let features = compute_features(&["virtual_bash".to_string()], &registry);
        assert!(features.contains(&"file_system".to_string()));
    }

    #[test]
    fn test_compute_features_generic_harness_set() {
        let registry = CapabilityRegistry::with_builtins();

        // Typical Generic Harness capabilities
        let features = compute_features(
            &[
                "session_file_system".to_string(),
                "virtual_bash".to_string(),
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
        let registry = CapabilityRegistry::with_builtins();

        // virtual_bash is Low (in-memory sandboxed execution)
        let bash = registry.get("virtual_bash").unwrap();
        assert_eq!(bash.risk_level(), RiskLevel::Low);

        // web_fetch is Medium (network access)
        let fetch = registry.get("web_fetch").unwrap();
        assert_eq!(fetch.risk_level(), RiskLevel::Medium);

        // Default capabilities should be Low
        let noop = registry.get("noop").unwrap();
        assert_eq!(noop.risk_level(), RiskLevel::Low);
    }

    // =========================================================================
    // OpenAI tool_search capability collection tests
    // =========================================================================

    #[tokio::test]
    async fn test_apply_capabilities_openai_tool_search() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.4");

        let applied = apply_capabilities(
            base_runtime_agent.clone(),
            &["openai_tool_search".to_string()],
            &registry,
            &test_ctx(),
        )
        .await;

        // OpenAiToolSearchCapability provides no tools and no system prompt
        assert_eq!(
            applied.runtime_agent.system_prompt,
            base_runtime_agent.system_prompt
        );
        assert!(applied.tool_registry.is_empty());
        assert_eq!(applied.applied_ids, vec!["openai_tool_search"]);

        // tool_search config should be set on the runtime agent
        let ts = applied.runtime_agent.tool_search.as_ref().unwrap();
        assert!(ts.enabled);
        assert_eq!(ts.threshold, DEFAULT_TOOL_SEARCH_THRESHOLD);
    }

    #[tokio::test]
    async fn test_apply_capabilities_openai_tool_search_with_other_capabilities() {
        let registry = CapabilityRegistry::with_builtins();
        let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.4");

        let applied = apply_capabilities(
            base_runtime_agent,
            &[
                "current_time".to_string(),
                "openai_tool_search".to_string(),
                "test_math".to_string(),
            ],
            &registry,
            &test_ctx(),
        )
        .await;

        // Should have tools from current_time and test_math
        assert!(applied.tool_registry.has("get_current_time"));
        assert!(applied.tool_registry.has("add"));
        assert!(applied.tool_registry.has("subtract"));
        assert!(applied.tool_registry.has("multiply"));
        assert!(applied.tool_registry.has("divide"));

        // tool_search should still be configured
        let ts = applied.runtime_agent.tool_search.as_ref().unwrap();
        assert!(ts.enabled);
        assert_eq!(ts.threshold, DEFAULT_TOOL_SEARCH_THRESHOLD);
    }

    #[tokio::test]
    async fn test_collect_capabilities_tool_search_custom_threshold() {
        let registry = CapabilityRegistry::with_builtins();

        let configs = vec![AgentCapabilityConfig {
            capability_ref: CapabilityId::new("openai_tool_search"),
            config: serde_json::json!({"threshold": 5}),
        }];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        let ts = collected.tool_search.as_ref().unwrap();
        assert!(ts.enabled);
        assert_eq!(ts.threshold, 5);
    }

    #[tokio::test]
    async fn test_collect_capabilities_no_tool_search_without_capability() {
        let registry = CapabilityRegistry::with_builtins();

        let configs = vec![AgentCapabilityConfig {
            capability_ref: CapabilityId::new("current_time"),
            config: serde_json::json!({}),
        }];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        assert!(collected.tool_search.is_none());
    }

    #[tokio::test]
    async fn test_collect_capabilities_tool_search_category_propagation() {
        let registry = CapabilityRegistry::with_builtins();

        // test_math capability has category "Testing"
        let configs = vec![
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("test_math"),
                config: serde_json::json!({}),
            },
            AgentCapabilityConfig {
                capability_ref: CapabilityId::new("openai_tool_search"),
                config: serde_json::json!({}),
            },
        ];

        let collected = collect_capabilities_with_configs(&configs, &registry, &test_ctx()).await;

        // Verify tool_search is configured
        assert!(collected.tool_search.is_some());

        // Verify tools have categories from their capability
        for tool_def in &collected.tool_definitions {
            // test_math tools should have the Math category
            if ["add", "subtract", "multiply", "divide"].contains(&tool_def.name()) {
                assert!(
                    tool_def.category().is_some(),
                    "Tool {} should have a category from its capability",
                    tool_def.name()
                );
            }
        }
    }
}
