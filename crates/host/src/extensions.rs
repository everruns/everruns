//! Neutral extension seams for services layered above the execution host.

use async_trait::async_trait;
use everruns_core::execution_loading::SessionStore;
use everruns_core::session_files::SessionFileSystem;
use everruns_core::session_task::SessionTaskRegistry;
use everruns_core::subagent_delegation::SubagentSessionDelegate;
use everruns_core::tool_context::ToolContextExtensions;
use everruns_core::tools::ToolRegistry;
use everruns_provider::error::Result;
use everruns_provider::tool_types::ToolDefinition;
use everruns_provider::typed_id::SessionId;
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
