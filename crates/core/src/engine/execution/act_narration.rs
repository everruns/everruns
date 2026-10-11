//! Per-call narration for the act atom: capability hooks, then the executing
//! tool, then the generic phrasing.

use super::{ActAtom, EventEmitter, ToolExecutor};
use crate::engine::execution::ExecutionContext;
use crate::engine::tool_narration::{
    ToolNarrationContext, ToolNarrationPhase, render_tool_narration_with_locale,
};
use crate::engine::tool_types::{ToolCall, ToolDefinition, ToolResult};

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync + 'static,
    E: EventEmitter + Send + Sync + 'static,
{
    pub(super) fn render_tool_narration(
        &self,
        execution_context: &ExecutionContext,
        tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
    ) -> String {
        self.render_tool_narration_with_result(
            execution_context,
            tool_def,
            tool_call,
            phase,
            locale,
            None,
        )
    }

    /// Narration that can read the completed tool's result and details, so a
    /// finished phrase can say what happened.
    pub(super) fn render_tool_narration_with_result(
        &self,
        execution_context: &ExecutionContext,
        tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        outcome: Option<&ToolResult>,
    ) -> String {
        let wrapped_store = self.wrap_file_store_for_narration(execution_context);
        let ctx = ToolNarrationContext::new(wrapped_store.as_deref())
            .with_result(outcome.and_then(|outcome| outcome.result.as_ref()))
            .with_details(outcome.and_then(|outcome| outcome.details.as_ref()));
        for hook in &self.tool_call_hooks {
            if let Some(narration) = hook.narration(tool_def, tool_call, phase, locale, ctx) {
                return narration;
            }
        }
        // Capability hooks only reach tools a capability lists in `tools()`.
        // Tools assembled outside any capability — the unified `spawn_agent`
        // dispatcher built from delegation targets, registry-augmented and
        // proxied tools — still own narration, so ask the executing tool
        // directly before falling back to the generic display-name phrasing.
        if let Some(narration) = self
            .context_services
            .tool_registry
            .as_ref()
            .and_then(|registry| registry.get(&tool_call.name))
            .and_then(|tool| tool.narrate(tool_call, phase, locale, ctx))
        {
            return narration;
        }
        render_tool_narration_with_locale(tool_def, tool_call, phase, locale)
    }
}
