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
pub mod tool_output_distillation;
pub mod tool_search;
pub mod usage_limit_auto_continue;

pub use auto_tool_search::{AUTO_TOOL_SEARCH_CAPABILITY_ID, AutoToolSearchCapability};
pub use btw::{BTW_CAPABILITY_ID, BtwCapability};
pub use budgeting::{BUDGETING_CAPABILITY_ID, BudgetingCapability};
pub use claude_tool_search::{CLAUDE_TOOL_SEARCH_CAPABILITY_ID, ClaudeToolSearchCapability};
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
pub use system_commands::{SYSTEM_COMMANDS_CAPABILITY_ID, SystemCommandsCapability};
pub use tool_approval::{
    ApprovalDecision, ApprovalMode, TOOL_APPROVAL_CAPABILITY_ID, ToolApprovalCapability,
    ToolApprover,
};
pub use tool_output_distillation::{
    DistillOutputHook, TOOL_OUTPUT_DISTILLATION_CAPABILITY_ID, ToolOutputDistillationCapability,
};
pub use tool_search::{
    TOOL_SEARCH_CAPABILITY_ID, TOOL_SEARCH_TOOL_NAME, ToolSearchCapability, ToolSearchTool,
};
pub use usage_limit_auto_continue::{
    AutoContinueConfig, USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID, UsageLimitAutoContinueCapability,
    resolve_usage_limit_auto_continue,
};

/// Register the built-ins that are safe on any host, including the default
/// in-process runtime: prompt/middleware policy, context management, tool
/// search and progressive disclosure, guards, and tool-output handling.
///
/// Registration fails loudly on a duplicate id, so a host that already
/// installed one of these gets a registry panic rather than a silently
/// shadowed implementation.
pub fn register_runtime_builtins(registry: &mut everruns_core::capabilities::CapabilityRegistry) {
    registry.register(HumanIntentCapability);
    registry.register(CurrentTimeCapability);
    registry.register(MessageMetadataCapability);
    registry.register(StatelessTodoListCapability);
    registry.register(BtwCapability);
    registry.register(InfinityContextCapability);
    registry.register(BudgetingCapability);
    registry.register(SelfBudgetCapability);
    registry.register(ErrorDisclosureCapability);
    registry.register(OpenAiToolSearchCapability::new());
    registry.register(ClaudeToolSearchCapability::new());
    registry.register(ToolSearchCapability::new());
    registry.register(AutoToolSearchCapability::new());
    registry.register(PromptCachingCapability::new());
    registry.register(ParallelToolCallsCapability);
    registry.register(SystemCommandsCapability);
    registry.register(ToolOutputDistillationCapability);
    registry.register(LoopDetectionCapability);
    registry.register(ProgressGuardCapability::new());
    registry.register(PromptCanaryGuardrailCapability);
    registry.register(GuardrailsCapability);
}

/// [`register_runtime_builtins`] plus the built-ins that need product host
/// services to be useful.
///
/// `openrouter_server_tools` needs provider credentials, and
/// `usage_limit_auto_continue` needs a schedule store plus a poller to fire the
/// continuation — neither exists in the default in-process runtime, so they are
/// registered only by product composition.
pub fn register_product_builtins(registry: &mut everruns_core::capabilities::CapabilityRegistry) {
    register_runtime_builtins(registry);
    registry.register(OpenRouterServerToolsCapability);
    registry.register(UsageLimitAutoContinueCapability);
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::CapabilityRegistry;

    #[test]
    fn runtime_bundle_registers_every_portable_policy_capability() {
        let mut registry = CapabilityRegistry::new();
        register_runtime_builtins(&mut registry);
        for id in [
            "human_intent",
            "current_time",
            "message_metadata",
            "stateless_todo_list",
            "btw",
            "infinity_context",
            "budgeting",
            "self_budget",
            "error_disclosure",
            "openai_tool_search",
            "claude_tool_search",
            "tool_search",
            "auto_tool_search",
            "prompt_caching",
            "parallel_tool_calls",
            "system_commands",
            "tool_output_distillation",
            "loop_detection",
            "progress_guard",
            "prompt_canary_guardrail",
            "guardrails",
        ] {
            assert!(registry.has(id), "runtime bundle is missing `{id}`");
        }
    }

    #[test]
    fn product_bundle_adds_the_host_service_backed_builtins() {
        let mut runtime = CapabilityRegistry::new();
        register_runtime_builtins(&mut runtime);
        assert!(!runtime.has("openrouter_server_tools"));
        assert!(!runtime.has("usage_limit_auto_continue"));

        let mut product = CapabilityRegistry::new();
        register_product_builtins(&mut product);
        assert!(product.has("openrouter_server_tools"));
        assert!(product.has("usage_limit_auto_continue"));
    }

    #[test]
    fn bundles_are_disjoint_from_the_core_preset() {
        // Core keeps the kernel-invoked implementations (compaction, tool-call
        // repair, tool-output persistence) and the session/skills family; the
        // bundle must not collide with them.
        let mut registry = everruns_core::capabilities::CapabilityRegistry::runtime_builtins();
        register_runtime_builtins(&mut registry);
        assert!(registry.has("compaction"));
        assert!(registry.has("current_time"));
    }
}
