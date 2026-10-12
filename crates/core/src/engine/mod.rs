#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Shared turn planning and Input/Reason/Act execution for hosts in the
//! [Everruns](https://everruns.com) ecosystem.
//!
//! [`TurnExecution`](crate::engine::TurnExecution) owns the serializable state machine shared by every
//! execution driver. Given a parsed [`ActivityOutcome`](crate::engine::ActivityOutcome) and host-resolved
//! [`HostFacts`](crate::engine::HostFacts), it returns the next [`TurnPlan`](crate::engine::TurnPlan) and ordered
//! [`TurnLifecycleEffect`](crate::engine::TurnLifecycleEffect)s. Framework applications use `everruns`; this
//! optional `engine` module lets in-process, durable, and custom hosts share
//! one turn model.
//!
//! Planning is sans I/O: hosts pass resolved facts and perform returned
//! lifecycle effects. The execution phases are portable async algorithms over
//! injected core/provider contracts such as [`crate::MessageRetriever`],
//! [`crate::EventEmitter`], and [`crate::ToolExecutor`]. The
//! module does not select stores, transports, processes, or deployment services.
//! [`Execution`](crate::engine::Execution) is the driver boundary. Core’s optional host module implements
//! it with process-local state; the worker’s durable driver checkpoints the
//! same state between activities scheduled by `everruns-durable`. Both share
//! phase behavior and turn transitions without an engine-to-host dependency.
//!
//! # Example
//!
//! ```
//! use everruns_core::engine::{TurnPlan, TurnState};
//!
//! fn accepts_plan(_state: &TurnState, _plan: &TurnPlan) {}
//! # let _ = accepts_plan;
//! ```

mod execution;
pub use execution::configured_max_tool_concurrency;
mod machine;
pub mod native_async;
mod phase_effects;
#[cfg(test)]
mod test_fixtures;
mod turn;

// Internal aliases keep the execution algorithms focused on their contracts
// while preserving the one-way engine -> core/provider/capability boundary.
pub(crate) use crate::{
    ANTHROPIC_COMPACTION_CHECKPOINT_FORMAT_VERSION, COMPACTION_CHECKPOINT_FORMAT_VERSION,
    CompactionCheckpoint, CompactionCheckpointPayload, CompactionCheckpointStore, DecisionsService,
    EgressService, McpToolInvoker, MessageQuery, ProactiveCompactionAttempt, RuntimeAgent,
    UtilityLlmService, annotation_hook, capabilities, compaction_policy, connection_services,
    delegation_services, durability, event_emitter, events, execution_loading, file_services,
    finalized_tool_calls, image_services, llm_conversions, llm_error_hook, localization, message,
    message_retriever, mount_fs, network_access, output_guardrail, runtime_context, session_files,
    session_services, session_task, subagent_delegation, tool_context, tool_execution,
    tool_fingerprint, tool_narration, tools,
};
pub(crate) use everruns_contracts::CapabilityRef;
pub(crate) use everruns_contracts::user_facing_error::{
    ErrorDisclosure, UserFacingError, UserFacingErrorContext, codes as user_facing_error_codes,
};
pub(crate) use everruns_contracts::{
    ChatDriver, CompactInputItem, ProviderEndpoint, ProviderOpaqueContext, compact,
    driver_registry, error, llm_retry, model_profiles, tool_types, typed_id,
};

pub(crate) mod tool_call_integrity {
    pub(crate) use crate::{
        retain_complete_llm_tool_exchanges_for_request, retain_complete_message_tool_exchanges,
    };
}

pub use execution::capability_usage_records;
pub use execution::model_wait;
pub use execution::{
    ActAtom, ActInput, ActResult, ClientSideToolHook, ConnectionSetupHook, ExecutionContext,
    InputAtom, InputAtomInput, InputAtomResult, NativeExecutionCounts, OutputHardLimitHook,
    PostActAction, PostActHook, ReasonAtom, ReasonInput, ReasonResult, ToolApprovalPauseHook,
    ToolCallResult, has_pending_tool_approval,
};
pub use machine::{Execution, ExecutionTransition, TurnExecution};
pub use phase_effects::{PhaseEffect, PhaseEffectSink};
pub use turn::{
    ActOutcome, ActPlan, ActSchedulingFacts, ActivityOutcome, HostFacts, TurnLifecycleEffect,
    TurnPlan, TurnState, act_pauses_turn, plan_after_act, plan_after_process_input,
    plan_after_reason, plan_next_turn, reason_schedules_act,
};
