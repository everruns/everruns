//! Per-call narration for the act atom: capability hooks, then the executing
//! tool, then the generic phrasing.

use std::sync::Arc;

use super::{ActAtom, EventEmitter, ToolExecutor};
use crate::engine::capabilities::ToolCallHook;
use crate::engine::execution::ExecutionContext;
use crate::engine::session_files::SessionFileSystem;
use crate::engine::tool_narration::{
    GroupHeadlineAction, ToolNarrationContext, ToolNarrationPhase,
    render_tool_narration_with_locale, summarize_group_actions, tool_call_for_group_summary,
};
use crate::engine::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::engine::tools::ToolRegistry;

impl<T, E> ActAtom<T, E>
where
    T: ToolExecutor + Send + Sync + 'static,
    E: EventEmitter + Send + Sync + 'static,
{
    pub(super) fn render_group_headline(
        &self,
        execution_context: &ExecutionContext,
        tool_calls: &[ToolCall],
        tool_map: &std::collections::HashMap<&str, &ToolDefinition>,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
    ) -> Option<String> {
        if tool_calls.is_empty() {
            return None;
        }
        if let [tool_call] = tool_calls {
            return Some(self.render_tool_narration(
                execution_context,
                tool_map.get(tool_call.name.as_str()).copied(),
                tool_call,
                phase,
                locale,
            ));
        }

        let actions = tool_calls
            .iter()
            .map(|tool_call| {
                let tool_def = tool_map.get(tool_call.name.as_str()).copied();
                let narration = self.render_tool_narration(
                    execution_context,
                    tool_def,
                    tool_call,
                    phase,
                    locale,
                );
                let repeated_narration = self.render_tool_narration(
                    execution_context,
                    tool_def,
                    &tool_call_for_group_summary(tool_call),
                    phase,
                    locale,
                );
                GroupHeadlineAction::new(tool_call, narration, repeated_narration)
            })
            .collect::<Vec<_>>();

        Some(summarize_group_actions(&actions, locale))
    }

    /// Mirror the file-store wrapping applied during tool execution so
    /// path-bearing narration uses the same mount resolver and workspace key.
    fn wrap_file_store_for_narration(
        &self,
        execution_context: &ExecutionContext,
    ) -> Option<Arc<dyn SessionFileSystem>> {
        let store = self.context_services.file_store.as_ref()?.clone();
        let store = if let Some(workspace_id) = execution_context.workspace_id {
            crate::engine::session_files::WorkspaceScopedFileSystem::wrap(store, workspace_id)
        } else {
            store
        };
        Some(crate::engine::mount_fs::MountFs::wrap_if_needed(store))
    }

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
        self.narrator(execution_context)
            .render(tool_def, tool_call, phase, locale, outcome)
    }

    /// What per-call narration reads, owned, so a call that outlives its act
    /// (a detached call, see `tool_detach`) can still narrate its completion.
    pub(super) fn narrator(&self, execution_context: &ExecutionContext) -> Narrator {
        Narrator {
            tool_call_hooks: self.tool_call_hooks.clone(),
            tool_registry: self.context_services.tool_registry.clone(),
            file_store: self.wrap_file_store_for_narration(execution_context),
        }
    }
}

/// Per-call narration: capability hooks, then the executing tool, then the
/// generic phrasing.
#[derive(Clone)]
pub(super) struct Narrator {
    tool_call_hooks: Vec<Arc<dyn ToolCallHook>>,
    tool_registry: Option<Arc<ToolRegistry>>,
    file_store: Option<Arc<dyn SessionFileSystem>>,
}

impl Narrator {
    pub(super) fn render(
        &self,
        tool_def: Option<&ToolDefinition>,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        outcome: Option<&ToolResult>,
    ) -> String {
        let ctx = ToolNarrationContext::new(self.file_store.as_deref())
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
