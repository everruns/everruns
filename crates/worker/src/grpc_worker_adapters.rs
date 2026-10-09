// gRPC implementation of WorkerAdapters
//
// Decision: Wraps GrpcClient and implements WorkerAdapters trait
// Decision: Used by external workers that connect to control-plane via gRPC

use crate::core::capabilities::CapabilityRegistry;
use crate::core::events::{Event, EventRequest};
use crate::core::leased_resource::LeasedResource;
use crate::core::session_file::{
    FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, SessionFile,
};
use crate::core::{AgentDefinition, HarnessDefinition};
use crate::core::{
    EgressService, ExecutionSession, MessageHistory, MessageQuery, RuntimeMessage,
    UtilityLlmService,
};
use crate::core::{
    connection_services::ProviderCredentialStore, image_services::ImageArtifactStore,
    image_services::ResolvedImage,
};
use crate::host::HostComposition;
use async_trait::async_trait;
use everruns_contracts::driver_registry::DriverRegistry;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::typed_id::{
    AgentId, HarnessId, LeasedResourceId, MessageId, ModelId, SessionId,
};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::grpc_adapters::{
    GrpcAdapter, GrpcBudgetChecker, GrpcClient, GrpcOrgAdapter, GrpcOutboundToolRateLimiter,
    GrpcPaymentAuthority, GrpcSessionCreationAuthority,
};
use crate::mcp_executor::McpServerInfo;
use crate::worker_adapters::{TurnContext, WorkerAdapters};

// =============================================================================
// GrpcWorkerAdapters Implementation
// =============================================================================

/// gRPC-backed worker adapters for external workers
#[derive(Clone)]
pub struct GrpcWorkerAdapters {
    client: GrpcClient,
    host_composition: HostComposition,
    stream_heartbeater: Option<Arc<dyn crate::core::durability::StreamHeartbeater>>,
}

impl GrpcWorkerAdapters {
    /// Create new gRPC adapters by connecting to control-plane
    pub async fn connect(grpc_address: &str) -> Result<Self> {
        Self::connect_with_host_composition(
            grpc_address,
            crate::platform::default_host_composition(),
        )
        .await
    }

    /// Create new gRPC adapters with an explicit platform definition.
    pub async fn connect_with_host_composition(
        grpc_address: &str,
        host_composition: HostComposition,
    ) -> Result<Self> {
        let client = GrpcClient::connect(grpc_address).await?;
        Ok(Self {
            client,
            host_composition,
            stream_heartbeater: None,
        })
    }

    /// Create from an existing GrpcClient
    pub fn from_client(client: GrpcClient) -> Self {
        Self::from_client_with_host_composition(client, crate::platform::default_host_composition())
    }

    /// Create from an existing GrpcClient with an explicit platform definition.
    pub fn from_client_with_host_composition(
        client: GrpcClient,
        host_composition: HostComposition,
    ) -> Self {
        Self {
            client,
            host_composition,
            stream_heartbeater: None,
        }
    }

    /// Set the stream heartbeater for liveness signalling (EVE-531).
    pub fn with_stream_heartbeater(
        mut self,
        heartbeater: Arc<dyn crate::core::durability::StreamHeartbeater>,
    ) -> Self {
        self.stream_heartbeater = Some(heartbeater);
        self
    }

    /// Get the underlying GrpcClient (for MCP executor)
    pub fn client(&self) -> GrpcClient {
        self.client.clone()
    }
}

#[async_trait]
impl WorkerAdapters for GrpcWorkerAdapters {
    // =========================================================================
    // Agent Operations
    // =========================================================================

    async fn get_agent(&self, org_id: i64, agent_id: Uuid) -> Result<Option<AgentDefinition>> {
        let store = GrpcOrgAdapter::new(self.client.clone(), org_id);
        crate::core::execution_loading::AgentStore::get_agent(&store, AgentId::from_uuid(agent_id))
            .await
    }

