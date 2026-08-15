//! Shared turn planning and Input/Reason/Act execution for hosts in the
//! [Everruns](https://everruns.com) ecosystem.
//!
//! [`TurnExecution`] owns the serializable state machine shared by every
//! execution driver. Given a parsed [`ActivityOutcome`] and host-resolved
//! [`HostFacts`], it returns the next [`TurnPlan`] and ordered
//! [`TurnLifecycleEffect`]s. Framework applications use `everruns`; this
//! focused crate lets in-process, durable, and custom hosts share one turn
//! model.
//!
//! Planning is sans I/O: hosts pass resolved facts and perform returned
//! lifecycle effects. The execution phases are portable async algorithms over
//! injected core/provider contracts such as [`everruns_core::MessageRetriever`],
//! [`everruns_core::EventEmitter`], and [`everruns_core::ToolExecutor`]. The
//! crate does not select stores, transports, processes, or deployment services.
//! [`Execution`] is the driver boundary. `everruns-host` implements it with
//! process-local state; `everruns-durable` implements it with checkpointed
//! state between scheduled activities. Both share phase behavior and turn
//! transitions without introducing an engine-to-host dependency.
//!
//! # Example
//!
//! ```
//! use everruns_engine::{TurnPlan, TurnState};
//!
//! fn accepts_plan(_state: &TurnState, _plan: &TurnPlan) {}
//! # let _ = accepts_plan;
//! ```

mod execution;
mod machine;
mod phase_effects;
#[cfg(test)]
mod test_fixtures;
mod turn;

// Internal aliases keep the execution algorithms focused on their contracts
// while preserving the one-way engine -> core/provider/capability boundary.
pub(crate) use everruns_capability::CapabilityRef;
pub(crate) use everruns_core::{
    COMPACTION_CHECKPOINT_FORMAT_VERSION, CompactionCheckpoint, CompactionCheckpointPayload,
    CompactionCheckpointStore, EgressService, McpToolInvoker, MessageQuery,
    ProactiveCompactionAttempt, RuntimeAgent, UtilityLlmService, annotation_hook, capabilities,
    compaction_policy, connection_services, delegation_services, durability, event_emitter, events,
    execution_loading, finalized_tool_calls, image_services, llm_conversions, llm_error_hook,
    localization, message, message_retriever, mount_fs, network_access, output_guardrail,
    runtime_context, session_files, session_services, session_task, subagent_delegation,
    tool_context, tool_execution, tool_fingerprint, tool_narration, tools,
};
pub(crate) use everruns_provider::user_facing_error::{
    ErrorDisclosure, UserFacingError, UserFacingErrorContext, codes as user_facing_error_codes,
};
pub(crate) use everruns_provider::{
    ChatDriver, CompactInputItem, ProviderEndpoint, ProviderOpaqueContext, compact,
    driver_registry, error, llm_retry, model, model_profiles, tool_types, typed_id,
};

pub(crate) mod tool_call_integrity {
    pub(crate) use everruns_core::{
        retain_complete_llm_tool_exchanges_for_request, retain_complete_message_tool_exchanges,
    };
}

pub use execution::{
    ActAtom, ActInput, ActResult, ClientSideToolHook, ConnectionSetupHook, ExecutionContext,
    InputAtom, InputAtomInput, InputAtomResult, OutputHardLimitHook, PostActAction, PostActHook,
    ReasonAtom, ReasonInput, ReasonResult, ToolCallResult,
};
pub use machine::{Execution, ExecutionTransition, TurnExecution};
pub use phase_effects::{PhaseEffect, PhaseEffectSink};
pub use turn::{
    ActOutcome, ActPlan, ActSchedulingFacts, ActivityOutcome, HostFacts, TurnLifecycleEffect,
    TurnPlan, TurnState, plan_after_act, plan_after_process_input, plan_after_reason,
    plan_next_turn, reason_schedules_act,
};
