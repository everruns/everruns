//! Adapter from hosted platform services to the neutral execution-host seams.

use async_trait::async_trait;
use everruns_core::execution_loading::SessionStore;
use everruns_core::session_files::SessionFileSystem;
use everruns_core::session_task::SessionTaskRegistry;
use everruns_core::tools::{Tool, ToolRegistry};
use everruns_host::HostToolAugmentor;
use everruns_provider::error::Result;
use everruns_provider::tool_types::ToolDefinition;
use everruns_provider::typed_id::SessionId;
use std::sync::Arc;

use crate::PlatformStore;
use crate::capabilities::{
    report_result_tool_for_child_session, report_task_progress_tool_for_child_session,
};

/// Factory producing a platform store scoped to one organization and session.
pub type PlatformStoreFactory = Arc<dyn Fn(i64, SessionId) -> Arc<dyn PlatformStore> + Send + Sync>;

/// Hosted tool policy installed into `everruns-host` by platform compositions.
#[derive(Debug, Default)]
pub struct PlatformToolAugmentor;

/// Adds a hosted platform store to an embedded host without introducing a
/// `host -> platform` dependency.
pub trait PlatformHostBackendsExt: Sized {
    /// Install the platform store, neutral delegation adapter, and hosted
    /// turn-dependent tools.
    fn with_platform_store_factory(self, factory: PlatformStoreFactory) -> Self;
}

fn attach_platform_store(
    backends: everruns_host::HostBackends,
    factory: PlatformStoreFactory,
) -> everruns_host::HostBackends {
    let context_factory = factory.clone();
    let delegate_factory = factory;
    backends
        .with_tool_context_extensions_factory(Arc::new(move |org_id, session_id| {
            let mut extensions = everruns_core::tool_context::ToolContextExtensions::default();
            extensions.insert(Arc::new(crate::PlatformStoreExt(context_factory(
                org_id, session_id,
            ))));
            extensions
        }))
        .with_subagent_delegate_factory(Arc::new(move |org_id, session_id| {
            Arc::new(crate::PlatformStoreSubagentDelegate(delegate_factory(
                org_id, session_id,
            )))
        }))
        .with_tool_augmentor(Arc::new(PlatformToolAugmentor))
}

impl PlatformHostBackendsExt for everruns_host::HostBackends {
    fn with_platform_store_factory(self, factory: PlatformStoreFactory) -> Self {
        attach_platform_store(self, factory)
    }
}

#[async_trait]
impl HostToolAugmentor for PlatformToolAugmentor {
    fn disabled_hook_contributions(
        &self,
        capability_id: &str,
        config: &serde_json::Value,
    ) -> Vec<String> {
        if capability_id == "user_hooks" {
            crate::capabilities::user_hooks::disabled_contributions(config)
        } else {
            Vec::new()
        }
    }

    async fn augment_reason_tools(
        &self,
        session_id: SessionId,
        session_store: Arc<dyn SessionStore>,
        task_registry: Option<Arc<dyn SessionTaskRegistry>>,
        definitions: &mut Vec<ToolDefinition>,
    ) -> Result<()> {
        let Some(task_registry) = task_registry else {
            return Ok(());
        };
        if let Some(tool) = report_result_tool_for_child_session(
            session_id,
            session_store.as_ref(),
            task_registry.as_ref(),
        )
        .await?
        {
            definitions.push(tool.to_definition());
        }
        if let Some(tool) = report_task_progress_tool_for_child_session(
            session_id,
            session_store.as_ref(),
            task_registry.as_ref(),
        )
        .await?
        {
            definitions.push(tool.to_definition());
        }
        Ok(())
    }

    async fn augment_act_tools(
        &self,
        session_id: SessionId,
        session_store: Arc<dyn SessionStore>,
        task_registry: Option<Arc<dyn SessionTaskRegistry>>,
        file_store: Arc<dyn SessionFileSystem>,
        requested_definitions: &[ToolDefinition],
        registry: &mut ToolRegistry,
    ) -> Result<()> {
        let Some(task_registry) = task_registry else {
            return Ok(());
        };
        if requested_definitions
            .iter()
            .any(|definition| definition.name() == "report_result")
            && let Some(tool) = report_result_tool_for_child_session(
                session_id,
                session_store.as_ref(),
                task_registry.as_ref(),
            )
            .await?
        {
            registry.register_boxed(Box::new(tool.with_file_store(file_store)));
        }
        if requested_definitions
            .iter()
            .any(|definition| definition.name() == "report_task_progress")
            && let Some(tool) = report_task_progress_tool_for_child_session(
                session_id,
                session_store.as_ref(),
                task_registry.as_ref(),
            )
            .await?
        {
            registry.register_boxed(Box::new(tool));
        }
        Ok(())
    }
}
