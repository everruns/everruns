//! Neutral hook seam for capability-owned transforms after model tool calls
//! have been finalized and before the assistant message is persisted.

use async_trait::async_trait;

use crate::atoms::AtomContext;
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::traits::EventEmitter;
use crate::typed_id::SessionId;

/// Per-turn context supplied to finalized tool-call hooks.
pub struct FinalizedToolCallsContext<'a> {
    pub event_emitter: &'a dyn EventEmitter,
    pub session_id: SessionId,
    pub atom_context: &'a AtomContext,
    pub tool_definitions: &'a [ToolDefinition],
    pub iteration: u32,
}

/// Capability-owned transform over a completed model tool-call batch.
#[async_trait]
pub trait FinalizedToolCallsHook: Send + Sync {
    async fn apply(&self, context: &FinalizedToolCallsContext<'_>, calls: &mut [ToolCall]);
}
