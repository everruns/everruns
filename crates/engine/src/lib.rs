//! Deprecated compatibility shim for the canonical core module.
//!
//! This is the final forwarding release. Enable the matching `everruns-core`
//! feature and migrate imports to its module before the next platform release.
//!
//! ```
//! use everruns_core::engine::{TurnPlan, TurnState};
//! fn accepts_plan(_: &TurnState, _: &TurnPlan) {}
//! let _ = accepts_plan;
//! ```

#![allow(deprecated)]

#[deprecated(note = "use everruns_core::engine::ActAtom")]
pub use everruns_core::engine::ActAtom;
#[deprecated(note = "use everruns_core::engine::ActInput")]
pub use everruns_core::engine::ActInput;
#[deprecated(note = "use everruns_core::engine::ActOutcome")]
pub use everruns_core::engine::ActOutcome;
#[deprecated(note = "use everruns_core::engine::ActPlan")]
pub use everruns_core::engine::ActPlan;
#[deprecated(note = "use everruns_core::engine::ActResult")]
pub use everruns_core::engine::ActResult;
#[deprecated(note = "use everruns_core::engine::ActSchedulingFacts")]
pub use everruns_core::engine::ActSchedulingFacts;
#[deprecated(note = "use everruns_core::engine::ActivityOutcome")]
pub use everruns_core::engine::ActivityOutcome;
#[deprecated(note = "use everruns_core::engine::ClientSideToolHook")]
pub use everruns_core::engine::ClientSideToolHook;
#[deprecated(note = "use everruns_core::engine::ConnectionSetupHook")]
pub use everruns_core::engine::ConnectionSetupHook;
#[deprecated(note = "use everruns_core::engine::Execution")]
pub use everruns_core::engine::Execution;
#[deprecated(note = "use everruns_core::engine::ExecutionContext")]
pub use everruns_core::engine::ExecutionContext;
#[deprecated(note = "use everruns_core::engine::ExecutionTransition")]
pub use everruns_core::engine::ExecutionTransition;
#[deprecated(note = "use everruns_core::engine::HostFacts")]
pub use everruns_core::engine::HostFacts;
#[deprecated(note = "use everruns_core::engine::InputAtom")]
pub use everruns_core::engine::InputAtom;
#[deprecated(note = "use everruns_core::engine::InputAtomInput")]
pub use everruns_core::engine::InputAtomInput;
#[deprecated(note = "use everruns_core::engine::InputAtomResult")]
pub use everruns_core::engine::InputAtomResult;
#[deprecated(note = "use everruns_core::engine::NativeExecutionCounts")]
pub use everruns_core::engine::NativeExecutionCounts;
#[deprecated(note = "use everruns_core::engine::OutputHardLimitHook")]
pub use everruns_core::engine::OutputHardLimitHook;
#[deprecated(note = "use everruns_core::engine::PhaseEffect")]
pub use everruns_core::engine::PhaseEffect;
#[deprecated(note = "use everruns_core::engine::PhaseEffectSink")]
pub use everruns_core::engine::PhaseEffectSink;
#[deprecated(note = "use everruns_core::engine::PostActAction")]
pub use everruns_core::engine::PostActAction;
#[deprecated(note = "use everruns_core::engine::PostActHook")]
pub use everruns_core::engine::PostActHook;
#[deprecated(note = "use everruns_core::engine::ReasonAtom")]
pub use everruns_core::engine::ReasonAtom;
#[deprecated(note = "use everruns_core::engine::ReasonInput")]
pub use everruns_core::engine::ReasonInput;
#[deprecated(note = "use everruns_core::engine::ReasonResult")]
pub use everruns_core::engine::ReasonResult;
#[deprecated(note = "use everruns_core::engine::ToolApprovalPauseHook")]
pub use everruns_core::engine::ToolApprovalPauseHook;
#[deprecated(note = "use everruns_core::engine::ToolCallResult")]
pub use everruns_core::engine::ToolCallResult;
#[deprecated(note = "use everruns_core::engine::TurnExecution")]
pub use everruns_core::engine::TurnExecution;
#[deprecated(note = "use everruns_core::engine::TurnLifecycleEffect")]
pub use everruns_core::engine::TurnLifecycleEffect;
#[deprecated(note = "use everruns_core::engine::TurnPlan")]
pub use everruns_core::engine::TurnPlan;
#[deprecated(note = "use everruns_core::engine::TurnState")]
pub use everruns_core::engine::TurnState;
#[deprecated(note = "use everruns_core::engine::act_pauses_turn")]
pub use everruns_core::engine::act_pauses_turn;
#[deprecated(note = "use everruns_core::engine::capability_usage_records")]
pub use everruns_core::engine::capability_usage_records;
#[deprecated(note = "use everruns_core::engine::configured_max_tool_concurrency")]
pub use everruns_core::engine::configured_max_tool_concurrency;
#[deprecated(note = "use everruns_core::engine::has_pending_tool_approval")]
pub use everruns_core::engine::has_pending_tool_approval;
#[deprecated(note = "use everruns_core::engine::plan_after_act")]
pub use everruns_core::engine::plan_after_act;
#[deprecated(note = "use everruns_core::engine::plan_after_process_input")]
pub use everruns_core::engine::plan_after_process_input;
#[deprecated(note = "use everruns_core::engine::plan_after_reason")]
pub use everruns_core::engine::plan_after_reason;
#[deprecated(note = "use everruns_core::engine::plan_next_turn")]
pub use everruns_core::engine::plan_next_turn;
#[deprecated(note = "use everruns_core::engine::reason_schedules_act")]
pub use everruns_core::engine::reason_schedules_act;
#[deprecated(note = "use everruns_core::engine")]
pub use everruns_core::engine::*;
