//! Neutral contracts for capability-contributed per-tool execution hooks.

use crate::tool_context::ToolContext;
use async_trait::async_trait;
use everruns_provider::tool_types::{ToolCall, ToolDefinition, ToolResult};

/// Decision returned by a [`PreToolUseHook`] before a tool is dispatched.
#[derive(Debug, Clone)]
pub enum PreToolUseDecision {
    /// Continue with the possibly transformed call.
    Continue(ToolCall),
    /// Block this call without affecting sibling calls in the batch.
    Block {
        /// Call that was blocked.
        tool_call: ToolCall,
        /// Error text recorded for the model and audit stream.
        reason: String,
        /// Optional message for a user-facing runtime.
        user_message: Option<String>,
    },
    /// Do not run this call yet; record `result` as its outcome instead.
    ///
    /// For gates that park the turn on a durable request rather than answer
    /// in-process (hosted tool approval): `result` carries a structured payload
    /// that a post-act hook turns into a pause, and its `error` is what the
    /// model reads. Like `Block`, the tool is never invoked and the chain stops.
    Defer {
        /// Call that was deferred.
        tool_call: ToolCall,
        /// Outcome recorded for the call in place of running it.
        result: ToolResult,
    },
}

/// Capability hook invoked before each individual tool execution.
#[async_trait]
pub trait PreToolUseHook: Send + Sync {
    /// Transform or block a tool call before dispatch.
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> PreToolUseDecision;
}

/// Ordering for capability-contributed post-tool hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PostToolExecHookPriority {
    /// Inspect or block output before normal mutating hooks.
    Guardrail = 0,
    /// Default ordering for transformation and observability hooks.
    Normal = 100,
}

/// Capability hook invoked after each individual tool execution.
#[async_trait]
pub trait PostToolExecHook: Send + Sync {
    /// Ordering within the capability-contributed hook phase.
    fn priority(&self) -> PostToolExecHookPriority {
        PostToolExecHookPriority::Normal
    }

    /// Inspect or transform one tool result before engine event emission.
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    );
}
