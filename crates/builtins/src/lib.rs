//! Portable built-in capabilities for Everruns.
//!
//! These are the policy capabilities that shape a turn without touching an
//! environment edge (no filesystem, shell, network or interpreter) and without
//! a hosted product service: context compaction and masking, tool-search and
//! progressive disclosure, loop/progress guards, tool-call repair, tool-output
//! handling, prompt caching and parallel-tool preference, guardrails, and the
//! budget/time/intent prompt contributions (EVE-884).
//!
//! `everruns-core` owns the neutral [`Capability`](everruns_core::capabilities::Capability)
//! contract and the collection hooks these implementations plug into; it does
//! not own the implementations. Hosts install this bundle explicitly:
//!
//! ```rust
//! let mut registry = everruns_core::CapabilityRegistry::new();
//! everruns_builtins::register_builtins(&mut registry);
//! assert!(registry.has("current_time"));
//! ```

pub mod auto_tool_search;
pub mod btw;
pub mod budgeting;
pub mod claude_tool_search;
pub mod compaction;
pub mod current_time;
pub mod error_disclosure;
pub mod guardrails;
pub mod human_intent;
pub mod infinity_context;
pub mod loop_detection;
pub mod message_metadata;
pub mod openai_tool_search;
pub mod openrouter_server_tools;
pub mod parallel_tool_calls;
pub mod progress_guard;
pub mod prompt_caching;
pub mod prompt_canary_guardrail;
pub mod self_budget;
pub mod stateless_todo_list;
pub mod system_commands;
pub mod tool_approval;
pub mod tool_call_repair;
pub mod tool_output_distillation;
pub mod tool_output_persistence;
pub mod tool_search;
pub mod usage_limit_auto_continue;

pub use auto_tool_search::{AUTO_TOOL_SEARCH_CAPABILITY_ID, AutoToolSearchCapability};
pub use btw::{BTW_CAPABILITY_ID, BtwCapability};
pub use budgeting::{BUDGETING_CAPABILITY_ID, BudgetingCapability};
pub use claude_tool_search::{CLAUDE_TOOL_SEARCH_CAPABILITY_ID, ClaudeToolSearchCapability};
pub use compaction::{
    COMPACTION_CAPABILITY_ID, CompactionCapability, CompactionConfig, CompactionStep,
    CompactionStrategy, CostControlConfig, CostControlMaskingResult, HierarchicalMemoryConfig,
    MaskingSummaryFormat, MemoryTier, ObservationMaskingConfig, ObservationMaskingResult,
    SessionCompactionMetrics, SummarizationConfig, aggressive_trim, apply_cost_control_masking,
    apply_hierarchical_memory, apply_observation_masking, build_model_view_messages,
    build_summarization_prompt, build_summary_message, classify_memory_tiers,
    compose_summary_with_recent, estimate_tokens, estimate_total_tokens,
    format_messages_for_summarization, should_compact_for_cost, should_compact_proactively,
    total_tool_result_bytes,
};
pub use current_time::{CURRENT_TIME_CAPABILITY_ID, CurrentTimeCapability, GetCurrentTimeTool};
pub use error_disclosure::{
    ERROR_DISCLOSURE_CAPABILITY_ID, ErrorDisclosureCapability, resolve_error_disclosure,
};
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
pub use openrouter_server_tools::{
    OPENROUTER_SERVER_TOOLS_CAPABILITY_ID, OpenRouterServerToolsCapability,
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
pub use usage_limit_auto_continue::{
    AutoContinueConfig, USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID, UsageLimitAutoContinueCapability,
    resolve_usage_limit_auto_continue,
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
