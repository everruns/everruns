//! Store-backed turn-context resolution for runtime hosts.

use std::sync::Arc;

use everruns_core::ResolvedExecutionSnapshot;
use everruns_core::capabilities::{CapabilityRegistry, collect_message_filters_only};
use everruns_core::execution_loading::{AgentStore, HarnessStore, SessionStore};
use everruns_core::message::{Message, MessageRole};
use everruns_core::message_filter::MessageQuery;
use everruns_core::message_retriever::MessageRetriever;
use everruns_core::provider_resolution::ProviderStore;
use everruns_core::runtime_context::{
    AssembledTurnContext, ResolvedModelExecution, ResolvedTurnContextInput, TurnContextRequest,
    TurnContextResolver, assemble_resolved_turn_context, resolve_snapshot_capabilities,
};
use everruns_core::session_files::SessionFileSystem;
use everruns_provider::driver_registry::{ChatDriver, DriverRegistry};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_provider::tool_types::ToolDefinition;
use everruns_provider::typed_id::{AgentId, HarnessId, ModelId, SessionId};

use crate::execution_snapshot::load_execution_snapshot;

/// Store-backed resolver used by direct atom callers and custom hosts.
#[derive(Clone)]
pub struct StoreTurnContextResolver {
    harness_store: Arc<dyn HarnessStore>,
    agent_store: Arc<dyn AgentStore>,
    session_store: Arc<dyn SessionStore>,
    message_retriever: Arc<dyn MessageRetriever>,
    provider_store: Arc<dyn ProviderStore>,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    file_store: Option<Arc<dyn SessionFileSystem>>,
}

impl StoreTurnContextResolver {
    /// Construct a resolver from org-scoped runtime stores and registries.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        harness_store: Arc<dyn HarnessStore>,
        agent_store: Arc<dyn AgentStore>,
        session_store: Arc<dyn SessionStore>,
        message_retriever: Arc<dyn MessageRetriever>,
        provider_store: Arc<dyn ProviderStore>,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
    ) -> Self {
        Self {
            harness_store,
            agent_store,
            session_store,
            message_retriever,
            provider_store,
            capability_registry,
            driver_registry,
            file_store: None,
        }
    }

    /// Supply the session filesystem used by dynamic prompt capabilities.
    pub fn with_file_store(mut self, file_store: Arc<dyn SessionFileSystem>) -> Self {
        self.file_store = Some(file_store);
        self
    }
}

