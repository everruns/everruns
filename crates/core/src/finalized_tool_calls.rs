//! Neutral hook seam for capability-owned transforms after model tool calls
//! have been finalized and before the assistant message is persisted.

use async_trait::async_trait;

use crate::event_emitter::EventEmitter;
use crate::execution_context::ExecutionContext;
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::typed_id::SessionId;

/// A finalized model tool call that must fail before tool dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedToolCallRejection {
    /// Provider-assigned identifier of the rejected call.
    pub tool_call_id: String,
    /// Error returned to the model as the tool result.
    pub error: String,
}

/// Per-turn context supplied to finalized tool-call hooks.
pub struct FinalizedToolCallsContext<'a> {
    pub event_emitter: &'a dyn EventEmitter,
    pub session_id: SessionId,
    pub execution_context: &'a ExecutionContext,
    pub tool_definitions: &'a [ToolDefinition],
    pub iteration: u32,
}

/// Capability-owned transform over a completed model tool-call batch.
#[async_trait]
pub trait FinalizedToolCallsHook: Send + Sync {
    async fn apply(&self, context: &FinalizedToolCallsContext<'_>, calls: &mut [ToolCall]);

    /// Apply transforms and return calls that must enter the ordinary tool-error path.
    ///
    /// The default preserves existing transform-only hook behavior.
    async fn apply_with_rejections(
        &self,
        context: &FinalizedToolCallsContext<'_>,
        calls: &mut [ToolCall],
    ) -> Vec<FinalizedToolCallRejection> {
        self.apply(context, calls).await;
        Vec::new()
    }
}
