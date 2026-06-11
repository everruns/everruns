//! BackgroundExecution Capability — exposes `spawn_background` to the model
//! whenever the active tool set contains at least one built-in tool that opts
//! into detached execution via `ToolHints::supports_background`.
//!
//! This capability is auto-activated by
//! [`super::collect_capabilities_with_configs`] after the explicit-capability
//! collection loop. It can also be selected explicitly by id
//! (`"background_execution"`) — the auto-activator skips it in that case.
//!
//! See `specs/background-execution.md` for the cross-cutting / meta-tool
//! capability contract this implements.

use super::{Capability, CapabilityStatus};
use crate::tools::{SpawnBackgroundTool, Tool};

/// Capability id used by the auto-activator in
/// `collect_capabilities_with_configs`.
pub const BACKGROUND_EXECUTION_CAPABILITY_ID: &str = "background_execution";

/// Cross-cutting capability that contributes `spawn_background`.
///
/// Auto-activation rule: any collected tool with
/// `hints.supports_background == Some(true)` causes this capability to be
/// added to the agent's tool set, exposing `spawn_background` to the model
/// and registering it in the worker tool registry.
pub struct BackgroundExecutionCapability;

impl Capability for BackgroundExecutionCapability {
    fn id(&self) -> &str {
        BACKGROUND_EXECUTION_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Background Execution"
    }

    fn description(&self) -> &str {
        "Run any background-capable built-in tool asynchronously via \
         `spawn_background`. Auto-activated whenever the agent has a tool \
         that declares `supports_background=true`."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("zap")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(SpawnBackgroundTool)]
    }
}

/// Task executor for `background_tool` tasks. Background runs have no cancel
/// token today, so cancellation reports unsupported; inbound messages keep
/// the default unsupported error.
pub struct BackgroundToolTaskExecutor;

#[async_trait::async_trait]
impl crate::session_task::TaskExecutor for BackgroundToolTaskExecutor {
    fn kind(&self) -> &str {
        crate::session_task::TASK_KIND_BACKGROUND_TOOL
    }

    async fn cancel(
        &self,
        _task: &crate::session_task::SessionTask,
        _context: &crate::traits::ToolContext,
    ) -> crate::error::Result<()> {
        Err(crate::error::AgentLoopError::tool(
            "background tool runs do not support cancellation yet; \
             the run continues until it finishes",
        ))
    }
}

inventory::submit! {
    crate::session_task::TaskExecutorPlugin {
        executor: || std::sync::Arc::new(BackgroundToolTaskExecutor),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;

    #[test]
    fn capability_metadata() {
        let cap = BackgroundExecutionCapability;
        assert_eq!(cap.id(), BACKGROUND_EXECUTION_CAPABILITY_ID);
        assert_eq!(cap.id(), "background_execution");
        assert_eq!(cap.name(), "Background Execution");
        assert_eq!(cap.category(), Some("Execution"));
        assert_eq!(cap.icon(), Some("zap"));
        assert_eq!(cap.status(), CapabilityStatus::Available);
    }

    #[test]
    fn capability_contributes_spawn_background_tool() {
        let cap = BackgroundExecutionCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "spawn_background");
    }

    #[test]
    fn capability_tool_definition_carries_spawn_background() {
        let cap = BackgroundExecutionCapability;
        let defs = cap.tool_definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name(), "spawn_background");
    }

    #[test]
    fn capability_registered_in_builtins() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry
            .get(BACKGROUND_EXECUTION_CAPABILITY_ID)
            .expect("background_execution must be registered as a built-in capability");
        assert_eq!(cap.id(), BACKGROUND_EXECUTION_CAPABILITY_ID);
        assert_eq!(cap.tools().len(), 1);
    }
}
