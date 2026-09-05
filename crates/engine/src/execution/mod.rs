//! Shared Input/Reason/Act execution kernel.

mod act;
mod act_hooks;
mod input;
mod reason;
mod tool_scheduler;
pub use tool_scheduler::configured_max_tool_concurrency;

pub use act::{ActAtom, ActInput, ActResult, ToolCallResult};
pub use act_hooks::{
    ClientSideToolHook, ConnectionSetupHook, OutputHardLimitHook, PostActAction, PostActHook,
};
pub use everruns_core::execution_context::ExecutionContext;
pub use input::{InputAtom, InputAtomInput, InputAtomResult};
pub use reason::{NativeExecutionCounts, ReasonAtom, ReasonInput, ReasonResult};
