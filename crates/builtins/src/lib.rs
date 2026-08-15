//! Portable built-in capabilities for the [Everruns](https://everruns.com) ecosystem.
//!
//! `everruns-builtins` is the optional, backend-neutral implementation bundle
//! for portable capabilities that compose through `everruns-core` execution
//! contracts. This includes the standard skills, context, and human-intent
//! implementations as well as policy hooks. It owns no server, database,
//! network transport, process runner, interpreter, or hosted service
//! implementation. OpenUI and A2UI prompt capabilities are available behind
//! the explicit `ui-capabilities` feature.
//!
//! # Example
//!
//! ```
//! use everruns_builtins::{CurrentTimeCapability, register_portable_capabilities};
//! use everruns_core::{Capability, CapabilityRegistry};
//!
//! let mut registry = CapabilityRegistry::new();
//! register_portable_capabilities(&mut registry)?;
//! assert_eq!(CurrentTimeCapability.id(), "current_time");
//! assert!(registry.get("current_time").is_some());
//! # Ok::<(), everruns_capability::CapabilityError>(())
//! ```

use std::sync::Arc;

use everruns_capability::CapabilityError;

// Public modules support focused imports; the root re-exports below are the
// curated common surface.
#[cfg(feature = "ui-capabilities")]
pub mod a2ui;
pub mod agent_instructions;
pub mod attach_skill;
pub mod auto_tool_search;
pub mod btw;
pub mod budgeting;
pub mod claude_tool_search;
pub mod compaction;
pub mod current_time;
pub mod error_disclosure;
mod framework_config;
pub mod guardrails;
pub mod human_intent;
pub mod infinity_context;
pub mod loop_detection;
pub mod message_metadata;
pub mod openai_tool_search;
#[cfg(feature = "ui-capabilities")]
pub mod openui;
pub mod parallel_tool_calls;
pub mod progress_guard;
pub mod prompt_caching;
pub mod prompt_canary_guardrail;
pub mod self_budget;
pub mod skills;
pub mod skills_scoped;
pub mod stateless_todo_list;
pub mod system_commands;
pub mod tool_approval;
pub mod tool_call_repair;
pub mod tool_output_distillation;
pub mod tool_output_persistence;
pub mod tool_search;
pub mod usage_limit_auto_continue;

// Compatibility paths used by the collocated implementation tests. These are
// aliases of core's provider-neutral execution modules, not copied contracts.
pub(crate) use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityRegistry, CapabilityStatus, Fact, FactsContext,
    ModelViewContext, ModelViewProvider, RiskLevel, SystemPromptContext, ToolDefinitionHook,
    Volatility,
};
pub(crate) use everruns_core::tool_hooks;
#[allow(unused_imports)]
// Collocated unit tests use a wider compatibility subset than the library.
pub(crate) use everruns_core::{
    DEFAULT_ORG_PUBLIC_ID, McpToolInvoker, UtilityLlmService, retain_complete_llm_tool_exchanges,
};
#[allow(unused_imports)]
// Collocated unit tests use a wider compatibility subset than the library.
pub(crate) use everruns_core::{
    budget, capability_dto, capability_types, command, command_host, events, guardrail_checks,
    llm_conversions, llm_error_hook, mcp_server, message, message_filter, output_guardrail,
    runtime_agent, session, session_file, session_files, session_resource, session_schedule, skill,
    tool_context, tool_fingerprint, tool_narration, tool_output_sanitizer, tools, utility_llm,
};
#[allow(unused_imports)]
pub(crate) use everruns_provider::driver_registry::{
    LlmCompletionMetadata, LlmMessage, LlmMessageRole, LlmResponse, LlmResponseStream,
};
#[allow(unused_imports)]
pub(crate) use everruns_provider::error::{AgentLoopError, Result};
#[allow(unused_imports)]
pub(crate) use everruns_provider::typed_id::{PrincipalId, WorkspaceId};
#[allow(unused_imports)]
pub(crate) use everruns_provider::user_facing_error::codes as user_facing_error_codes;
#[allow(unused_imports)]
pub(crate) use everruns_provider::{
    driver_registry, error, model_profiles, provider, tool_types, typed_id, user_facing_error,
};

#[cfg(test)]
mod test_fixtures;