    async fn get_harness(
        &self,
        org_id: i64,
        harness_id: Uuid,
    ) -> Result<Option<HarnessDefinition>> {
        let store = GrpcOrgAdapter::new(self.client.clone(), org_id);
        // The server returns a single pre-merged stored record (EVE-881).
        crate::core::execution_loading::HarnessStore::get_harness(
            &store,
            HarnessId::from_uuid(harness_id),
        )
        .await
    }

    async fn get_agent_blocker(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<crate::core::DependencyBlocker>> {
        let store = GrpcOrgAdapter::new(self.client.clone(), org_id);
        crate::core::execution_loading::AgentStore::get_agent_blocker(
            &store,
            AgentId::from_uuid(id),
        )
        .await
    }
    async fn get_harness_blocker(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<crate::core::DependencyBlocker>> {
        let store = GrpcOrgAdapter::new(self.client.clone(), org_id);
        crate::core::execution_loading::HarnessStore::get_harness_blocker(
            &store,
            HarnessId::from_uuid(id),
        )
        .await
    }

    async fn resolve_agent_read(
        &self,
        org_id: i64,
        agent_id: Uuid,
    ) -> Result<(
        Result<Option<AgentDefinition>>,
        Option<crate::core::DependencyBlocker>,
    )> {
        GrpcOrgAdapter::new(self.client.clone(), org_id)
            .resolve_agent_read(AgentId::from_uuid(agent_id))
            .await
    }

    async fn resolve_harness_read(
        &self,
        org_id: i64,
        harness_id: Uuid,
    ) -> Result<(
        Result<Option<HarnessDefinition>>,
        Option<crate::core::DependencyBlocker>,
    )> {
        GrpcOrgAdapter::new(self.client.clone(), org_id)
            .resolve_harness_read(HarnessId::from_uuid(harness_id))
            .await
    }

    // =========================================================================
    // Session Operations
    // =========================================================================

    async fn get_session(&self, org_id: i64, session_id: Uuid) -> Result<Option<ExecutionSession>> {
        let store = GrpcOrgAdapter::new(self.client.clone(), org_id);
        crate::core::execution_loading::SessionStore::get_session(
            &store,
            SessionId::from_uuid(session_id),
        )
        .await
    }

    async fn set_session_status(&self, org_id: i64, session_id: Uuid, status: &str) -> Result<()> {
        self.client
            .set_session_status(org_id, SessionId::from_uuid(session_id), status)
            .await
    }

    async fn set_session_title(
        &self,
        org_id: i64,
        session_id: Uuid,
        title: String,
    ) -> Result<ExecutionSession> {
        self.client
            .set_session_title(org_id, SessionId::from_uuid(session_id), &title)
            .await
    }

    // =========================================================================
    // Message Operations
    // =========================================================================

    async fn get_message(
        &self,
        session_id: Uuid,
        message_id: Uuid,
    ) -> Result<Option<RuntimeMessage>> {
        let retriever = GrpcAdapter::new(self.client.clone());
        crate::core::MessageRetriever::get(
            &retriever,
            SessionId::from_uuid(session_id),
            MessageId::from_uuid(message_id),
        )
        .await
    }

    async fn load_messages(&self, session_id: Uuid) -> Result<Vec<RuntimeMessage>> {
        let retriever = GrpcAdapter::new(self.client.clone());
        crate::core::MessageRetriever::load(&retriever, SessionId::from_uuid(session_id)).await
    }

    async fn load_message_history(&self, query: MessageQuery) -> Result<MessageHistory> {
        let retriever = GrpcAdapter::new(self.client.clone());
        crate::core::MessageRetriever::load_filtered_history(&retriever, query).await
    }

    // =========================================================================
    // Event Operations
    // =========================================================================

    async fn emit_event(&self, request: EventRequest) -> Result<Event> {
        let emitter = GrpcAdapter::new(self.client.clone());
        crate::core::event_emitter::EventEmitter::emit(&emitter, request).await
    }

    async fn emit_events(&self, requests: Vec<EventRequest>) -> Result<()> {
        GrpcAdapter::new(self.client.clone())
            .emit_stored_batch(requests)
            .await
            .map(|_| ())
    }

    async fn emit_events_then(
        &self,
        mut requests: Vec<EventRequest>,
        last: EventRequest,
    ) -> Result<Event> {
        let provisional = crate::write_behind::provisional_event(&last);
        requests.push(last);
        let stored = GrpcAdapter::new(self.client.clone())
            .emit_stored_batch(requests)
            .await?;
        // A control plane older than this worker (during a rolling deploy)
        // stores the batch but does not return the last event; the event is
        // stored either way, only its sequence is unknown here.
        Ok(stored.unwrap_or(provisional))
    }

    // =========================================================================
    // LLM Provider Operations
    // =========================================================================

    async fn get_model_spec(&self, org_id: i64, model_id: Uuid) -> Result<Option<ModelSpec>> {
        let store = GrpcOrgAdapter::new(self.client.clone(), org_id);
        crate::core::provider_resolution::ProviderStore::get_model_spec(
            &store,
            ModelId::from_uuid(model_id),
        )
        .await
    }

    async fn get_default_model_spec(&self, org_id: i64) -> Result<Option<ModelSpec>> {
        let store = GrpcOrgAdapter::new(self.client.clone(), org_id);
        crate::core::provider_resolution::ProviderStore::get_default_model_spec(&store).await
    }

    async fn get_provider_config_for_session(
        &self,
        org_id: i64,
        provider: &everruns_contracts::ProviderKey,
        session: SessionId,
    ) -> Result<Option<everruns_contracts::driver_registry::ProviderConfig>> {
        self.client
            .get_provider_config_for_session(org_id, provider.as_str(), Some(session))
            .await
    }

    async fn get_provider_config(
        &self,
        org_id: i64,
        provider: &everruns_contracts::runtime_provider::ProviderKey,
    ) -> Result<Option<everruns_contracts::driver_registry::ProviderConfig>> {
        self.client
            .get_provider_config(org_id, provider.as_str())
            .await
    }

    // =========================================================================
    // Image Resolution Operations
    // =========================================================================

    async fn resolve_image(&self, org_id: i64, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        let resolver = GrpcOrgAdapter::new(self.client.clone(), org_id);
        crate::core::image_services::ImageResolver::resolve_image(&resolver, image_id).await
    }

    async fn resolve_images_batch(
        &self,
        org_id: i64,
        image_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ResolvedImage>> {
        let resolver = GrpcOrgAdapter::new(self.client.clone(), org_id);
        resolver
            .resolve_images_batch(image_ids)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve images: {}", e)))
    }

    async fn resolve_files_batch(
        &self,
        org_id: i64,
        file_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, crate::core::file_services::ResolvedFile>> {
        let resolver = GrpcOrgAdapter::new(self.client.clone(), org_id);
        resolver
            .resolve_files_batch(file_ids)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve files: {}", e)))
    }

    // =========================================================================
    // Session File Operations
    // =========================================================================

    async fn read_file(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFile>> {
        let store = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        crate::core::session_files::SessionFileSystem::read_file(
            &store,
            SessionId::from_uuid(session_id),
            path,
        )
        .await
    }

    async fn write_file(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let store = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        crate::core::session_files::SessionFileSystem::write_file(
            &store,
            SessionId::from_uuid(session_id),
            path,
            content,
            encoding,
        )
        .await
    }

    async fn write_file_if_content_matches(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        let store = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        crate::core::session_files::SessionFileSystem::write_file_if_content_matches(
            &store,
            SessionId::from_uuid(session_id),
            path,
            expected_content,
            expected_encoding,
            content,
            encoding,
        )
        .await
    }

    async fn delete_file(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        let store = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        crate::core::session_files::SessionFileSystem::delete_file(
            &store,
            SessionId::from_uuid(session_id),
            path,
            recursive,
        )
        .await
    }

    async fn list_directory(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<Vec<FileInfo>> {
        let store = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        crate::core::session_files::SessionFileSystem::list_directory(
            &store,
            SessionId::from_uuid(session_id),
            path,
        )
        .await
    }

    async fn stat_file(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<FileStat>> {
        let store = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        crate::core::session_files::SessionFileSystem::stat_file(
            &store,
            SessionId::from_uuid(session_id),
            path,
        )
        .await
    }

    async fn grep_files(
        &self,
        org_id: i64,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let store = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        crate::core::session_files::SessionFileSystem::grep_files(
            &store,
            SessionId::from_uuid(session_id),
            pattern,
            path_pattern,
        )
        .await
    }

    async fn grep_files_with_options(
        &self,
        org_id: i64,
        session_id: Uuid,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        let store = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        crate::core::session_files::SessionFileSystem::grep_files_with_options(
            &store,
            SessionId::from_uuid(session_id),
            pattern,
            options,
        )
        .await
    }

    async fn create_directory(
        &self,
        org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<FileInfo> {
        let store = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        crate::core::session_files::SessionFileSystem::create_directory(
            &store,
            SessionId::from_uuid(session_id),
            path,
        )
        .await
    }

    // =========================================================================
    // MCP Server Operations
    // =========================================================================

    async fn get_mcp_server_by_prefix(
        &self,
        org_id: i64,
        session_id: Option<Uuid>,
        server_prefix: &str,
    ) -> Result<McpServerInfo> {
        self.client
            .get_mcp_server_by_prefix(org_id, session_id, server_prefix)
            .await
    }

    // =========================================================================
    // Turn Context (batch operation)
    // =========================================================================

    async fn load_turn_context(&self, org_id: i64, session_id: Uuid) -> Result<TurnContext> {
        let ctx = crate::grpc_adapters::load_turn_context(
            &self.client,
            org_id,
            SessionId::from_uuid(session_id),
        )
        .await?;
        Ok(TurnContext {
            agent: ctx.agent,
            session: ctx.session,
            messages: ctx.messages,
            model: ctx.model,
            mcp_tool_definitions: ctx.mcp_tool_definitions,
        })
    }

    async fn load_turn_context_for_execution(
        &self,
        org_id: i64,
        session_id: Uuid,
        input_message_id: Uuid,
    ) -> Result<TurnContext> {
        let ctx = crate::grpc_adapters::load_turn_context_for_execution(
            &self.client,
            org_id,
            SessionId::from_uuid(session_id),
            Some(input_message_id),
        )
        .await?;
        Ok(TurnContext {
            agent: ctx.agent,
            session: ctx.session,
            messages: ctx.messages,
            model: ctx.model,
            mcp_tool_definitions: ctx.mcp_tool_definitions,
        })
    }
    async fn get_mcp_server_for_execution(
        &self,
        org_id: i64,
        session_id: Uuid,
        server_prefix: &str,
        input_message_id: Uuid,
    ) -> Result<crate::mcp_executor::McpServerInfo> {
        self.client
            .get_mcp_server_for_execution(org_id, session_id, server_prefix, input_message_id)
            .await
    }
    // =========================================================================
    // Factory Methods
    // =========================================================================

    fn capability_registry(&self) -> CapabilityRegistry {
        (*self.host_composition.capability_registry()).clone()
    }

    fn driver_registry(&self) -> DriverRegistry {
        self.host_composition.driver_registry().clone()
    }

    fn sqldb_store(
        &self,
        org_id: i64,
    ) -> std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore> {
        Arc::new(GrpcAdapter::new_org_scoped(self.client.clone(), org_id))
    }

    fn native_async_store(
        &self,
    ) -> Option<Arc<dyn crate::core::native_async_store::NativeAsyncStore>> {
        Some(Arc::new(GrpcAdapter::new(self.client.clone())))
    }

    fn agents_api_store(&self) -> Option<Arc<dyn crate::core::agents_api_store::AgentsApiStore>> {
        Some(Arc::new(GrpcAdapter::new(self.client.clone())))
    }

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn crate::core::CompactionCheckpointStore>> {
        Some(Arc::new(GrpcAdapter::new(self.client.clone())))
    }

    fn storage_store(
        &self,
        org_id: i64,
    ) -> Arc<dyn crate::core::session_services::SessionStorageStore> {
        let adapter = GrpcAdapter::new_org_scoped(self.client.clone(), org_id);
        Arc::new(crate::internal_commands::CommandSessionStorageStore::new(
            adapter.clone(),
            adapter,
        ))
    }

    fn image_artifact_store(&self, org_id: i64) -> Arc<dyn ImageArtifactStore> {
        Arc::new(GrpcOrgAdapter::new(self.client.clone(), org_id))
    }

    fn provider_credential_store(&self, org_id: i64) -> Arc<dyn ProviderCredentialStore> {
        Arc::new(GrpcOrgAdapter::new(self.client.clone(), org_id))
    }

    fn utility_llm_service(&self) -> Option<Arc<dyn UtilityLlmService>> {
        Some(self.host_composition.utility_llm_service())
    }

    fn decisions(&self) -> Option<Arc<dyn crate::core::DecisionsService>> {
        Some(self.host_composition.decisions())
    }

    fn egress_service(&self) -> Option<Arc<dyn EgressService>> {
        Some(self.host_composition.egress_service())
    }

    fn platform_store(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Arc<dyn everruns_capabilities::PlatformStore> {
        Arc::new(
            crate::grpc_adapters::GrpcOrgAdapter::new_for_platform_session(
                self.client.clone(),
                org_id,
                Some(session_id),
            ),
        )
    }

    fn connection_resolver(
        &self,
    ) -> Arc<dyn crate::core::connection_services::UserConnectionResolver> {
        Arc::new(crate::grpc_adapters::GrpcAdapter::new(self.client.clone()))
    }

    fn leased_resource_store(
        &self,
        org_id: i64,
    ) -> Arc<dyn crate::core::session_services::LeasedResourceStore> {
        Arc::new(crate::internal_commands::CommandLeasedResourceStore::new(
            crate::grpc_adapters::GrpcAdapter::new_org_scoped(self.client.clone(), org_id),
        ))
    }

    fn session_resource_registry(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn crate::core::session_services::SessionResourceRegistry>> {
        Some(Arc::new(
            crate::internal_commands::CommandSessionResourceRegistry::new(
                crate::grpc_adapters::GrpcAdapter::new_org_scoped(self.client.clone(), org_id),
            ),
        ))
    }

    fn session_task_registry(
        &self,
    ) -> Option<Arc<dyn crate::core::session_task::SessionTaskRegistry>> {
        Some(Arc::new(crate::grpc_adapters::GrpcAdapter::new(
            self.client.clone(),
        )))
    }

    fn schedule_store(
        &self,
        org_id: i64,
    ) -> Arc<dyn crate::core::session_services::SessionScheduleStore> {
        Arc::new(crate::internal_commands::CommandSessionScheduleStore::new(
            crate::grpc_adapters::GrpcAdapter::new_org_scoped(self.client.clone(), org_id),
        ))
    }

    fn budget_checker(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn crate::core::tool_execution::BudgetChecker>> {
        Some(Arc::new(
            GrpcBudgetChecker::new(self.client.clone(), org_id)
                .with_agent_id(agent_id.map(|id| id.to_string())),
        ))
    }

    fn slack_action_invoker(
        &self,
        org_id: i64,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Option<Arc<dyn everruns_capabilities::slack_action::SlackActionInvoker>> {
        Some(Arc::new(
            crate::grpc_slack_actions::GrpcSlackActionInvoker::new(
                self.client.clone(),
                org_id,
                session_id,
            ),
        ))
    }

    fn saved_script_store(
        &self,
        org_id: i64,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Option<Arc<dyn everruns_contracts::runtime::saved_scripts::SavedScriptStore>> {
        Some(Arc::new(crate::grpc_saved_scripts::GrpcSavedScripts::new(
            GrpcAdapter::new_org_scoped(self.client.clone(), org_id),
            session_id,
        )))
    }

    fn user_mcp_invoker(
        &self,
        org_id: i64,
        session_id: everruns_contracts::typed_id::SessionId,
    ) -> Option<Arc<dyn everruns_capabilities::capabilities::UserMcpCallInvoker>> {
        Some(Arc::new(crate::grpc_user_mcp::GrpcUserMcpInvoker::new(
            self.client.clone(),
            org_id,
            session_id,
        )))
    }

    fn sandbox_persistence_store(
        &self,
    ) -> Option<Arc<dyn everruns_capabilities::sandbox_state::SandboxPersistenceStore>> {
        Some(Arc::new(
            crate::grpc_sandbox_persistence::GrpcSandboxPersistenceStore::new(self.client.clone()),
        ))
    }

    fn payment_authority(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn crate::core::tool_execution::PaymentAuthority>> {
        Some(Arc::new(
            GrpcPaymentAuthority::new(self.client.clone(), org_id)
                .with_agent_id(agent_id.map(|id| id.to_string())),
        ))
    }

    fn session_creation_authority(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn crate::core::delegation_services::SessionCreationAuthority>> {
        Some(Arc::new(GrpcSessionCreationAuthority::new(
            self.client.clone(),
            org_id,
            session_id,
        )))
    }

    fn outbound_tool_rate_limiter(
        &self,
        _org_id: i64,
    ) -> Option<Arc<dyn crate::core::tool_execution::OutboundToolRateLimiter>> {
        Some(Arc::new(GrpcOutboundToolRateLimiter::new(
            self.client.clone(),
        )))
    }

    fn stream_heartbeater(&self) -> Option<Arc<dyn crate::core::durability::StreamHeartbeater>> {
        self.stream_heartbeater.clone()
    }

    fn partial_stream_store(&self) -> Option<Arc<dyn crate::core::durability::PartialStreamStore>> {
        Some(Arc::new(
            crate::grpc_partial_stream::GrpcPartialStreamStore::new(self.client.clone()),
        ))
    }

    async fn invoke_scheduled_channel(
        &self,
        org_id: i64,
        app_id: &str,
        channel_id: &str,
    ) -> Result<serde_json::Value> {
        self.client
            .invoke_scheduled_app_channel(org_id, app_id, channel_id)
            .await
    }

    async fn invoke_agent_trigger(
        &self,
        org_id: i64,
        agent_id: &str,
        trigger_id: &str,
    ) -> Result<serde_json::Value> {
        self.client
            .invoke_agent_trigger(org_id, agent_id, trigger_id)
            .await
    }

    async fn claim_due_leased_resources(
        &self,
        limit: u32,
        stale_after_seconds: u32,
    ) -> Result<Vec<(i64, LeasedResource)>> {
        self.client
            .claim_due_leased_resources(limit, stale_after_seconds)
            .await
    }

    async fn mark_leased_resource_released(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<bool> {
        self.client
            .mark_leased_resource_released(resource_id, expected_cleanup_started_at)
            .await
    }

    async fn mark_leased_resource_cleanup_failed(
        &self,
        resource_id: LeasedResourceId,
        expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
        retry_after_seconds: u32,
        error: &str,
    ) -> Result<bool> {
        self.client
            .mark_leased_resource_cleanup_failed(
                resource_id,
                expected_cleanup_started_at,
                retry_after_seconds,
                error,
            )
            .await
    }

    async fn list_orphaned_session_task_ids(
        &self,
        stale_after: chrono::Duration,
        limit: i64,
    ) -> Result<Vec<(i64, everruns_contracts::typed_id::SessionId, String)>> {
        self.client
            .list_orphaned_session_tasks(stale_after.num_seconds(), limit)
            .await
    }

    fn reaper_session_task_registry(
        &self,
    ) -> std::sync::Arc<dyn crate::core::session_task::SessionTaskRegistry> {
        // Reuse the same gRPC-backed session-task registry the worker uses for
        // executor RPCs — lifecycle invariants, events, and wake_policy all
        // flow through the server's DbSessionTaskRegistry.
        Arc::new(crate::grpc_adapters::GrpcAdapter::new(self.client.clone()))
    }

    async fn prune_terminal_session_tasks(
        &self,
        ttl: chrono::Duration,
        limit: i64,
    ) -> Result<usize> {
        // Pruning needs the storage backend + blob store, which only the server
        // holds, so it runs server-side via the PruneTerminalSessionTasks RPC.
        self.client
            .prune_terminal_session_tasks(ttl.num_seconds(), limit)
            .await
    }
}
