//! Shared Input/Reason/Act execution kernel.

mod act;
mod act_hooks;
mod cpu_time;
pub use cpu_time::ThreadCpuClock;
mod input;
pub mod model_wait;
mod provider_checkpoint;
mod reason;
pub mod tool_detach;
mod tool_scheduler;
pub use tool_scheduler::configured_max_tool_concurrency;

pub use crate::execution_context::ExecutionContext;
pub use act::{ActAtom, ActInput, ActResult, ToolCallResult};
pub use act_hooks::{
    ClientSideToolHook, ConnectionSetupHook, OutputHardLimitHook, PostActAction, PostActHook,
    ToolApprovalPauseHook, has_pending_tool_approval,
};
pub use input::{InputAtom, InputAtomInput, InputAtomResult};
pub use reason::{
    NativeExecutionCounts, ReasonAtom, ReasonInput, ReasonResult, capability_usage_records,
};
