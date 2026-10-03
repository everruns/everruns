use crate::engine::capabilities::CapabilityRegistry;
use crate::engine::event_emitter::EventEmitter;
use crate::engine::finalized_tool_calls::{FinalizedToolCallRejection, FinalizedToolCallsContext};
use crate::engine::tool_types::{ToolCall, ToolDefinition};
use crate::engine::typed_id::SessionId;
use crate::engine::{CapabilityRef, ExecutionContext};

#[allow(clippy::too_many_arguments)]
pub(super) async fn apply_finalized_tool_calls_hooks(
    capability_registry: &CapabilityRegistry,
    event_emitter: &dyn EventEmitter,
    session_id: SessionId,
    context: &ExecutionContext,
    resolved_capability_configs: &[CapabilityRef],
    tool_definitions: &[ToolDefinition],
    tool_calls: &mut [ToolCall],
    iteration: u32,
) -> Vec<FinalizedToolCallRejection> {
    let hook_context = FinalizedToolCallsContext {
        event_emitter,
        session_id,
        execution_context: context,
        tool_definitions,
        iteration,
    };
    let mut rejections = Vec::new();
    for config in resolved_capability_configs {
        let Some(capability) = capability_registry.get(config.capability_id()) else {
            continue;
        };
        if let Some(hook) = capability.finalized_tool_calls_hook(config.config_value()) {
            rejections.extend(hook.apply_with_rejections(&hook_context, tool_calls).await);
        }
    }
    rejections
}
