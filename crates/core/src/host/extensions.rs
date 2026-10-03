//! Neutral extension seams for services layered above the execution host.

use crate::execution_loading::SessionStore;
use crate::session_files::SessionFileSystem;
use crate::session_task::SessionTaskRegistry;
use crate::subagent_delegation::SubagentSessionDelegate;
use crate::tool_context::ToolContextExtensions;
use crate::tools::ToolRegistry;
use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::tool_types::ToolDefinition;
use everruns_contracts::typed_id::SessionId;
use std::sync::Arc;

/// Factory for type-erased tool services supplied by a higher-level host.
pub type ToolContextExtensionsFactory =
    Arc<dyn Fn(i64, SessionId) -> ToolContextExtensions + Send + Sync>;

/// Factory for the neutral subagent delegate supplied by a higher-level host.
pub type SubagentDelegateFactory =
    Arc<dyn Fn(i64, SessionId) -> Arc<dyn SubagentSessionDelegate> + Send + Sync>;

/// Adds higher-level, execution-time tools without coupling the host to their
/// implementation crate.
#[async_trait]
pub trait HostToolAugmentor: Send + Sync {
    /// Return contribution IDs muted by a higher-level capability config.
    fn disabled_hook_contributions(
        &self,
        _capability_id: &str,
        _config: &serde_json::Value,
    ) -> Vec<String> {
        Vec::new()
    }

    /// Add definitions that should be visible to the model for this turn.
    async fn augment_reason_tools(
        &self,
        session_id: SessionId,
        session_store: Arc<dyn SessionStore>,
        task_registry: Option<Arc<dyn SessionTaskRegistry>>,
        definitions: &mut Vec<ToolDefinition>,
    ) -> Result<()>;

    /// Add executable tools requested by the reason phase to the act registry.
    async fn augment_act_tools(
        &self,
        session_id: SessionId,
        session_store: Arc<dyn SessionStore>,
        task_registry: Option<Arc<dyn SessionTaskRegistry>>,
        file_store: Arc<dyn SessionFileSystem>,
        requested_definitions: &[ToolDefinition],
        registry: &mut ToolRegistry,
    ) -> Result<()>;
}

/// Factory for a host's shell-hook dispatcher, using the resolved session filesystem.
pub type BashHookDispatcherFactory = Arc<
    dyn Fn(Arc<dyn SessionFileSystem>) -> Arc<dyn crate::hook_executor::BashHookDispatcher>
        + Send
        + Sync,
>;

/// Disabled shell hook implementation for hosts without an injected dispatcher.
pub struct DisabledBashHookDispatcher;

#[async_trait]
impl crate::hook_executor::BashHookDispatcher for DisabledBashHookDispatcher {
    async fn dispatch(
        &self,
        _payload: &crate::hook_executor::HookPayload,
        _command: &str,
        _extra_env: &std::collections::BTreeMap<String, String>,
        _opts: &crate::hook_executor::ExecutorOpts,
    ) -> std::result::Result<crate::hook_executor::BashExecOutput, String> {
        Err("bash hooks require an injected BashHookDispatcher".to_string())
    }
}