#[cfg(feature = "ui-capabilities")]
pub use a2ui::{A2UI_CAPABILITY_ID, A2UiCapability};
pub use agent_instructions::{
    AGENT_INSTRUCTIONS_CAPABILITY_ID, AGENTS_MD_PATH, AgentInstructionsCapability,
    AgentInstructionsConfig, DEFAULT_AGENT_INSTRUCTIONS_FILE, MAX_AGENT_INSTRUCTIONS_FILES,
    MAX_AGENTS_MD_SIZE, format_agents_md_content, format_instruction_file_content,
};
pub use attach_skill::AttachSkillCapability;
pub use auto_tool_search::{AUTO_TOOL_SEARCH_CAPABILITY_ID, AutoToolSearchCapability};
pub use btw::{BTW_CAPABILITY_ID, BtwCapability};
pub use budgeting::{BUDGETING_CAPABILITY_ID, BudgetingCapability};
pub use claude_tool_search::{CLAUDE_TOOL_SEARCH_CAPABILITY_ID, ClaudeToolSearchCapability};
pub use compaction::{
    COMPACTION_CAPABILITY_ID, CompactionCapability, CompactionConfig as RuntimeCompactionConfig,
    CompactionStep, CompactionStrategy as RuntimeCompactionStrategy, CostControlConfig,
    CostControlMaskingResult, HierarchicalMemoryConfig, MaskingSummaryFormat, MemoryTier,
    ObservationMaskingConfig, ObservationMaskingResult, SessionCompactionMetrics,
    SummarizationConfig, aggressive_trim, apply_cost_control_masking, apply_hierarchical_memory,
    apply_observation_masking, build_model_view_messages, build_summarization_prompt,
    build_summary_message, classify_memory_tiers, compose_summary_with_recent, estimate_tokens,
    estimate_total_tokens, format_messages_for_summarization, should_compact_for_cost,
    should_compact_proactively, total_tool_result_bytes,
};
pub use current_time::{CURRENT_TIME_CAPABILITY_ID, CurrentTimeCapability, GetCurrentTimeTool};
pub use error_disclosure::{
    ERROR_DISCLOSURE_CAPABILITY_ID, ErrorDisclosureCapability, resolve_error_disclosure,
};
pub use framework_config::{CompactionConfig, CompactionStrategy, ToolSearch};
pub use guardrails::{GUARDRAILS_CAPABILITY_ID, GuardrailsCapability};
pub use human_intent::{HUMAN_INTENT_CAPABILITY_ID, HumanIntentCapability};
pub use infinity_context::{
    INFINITY_CONTEXT_CAPABILITY_ID, InfinityContextCapability, InfinityContextFilterOnlyCapability,
    QueryHistoryTool,
};
pub use loop_detection::{LOOP_DETECTION_CAPABILITY_ID, LoopDetectionCapability};
pub use message_metadata::{
    MESSAGE_METADATA_CAPABILITY_ID, MessageMetadataCapability, MessageMetadataConfig,
    MessageMetadataField, render_annotation, strip_leading_timestamp_annotations,
};
pub use openai_tool_search::{
    DEFAULT_TOOL_SEARCH_THRESHOLD, OPENAI_TOOL_SEARCH_CAPABILITY_ID, OpenAiToolSearchCapability,
    model_supports_native_tool_search,
};
#[cfg(feature = "ui-capabilities")]
pub use openui::{OPENUI_CAPABILITY_ID, OpenUiCapability};
pub use parallel_tool_calls::{
    PARALLEL_TOOL_CALLS_CAPABILITY_ID, ParallelToolCallsCapability, ParallelToolCallsMode,
    parallel_tool_calls_from_config,
};
pub use progress_guard::{PROGRESS_GUARD_CAPABILITY_ID, ProgressGuardCapability};
pub use prompt_caching::{PROMPT_CACHING_CAPABILITY_ID, PromptCachingCapability};
pub use prompt_canary_guardrail::{
    DEFAULT_REPLACEMENT as PROMPT_CANARY_DEFAULT_REPLACEMENT,
    PROMPT_CANARY_GUARDRAIL_CAPABILITY_ID, PromptCanaryGuardrailCapability,
    REASON_CODE_SYSTEM_PROMPT_LEAK,
};
pub use self_budget::{SELF_BUDGET_CAPABILITY_ID, SelfBudgetCapability};
pub use skills::{SKILLS_CAPABILITY_ID, Skills, SkillsCapability};
pub use skills_scoped::{
    ScopedSkillsCapability, SkillDirResolver, SkillScope, SkillsConfig, VfsSkillDirResolver,
};
pub use stateless_todo_list::{
    STATELESS_TODO_LIST_CAPABILITY_ID, StatelessTodoList, StatelessTodoListCapability,
    WriteTodosTool,
};
pub use system_commands::{SYSTEM_COMMANDS_CAPABILITY_ID, SystemCommandsCapability};
pub use tool_approval::{
    ApprovalDecision, ApprovalMode, TOOL_APPROVAL_CAPABILITY_ID, ToolApprovalCapability,
    ToolApprover,
};
pub use tool_call_repair::{
    DEFAULT_MAX_REPROMPTS, MAX_SALVAGE_INPUT_BYTES, RepairOutcome, SalvageResult,
    TOOL_CALL_REPAIR_CAPABILITY_ID, ToolCallRepairCapability, ToolCallRepairConfig,
    salvage_tool_arguments, tool_call_repair_capability,
};
pub use tool_output_distillation::{
    DistillOutputHook, TOOL_OUTPUT_DISTILLATION_CAPABILITY_ID, ToolOutputDistillationCapability,
};
pub use tool_output_persistence::{
    PersistOutputHook, TOOL_OUTPUT_PERSISTENCE_CAPABILITY_ID, ToolOutputPersistenceCapability,
};
pub use tool_search::{
    TOOL_SEARCH_CAPABILITY_ID, TOOL_SEARCH_TOOL_NAME, ToolSearchCapability, ToolSearchTool,
};
pub use usage_limit_auto_continue::{
    AutoContinueConfig, USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID, UsageLimitAutoContinueCapability,
    resolve_usage_limit_auto_continue,
};

