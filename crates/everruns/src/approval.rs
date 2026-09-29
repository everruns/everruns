//! Stability: alpha — may change without a major bump; see [`stability`](crate::stability).
//!
//! Host contract for approving individual tool calls.
//!
//! Mark a [`FunctionTool`](crate::FunctionTool) with
//! [`needs_approval`](crate::FunctionTool::needs_approval) or
//! [`always_needs_approval`](crate::FunctionTool::always_needs_approval), then
//! register a [`ToolApprover`] with
//! [`AgentBuilder::approver`](crate::AgentBuilder::approver). Before a gated
//! call runs, the approver receives the [`SessionId`](crate::SessionId), the
//! [`ToolCall`](crate::ToolCall) (its `id` is the tool call id, its
//! `arguments` the model's input), and the tool's
//! [`ToolDefinition`](crate::ToolDefinition), so a host serving many sessions
//! can route the request to the right person. The turn waits for the answer.
//!
//! A rejected call never runs its handler; the model receives a tool error
//! saying the call was rejected. [`ApprovalDecision::AllowAlways`] and
//! [`ApprovalDecision::RejectAlways`] are remembered per session and tool.

pub use async_trait::async_trait;
pub use everruns_builtins::{ApprovalDecision, ToolApprover};
