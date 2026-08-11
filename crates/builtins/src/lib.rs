//! Portable built-in capabilities for the [Everruns](https://everruns.com) ecosystem.
//!
//! `everruns-builtins` is the optional, backend-neutral implementation bundle
//! for policy capabilities that compose through `everruns-core` execution
//! contracts. It owns no server, database, network transport, process runner,
//! interpreter, or hosted service implementation.
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

// Preserve the source modules while the bundle is extracted from core. Public
// modules make focused imports possible; the root re-exports below are the
// curated common surface.
pub mod agent_instructions;
pub mod auto_tool_search;
pub mod btw;
pub mod budgeting;
pub mod claude_tool_search;
pub mod compaction;
pub mod current_time;
pub mod error_disclosure;
mod framework_config;
pub mod guardrails;
pub mod loop_detection;
pub mod message_metadata;
pub mod openai_tool_search;
pub mod parallel_tool_calls;
pub mod progress_guard;
pub mod prompt_caching;
pub mod prompt_canary_guardrail;
pub mod self_budget;
pub mod stateless_todo_list;
pub mod system_commands;
pub mod tool_call_repair;
pub mod tool_output_distillation;
pub mod tool_output_persistence;
pub mod tool_search;
pub mod usage_limit_auto_continue;

// Compatibility paths used by the collocated implementation tests. These are
// aliases of core's provider-neutral execution modules, not copied contracts.
pub use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityRegistry, CapabilityStatus, Fact, FactsContext,
    ModelViewContext, ModelViewProvider, RiskLevel, SystemPromptContext, ToolDefinitionHook,
    Volatility,
};
pub use everruns_core::{
    AgentLoopError, DEFAULT_ORG_PUBLIC_ID, LlmCompletionMetadata, LlmMessage, LlmMessageRole,
    LlmResponse, LlmResponseStream, McpToolInvoker, PrincipalId, Result, UtilityLlmService,
    WorkspaceId, retain_complete_llm_tool_exchanges, user_facing_error_codes,
};
pub use everruns_core::{
    atoms, budget, capability_types, command, command_host, driver_registry, error, events,
    guardrail_checks, llm_error_hook, mcp_server, message, message_filter, model_profiles,
    output_guardrail, provider, session, session_file, session_schedule, tool_fingerprint,
    tool_narration, tool_output_sanitizer, tool_types, tools, traits, typed_id, user_facing_error,
    utility_llm,
};

pub use agent_instructions::{
    AGENT_INSTRUCTIONS_CAPABILITY_ID, AGENTS_MD_PATH, AgentInstructionsCapability,
    AgentInstructionsConfig, DEFAULT_AGENT_INSTRUCTIONS_FILE, MAX_AGENT_INSTRUCTIONS_FILES,
    MAX_AGENTS_MD_SIZE, format_agents_md_content, format_instruction_file_content,
};
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
pub use loop_detection::{LOOP_DETECTION_CAPABILITY_ID, LoopDetectionCapability};
pub use message_metadata::{
    MESSAGE_METADATA_CAPABILITY_ID, MessageMetadataCapability, MessageMetadataConfig,
    MessageMetadataField, render_annotation, strip_leading_timestamp_annotations,
};
pub use openai_tool_search::{
    DEFAULT_TOOL_SEARCH_THRESHOLD, OPENAI_TOOL_SEARCH_CAPABILITY_ID, OpenAiToolSearchCapability,
    model_supports_native_tool_search,
};
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
pub use stateless_todo_list::{
    STATELESS_TODO_LIST_CAPABILITY_ID, StatelessTodoListCapability, WriteTodosTool,
};
pub use system_commands::{SYSTEM_COMMANDS_CAPABILITY_ID, SystemCommandsCapability};
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

/// Compatibility namespace for implementation tests and downstream code
/// transitioning from `everruns_core::capabilities`.
pub mod capabilities {
    pub use crate::{
        AUTO_TOOL_SEARCH_CAPABILITY_ID, AutoToolSearchCapability, CURRENT_TIME_CAPABILITY_ID,
        CurrentTimeCapability, DEFAULT_TOOL_SEARCH_THRESHOLD, FactsContext,
        OPENAI_TOOL_SEARCH_CAPABILITY_ID, OpenAiToolSearchCapability,
        STATELESS_TODO_LIST_CAPABILITY_ID, StatelessTodoListCapability, TOOL_SEARCH_CAPABILITY_ID,
        TOOL_SEARCH_TOOL_NAME, ToolSearchCapability, ToolSearchTool, Volatility,
    };
    pub use everruns_core::capabilities::*;
}

/// Register every portable policy capability in stable product order.
///
/// Registration is explicit: linking this crate has no inventory side effect.
/// Existing IDs and alias collisions are rejected rather than silently
/// replacing an application-provided implementation.
pub fn register_portable_capabilities(
    registry: &mut CapabilityRegistry,
) -> std::result::Result<(), CapabilityError> {
    for capability in portable_capabilities() {
        registry.try_register_arc(capability)?;
    }
    Ok(())
}

/// Build a registry containing only portable policy capabilities.
pub fn portable_capability_registry() -> std::result::Result<CapabilityRegistry, CapabilityError> {
    let mut registry = CapabilityRegistry::new();
    register_portable_capabilities(&mut registry)?;
    Ok(registry)
}

fn portable_capabilities() -> Vec<Arc<dyn Capability>> {
    vec![
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
        Arc::new(UsageLimitAutoContinueCapability),
    ]
}

#[cfg(test)]
mod bundle_tests {
    use super::*;

    const PORTABLE_IDS: [&str; 24] = [
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
        "usage_limit_auto_continue",
    ];

    #[test]
    fn bundle_has_the_curated_stable_catalog() {
        let capabilities = portable_capabilities();
        let ids: Vec<_> = capabilities
            .iter()
            .map(|capability| capability.id())
            .collect();
        assert_eq!(ids, PORTABLE_IDS);
    }

    #[test]
    fn duplicate_registration_is_rejected_without_replacement() {
        let mut registry = portable_capability_registry().unwrap();
        let original = Arc::clone(registry.get(AGENT_INSTRUCTIONS_CAPABILITY_ID).unwrap());

        let error = register_portable_capabilities(&mut registry).unwrap_err();

        assert!(error.is_duplicate());
        assert_eq!(error.id(), AGENT_INSTRUCTIONS_CAPABILITY_ID);
        assert!(Arc::ptr_eq(
            &original,
            registry.get(AGENT_INSTRUCTIONS_CAPABILITY_ID).unwrap()
        ));
    }

    #[test]
    fn dependencies_are_portable_or_explicit_host_seams() {
        let registry = portable_capability_registry().unwrap();
        let host_dependencies = ["session_file_system"];

        for capability in registry.list() {
            for dependency in capability.dependencies() {
                assert!(
                    registry.has(dependency) || host_dependencies.contains(&dependency),
                    "{} has an undeclared external dependency on {dependency}",
                    capability.id()
                );
            }
        }
    }
}