/// Internal compatibility namespace for the collocated implementations.
#[allow(unused_imports)] // Individual implementation and test builds use different subsets.
pub(crate) mod capabilities {
    pub(crate) use crate::attach_skill;
    pub(crate) use crate::{
        AUTO_TOOL_SEARCH_CAPABILITY_ID, AutoToolSearchCapability, CLAUDE_TOOL_SEARCH_CAPABILITY_ID,
        CURRENT_TIME_CAPABILITY_ID, CurrentTimeCapability, DEFAULT_TOOL_SEARCH_THRESHOLD,
        FactsContext, OPENAI_TOOL_SEARCH_CAPABILITY_ID, OpenAiToolSearchCapability,
        STATELESS_TODO_LIST_CAPABILITY_ID, StatelessTodoListCapability, TOOL_SEARCH_CAPABILITY_ID,
        TOOL_SEARCH_TOOL_NAME, ToolSearchCapability, ToolSearchTool, Volatility,
    };
    pub(crate) use everruns_core::capabilities::*;
}

/// Register the runtime-safe portable capabilities in stable order.
///
/// This excludes [`UsageLimitAutoContinueCapability`], whose error hook needs
/// a schedule store and poller that the default embedded host does not supply.
pub fn register_runtime_capabilities(
    registry: &mut CapabilityRegistry,
) -> std::result::Result<(), CapabilityError> {
    register_capabilities_atomically(registry, runtime_capabilities())
}

/// Register every portable capability in stable product order.
///
/// Registration is explicit: linking this crate has no inventory side effect.
/// Existing IDs and alias collisions are rejected rather than silently
/// replacing an application-provided implementation. A rejected bundle leaves
/// the caller's registry unchanged.
pub fn register_portable_capabilities(
    registry: &mut CapabilityRegistry,
) -> std::result::Result<(), CapabilityError> {
    register_capabilities_atomically(registry, portable_capabilities())
}

/// Build a registry containing only portable capabilities.
pub fn portable_capability_registry() -> std::result::Result<CapabilityRegistry, CapabilityError> {
    let mut registry = CapabilityRegistry::new();
    register_portable_capabilities(&mut registry)?;
    Ok(registry)
}

/// Add the portable context-free tools that belong in a host's default
/// executor registry.
///
/// Capability collection contributes the same tools when their owning
/// capability is configured. Hosts also install these two legacy defaults so
/// executor behavior remains compatible for blueprints and seed-time tooling.
pub fn register_default_tools(registry: &mut everruns_core::ToolRegistry) {
    registry.register(GetCurrentTimeTool);
    registry.register(WriteTodosTool);
}

/// Add portable context-free tools safe for scheduled monitor probes.
pub fn register_monitor_tools(registry: &mut everruns_core::ToolRegistry) {
    registry.register(GetCurrentTimeTool);
}

fn portable_capabilities() -> Vec<Arc<dyn Capability>> {
    let mut capabilities = runtime_capabilities();
    capabilities.push(Arc::new(UsageLimitAutoContinueCapability));
    #[cfg(feature = "ui-capabilities")]
    {
        capabilities.push(Arc::new(OpenUiCapability));
        capabilities.push(Arc::new(A2UiCapability));
    }
    capabilities
}

fn register_capabilities_atomically(
    registry: &mut CapabilityRegistry,
    capabilities: Vec<Arc<dyn Capability>>,
) -> std::result::Result<(), CapabilityError> {
    let mut candidate = registry.clone();
    for capability in capabilities {
        candidate.try_register_arc(capability)?;
    }
    *registry = candidate;
    Ok(())
}