#[async_trait::async_trait]
impl TurnContextResolver for StoreTurnContextResolver {
    async fn resolve_turn_context(
        &self,
        request: TurnContextRequest,
    ) -> Result<AssembledTurnContext> {
        assemble_turn_context(
            self.harness_store.as_ref(),
            self.agent_store.as_ref(),
            self.session_store.as_ref(),
            self.message_retriever.as_ref(),
            self.provider_store.as_ref(),
            &self.capability_registry,
            &self.driver_registry,
            request.session_id,
            request.harness_id,
            request.agent_id,
            &request.mcp_tool_definitions,
            self.file_store.clone(),
        )
        .await
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AssemblyMode {
    RequireMessages,
    AllowEmptyMessages,
}

#[allow(clippy::too_many_arguments)]
/// Load and assemble a reason context, requiring at least one model-visible
/// message.
pub async fn assemble_turn_context(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session_store: &dyn SessionStore,
    message_retriever: &dyn MessageRetriever,
    provider_store: &dyn ProviderStore,
    capability_registry: &CapabilityRegistry,
    driver_registry: &DriverRegistry,
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    mcp_tool_definitions: &[ToolDefinition],
    file_store: Option<Arc<dyn SessionFileSystem>>,
) -> Result<AssembledTurnContext> {
    let snapshot =
        load_execution_snapshot(harness_store, agent_store, session_store, session_id).await?;
    validate_requested_topology(&snapshot, harness_id, agent_id)?;
    assemble_from_snapshot(
        snapshot,
        message_retriever,
        provider_store,
        capability_registry,
        driver_registry,
        mcp_tool_definitions,
        file_store,
        AssemblyMode::RequireMessages,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
/// Load and assemble a context for inspection, allowing empty history.
pub async fn inspect_turn_context(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session_store: &dyn SessionStore,
    message_retriever: &dyn MessageRetriever,
    provider_store: &dyn ProviderStore,
    capability_registry: &CapabilityRegistry,
    driver_registry: &DriverRegistry,
    session_id: SessionId,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
    mcp_tool_definitions: &[ToolDefinition],
    file_store: Option<Arc<dyn SessionFileSystem>>,
) -> Result<AssembledTurnContext> {
    let snapshot =
        load_execution_snapshot(harness_store, agent_store, session_store, session_id).await?;
    validate_requested_topology(&snapshot, harness_id, agent_id)?;
    assemble_from_snapshot(
        snapshot,
        message_retriever,
        provider_store,
        capability_registry,
        driver_registry,
        mcp_tool_definitions,
        file_store,
        AssemblyMode::AllowEmptyMessages,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn inspect_turn_context_for_session(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    session_store: &dyn SessionStore,
    message_retriever: &dyn MessageRetriever,
    provider_store: &dyn ProviderStore,
    capability_registry: &CapabilityRegistry,
    driver_registry: &DriverRegistry,
    session_id: SessionId,
    mcp_tool_definitions: &[ToolDefinition],
    file_store: Option<Arc<dyn SessionFileSystem>>,
) -> Result<AssembledTurnContext> {
    let snapshot =
        load_execution_snapshot(harness_store, agent_store, session_store, session_id).await?;
    assemble_from_snapshot(
        snapshot,
        message_retriever,
        provider_store,
        capability_registry,
        driver_registry,
        mcp_tool_definitions,
        file_store,
        AssemblyMode::AllowEmptyMessages,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
/// Assemble a reason context from a snapshot a host already loaded.
pub async fn assemble_turn_context_from_snapshot(
    snapshot: ResolvedExecutionSnapshot,
    message_retriever: &dyn MessageRetriever,
    provider_store: &dyn ProviderStore,
    capability_registry: &CapabilityRegistry,
    driver_registry: &DriverRegistry,
    mcp_tool_definitions: &[ToolDefinition],
    file_store: Option<Arc<dyn SessionFileSystem>>,
) -> Result<AssembledTurnContext> {
    assemble_from_snapshot(
        snapshot,
        message_retriever,
        provider_store,
        capability_registry,
        driver_registry,
        mcp_tool_definitions,
        file_store,
        AssemblyMode::RequireMessages,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn assemble_from_snapshot(
    snapshot: ResolvedExecutionSnapshot,
    message_retriever: &dyn MessageRetriever,
    provider_store: &dyn ProviderStore,
    capability_registry: &CapabilityRegistry,
    driver_registry: &DriverRegistry,
    mcp_tool_definitions: &[ToolDefinition],
    file_store: Option<Arc<dyn SessionFileSystem>>,
    mode: AssemblyMode,
) -> Result<AssembledTurnContext> {
    let resolved = resolve_snapshot_capabilities(&snapshot, capability_registry);
    let filters = collect_message_filters_only(
        &resolved.effective_overlay.capabilities,
        capability_registry,
    );
    let mut query = MessageQuery::new(snapshot.session_id);
    filters.apply_message_filters(&mut query);
    let history = message_retriever.load_filtered_history(query).await?;
    let mut messages = history.messages;
    filters.apply_post_load_filters(&mut messages);
    if messages.is_empty() && mode == AssemblyMode::RequireMessages {
        return Err(AgentLoopError::NoMessages);
    }

    let controls_model_id = latest_model_override(&messages);
    let (model, resolved_model_id) =
        resolve_model(provider_store, controls_model_id, snapshot.default_model_id).await?;
    let model = resolve_model_execution(provider_store, driver_registry, model).await?;

    assemble_resolved_turn_context(
        ResolvedTurnContextInput {
            snapshot,
            messages,
            message_source_sequence: history.source_sequence,
            model,
            resolved_model_id,
            mcp_tool_definitions: mcp_tool_definitions.to_vec(),
        },
        capability_registry,
        file_store,
    )
    .await
}

fn validate_requested_topology(
    snapshot: &ResolvedExecutionSnapshot,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
) -> Result<()> {
    if snapshot.harness_id != harness_id || snapshot.agent_id != agent_id {
        return Err(AgentLoopError::config(format!(
            "resolved topology mismatch for session {}",
            snapshot.session_id
        )));
    }
    Ok(())
}

fn latest_model_override(messages: &[Message]) -> Option<ModelId> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::User)
        .and_then(|message| message.controls.as_ref())
        .and_then(|controls| controls.model_id)
}

async fn resolve_model(
    provider_store: &dyn ProviderStore,
    controls_model_id: Option<ModelId>,
    snapshot_model_id: Option<ModelId>,
) -> Result<(ModelSpec, Option<ModelId>)> {
    for model_id in [controls_model_id, snapshot_model_id].into_iter().flatten() {
        if let Some(model) = provider_store.get_model_spec(model_id).await? {
            return Ok((model, Some(model_id)));
        }
    }
    let model = provider_store
        .get_default_model_spec()
        .await?
        .ok_or_else(|| {
        AgentLoopError::llm(
            "No model configured: no model_id in controls or execution snapshot, and no system default model is set",
        )
    })?;
    Ok((model, None))
}

/// Resolve provider construction independently from credential-free model identity.
pub(crate) async fn resolve_model_execution(
    provider_store: &dyn ProviderStore,
    driver_registry: &DriverRegistry,
    spec: ModelSpec,
) -> Result<ResolvedModelExecution> {
    let config = provider_store
        .get_provider_config(&spec.provider)
        .await?
        .unwrap_or_else(|| {
            everruns_provider::driver_registry::ProviderConfig::for_provider(
                spec.provider.clone(),
                DriverId::external(spec.provider.as_str()),
            )
        });
    let provider_type = config.provider_type.clone();
    let driver: Arc<dyn ChatDriver> = Arc::from(driver_registry.create_chat_driver(&config)?);
    Ok(ResolvedModelExecution {
        model: spec.model,
        provider: spec.provider,
        provider_type,
        driver,
    })
}
