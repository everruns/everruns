//! Neutral contracts for capability-contributed per-tool execution hooks.

use crate::runtime::tool_context::ToolContext;
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use async_trait::async_trait;

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

    /// Whether this hook's decision authorizes the exact call it saw (tool
    /// approval, guardrails), rather than transforming or observing it.
    ///
    /// The engine runs every policy gate after all other hooks, so a gate
    /// always decides on the arguments that will execute (EVE-1184). Wrap a
    /// hook in [`PolicyGate`] to mark it.
    fn is_policy_gate(&self) -> bool {
        false
    }
}

/// Marks a [`PreToolUseHook`] as a policy gate: its `Continue` is an
/// authorization for that exact call, so it must see the final arguments.
///
/// THREAT[TM-HOOK-007]: a transforming hook that ran after a gate could
/// otherwise rewrite an approved call into one nobody approved.
pub struct PolicyGate<H>(pub H);

#[async_trait]
impl<H: PreToolUseHook> PreToolUseHook for PolicyGate<H> {
    async fn before_exec(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> PreToolUseDecision {
        self.0.before_exec(tool_call, tool_def, context).await
    }

    fn is_policy_gate(&self) -> bool {
        true
    }
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

/// The act phase's per-call policy, handed to a tool that runs another tool on
/// the model's behalf (`spawn_background`, Lua code mode).
///
/// THREAT[TM-TOOL-055]: hooks see the outer call, so without this a nested
/// target call skips the target tool's approval, guardrail, and user hook
/// decisions (EVE-1186). The engine installs it on every act-phase
/// [`ToolContext`]; a dispatching tool runs its nested call through it and
/// refuses to dispatch when it is absent.
#[async_trait]
pub trait NestedToolPolicy: Send + Sync {
    /// Run the pre-tool chain on a nested call, exactly as for a direct one.
    ///
    /// `Ok` is the call to run, possibly rewritten by hooks; only it may be
    /// executed. `Err` is the outcome to record instead (a block, or a
    /// deferral such as a hosted approval request).
    async fn authorize(
        &self,
        tool_call: ToolCall,
        tool_def: &ToolDefinition,
        context: &ToolContext,
    ) -> Result<ToolCall, ToolResult>;

    /// Run the post-tool chain on the nested call's result.
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    );
}