fn runtime_capabilities() -> Vec<Arc<dyn Capability>> {
    vec![
        Arc::new(HumanIntentCapability),
        Arc::new(InfinityContextCapability),
        Arc::new(SkillsCapability),
        Arc::new(AgentInstructionsCapability),
        Arc::new(CurrentTimeCapability),
        Arc::new(MessageMetadataCapability),
        Arc::new(StatelessTodoListCapability),
        Arc::new(BtwCapability),
        Arc::new(BudgetingCapability),
        Arc::new(SelfBudgetCapability),
        Arc::new(CompactionCapability),
        Arc::new(ErrorDisclosureCapability),
        Arc::new(OpenAiToolSearchCapability::new()),
        Arc::new(ClaudeToolSearchCapability::new()),
        Arc::new(ToolSearchCapability::new()),
        Arc::new(AutoToolSearchCapability::new()),
        Arc::new(PromptCachingCapability::new()),
        Arc::new(ParallelToolCallsCapability),
        Arc::new(SystemCommandsCapability),
        Arc::new(ToolOutputPersistenceCapability),
        Arc::new(ToolOutputDistillationCapability),
        Arc::new(LoopDetectionCapability),
        Arc::new(ProgressGuardCapability::new()),
        Arc::new(ToolCallRepairCapability),
        Arc::new(PromptCanaryGuardrailCapability),
        Arc::new(GuardrailsCapability),
    ]
}

#[cfg(test)]
mod bundle_tests {
    use super::*;

    const RUNTIME_IDS: [&str; 26] = [
        "human_intent",
        "infinity_context",
        "skills",
        "agent_instructions",
        "current_time",
        "message_metadata",
        "stateless_todo_list",
        "btw",
        "budgeting",
        "self_budget",
        "compaction",
        "error_disclosure",
        "openai_tool_search",
        "claude_tool_search",
        "tool_search",
        "auto_tool_search",
        "prompt_caching",
        "parallel_tool_calls",
        "system_commands",
        "tool_output_persistence",
        "tool_output_distillation",
        "loop_detection",
        "progress_guard",
        "tool_call_repair",
        "prompt_canary_guardrail",
        "guardrails",
    ];

    fn portable_ids() -> Vec<&'static str> {
        let mut ids = RUNTIME_IDS.to_vec();
        ids.push("usage_limit_auto_continue");
        #[cfg(feature = "ui-capabilities")]
        {
            ids.push("openui");
            ids.push("a2ui");
        }
        ids
    }

    #[test]
    fn bundle_has_the_curated_stable_catalog() {
        let capabilities = portable_capabilities();
        let ids: Vec<_> = capabilities
            .iter()
            .map(|capability| capability.id())
            .collect();
        assert_eq!(ids, portable_ids());
    }

    #[test]
    fn runtime_bundle_excludes_schedule_backed_auto_continue() {
        let mut registry = CapabilityRegistry::new();
        register_runtime_capabilities(&mut registry).unwrap();

        assert_eq!(registry.len(), RUNTIME_IDS.len());
        assert!(!registry.has(USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID));
    }

    #[test]
    fn duplicate_registration_is_rejected_without_replacement() {
        let mut registry = portable_capability_registry().unwrap();
        let original = Arc::clone(registry.get(HUMAN_INTENT_CAPABILITY_ID).unwrap());

        let error = register_portable_capabilities(&mut registry).unwrap_err();

        assert!(error.is_duplicate());
        assert_eq!(error.id(), HUMAN_INTENT_CAPABILITY_ID);
        assert!(Arc::ptr_eq(
            &original,
            registry.get(HUMAN_INTENT_CAPABILITY_ID).unwrap()
        ));
    }

    #[test]
    fn late_collision_does_not_partially_register_the_bundle() {
        let mut registry = CapabilityRegistry::new();
        registry.register(GuardrailsCapability);

        let error = register_portable_capabilities(&mut registry).unwrap_err();

        assert!(error.is_duplicate());
        assert_eq!(error.id(), GUARDRAILS_CAPABILITY_ID);
        assert_eq!(registry.len(), 1);
        assert!(registry.has(GUARDRAILS_CAPABILITY_ID));
        assert!(!registry.has(CURRENT_TIME_CAPABILITY_ID));
    }

    #[test]
    fn dependencies_are_portable_or_explicit_host_seams() {
        let registry = portable_capability_registry().unwrap();
        let host_dependencies = ["session_file_system"];
        let mut external_dependencies = std::collections::BTreeSet::new();

        for capability in registry.list() {
            for dependency in capability.dependencies() {
                if !registry.has(dependency) {
                    external_dependencies.insert(dependency);
                }
                assert!(
                    registry.has(dependency) || host_dependencies.contains(&dependency),
                    "{} has an undeclared external dependency on {dependency}",
                    capability.id()
                );
            }
        }

        assert_eq!(
            external_dependencies,
            host_dependencies.into_iter().collect(),
            "host-provided capability dependencies are an explicit structural boundary"
        );
    }
}
