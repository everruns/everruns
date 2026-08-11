// gRPC implementation of WorkerAdapters
//
// Decision: Wraps GrpcClient and implements WorkerAdapters trait
// Decision: Used by external workers that connect to control-plane via gRPC

use async_trait::async_trait;
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{Event, EventRequest};
use everruns_core::leased_resource::LeasedResource;
use everruns_core::session_file::{
    FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, SessionFile,
};
use everruns_core::traits::{
    ImageArtifactStore, ProviderCredentialStore, ResolvedImage, ResolvedModel,
};
use everruns_core::typed_id::{
    AgentId, HarnessId, LeasedResourceId, MessageId, ModelId, SessionId,
};
use everruns_core::{
    DriverRegistry, EgressService, ExecutionSession, Message, MessageHistory, MessageQuery,
    PlatformDefinition, UtilityLlmService,
};
use everruns_platform::{Agent, Harness};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

use crate::grpc_adapters::{
    GrpcAgentStore, GrpcBudgetChecker, GrpcClient, GrpcEventEmitter, GrpcHarnessStore,
    GrpcImageArtifactStore, GrpcImageResolver, GrpcLeasedResourceStore, GrpcMessageRetriever,
    GrpcOutboundToolRateLimiter, GrpcPaymentAuthority, GrpcProviderCredentialStore,
    GrpcProviderStore, GrpcSessionCreationAuthority, GrpcSessionFileStore, GrpcSessionSqlDbStore,
    GrpcSessionStorageStore, GrpcSessionStore,
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
    platform_definition: PlatformDefinition,
    stream_heartbeater: Option<Arc<dyn everruns_core::traits::StreamHeartbeater>>,
}

impl GrpcWorkerAdapters {
    /// Create new gRPC adapters by connecting to control-plane
    pub async fn connect(grpc_address: &str) -> Result<Self> {
        Self::connect_with_platform_definition(
            grpc_address,
            crate::platform::default_platform_definition(),
        )
        .await
    }

    /// Create new gRPC adapters with an explicit platform definition.
    pub async fn connect_with_platform_definition(
        grpc_address: &str,
        platform_definition: PlatformDefinition,
    ) -> Result<Self> {
        let client = GrpcClient::connect(grpc_address).await?;
        Ok(Self {
            client,
            platform_definition,
            stream_heartbeater: None,
        })
    }

    /// Create from an existing GrpcClient
    pub fn from_client(client: GrpcClient) -> Self {
        Self::from_client_with_platform_definition(
            client,
            crate::platform::default_platform_definition(),
        )
    }

    /// Create from an existing GrpcClient with an explicit platform definition.
    pub fn from_client_with_platform_definition(
        client: GrpcClient,
        platform_definition: PlatformDefinition,
    ) -> Self {
        Self {
            client,
            platform_definition,
            stream_heartbeater: None,
        }
    }

    /// Set the stream heartbeater for liveness signalling (EVE-531).
    pub fn with_stream_heartbeater(
        mut self,
        heartbeater: Arc<dyn everruns_core::traits::StreamHeartbeater>,
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

    async fn get_agent(&self, org_id: i64, agent_id: Uuid) -> Result<Option<Agent>> {
        let store = GrpcAgentStore::new(self.client.clone(), org_id);
        store.fetch_agent_record(AgentId::from_uuid(agent_id)).await
    }

    async fn get_harness(&self, org_id: i64, harness_id: Uuid) -> Result<Option<Harness>> {
        let store = GrpcHarnessStore::new(self.client.clone(), org_id);
        // The server returns a single pre-merged stored record (EVE-881).
        store
            .fetch_harness_record(HarnessId::from_uuid(harness_id))
            .await
    }

    // =========================================================================
    // Session Operations
    // =========================================================================

    async fn get_session(&self, org_id: i64, session_id: Uuid) -> Result<Option<ExecutionSession>> {
        let store = GrpcSessionStore::new(self.client.clone(), org_id);
        everruns_core::traits::SessionStore::get_session(&store, SessionId::from_uuid(session_id))
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

    async fn get_message(&self, session_id: Uuid, message_id: Uuid) -> Result<Option<Message>> {
        let retriever = GrpcMessageRetriever::new(self.client.clone());
        everruns_core::MessageRetriever::get(
            &retriever,
            SessionId::from_uuid(session_id),
            MessageId::from_uuid(message_id),
        )
        .await
    }

    async fn load_messages(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let retriever = GrpcMessageRetriever::new(self.client.clone());
        everruns_core::MessageRetriever::load(&retriever, SessionId::from_uuid(session_id)).await
    }

    async fn load_message_history(&self, query: MessageQuery) -> Result<MessageHistory> {
        let retriever = GrpcMessageRetriever::new(self.client.clone());
        everruns_core::MessageRetriever::load_filtered_history(&retriever, query).await
    }

    // =========================================================================
    // Event Operations
    // =========================================================================

    async fn emit_event(&self, request: EventRequest) -> Result<Event> {
        let emitter = GrpcEventEmitter::new(self.client.clone());
        everruns_core::traits::EventEmitter::emit(&emitter, request).await
    }

    // =========================================================================
    // LLM Provider Operations
    // =========================================================================

    async fn get_resolved_model(
        &self,
        org_id: i64,
        model_id: Uuid,
    ) -> Result<Option<ResolvedModel>> {
        let store = GrpcProviderStore::new(self.client.clone(), org_id);
        everruns_core::traits::ProviderStore::get_resolved_model(
            &store,
            ModelId::from_uuid(model_id),
        )
        .await
    }

    async fn get_default_model(&self, org_id: i64) -> Result<Option<ResolvedModel>> {
        let store = GrpcProviderStore::new(self.client.clone(), org_id);
        everruns_core::traits::ProviderStore::get_default_model(&store).await
    }

    async fn get_provider_config(
        &self,
        org_id: i64,
        provider: &everruns_core::ProviderKey,
    ) -> Result<Option<everruns_core::ProviderConfig>> {
        self.client
            .get_provider_config(org_id, provider.as_str())
            .await
    }

    // =========================================================================
    // Image Resolution Operations
    // =========================================================================

    async fn resolve_image(&self, org_id: i64, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        let resolver = GrpcImageResolver::new(self.client.clone(), org_id);
        everruns_core::traits::ImageResolver::resolve_image(&resolver, image_id).await
    }

    async fn resolve_images_batch(
        &self,
        org_id: i64,
        image_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ResolvedImage>> {
        let resolver = GrpcImageResolver::new(self.client.clone(), org_id);
        resolver
            .resolve_images_batch(image_ids)
            .await
            .map_err(|e| AgentLoopError::store(format!("Failed to resolve images: {}", e)))
    }

    // =========================================================================
    // Session File Operations
    // =========================================================================

    async fn read_file(&self, session_id: Uuid, path: &str) -> Result<Option<SessionFile>> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileSystem::read_file(
            &store,
            SessionId::from_uuid(session_id),
            path,
        )
        .await
    }

    async fn write_file(
        &self,
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileSystem::write_file(
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
        session_id: Uuid,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileSystem::write_file_if_content_matches(
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

    async fn delete_file(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileSystem::delete_file(
            &store,
            SessionId::from_uuid(session_id),
            path,
            recursive,
        )
        .await
    }

    async fn list_directory(&self, session_id: Uuid, path: &str) -> Result<Vec<FileInfo>> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileSystem::list_directory(
            &store,
            SessionId::from_uuid(session_id),
            path,
        )
        .await
    }

    async fn stat_file(&self, session_id: Uuid, path: &str) -> Result<Option<FileStat>> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileSystem::stat_file(
            &store,
            SessionId::from_uuid(session_id),
            path,
        )
        .await
    }

    async fn grep_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileSystem::grep_files(
            &store,
            SessionId::from_uuid(session_id),
            pattern,
            path_pattern,
        )
        .await
    }

    async fn grep_files_with_options(
        &self,
        session_id: Uuid,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileSystem::grep_files_with_options(
            &store,
            SessionId::from_uuid(session_id),
            pattern,
            options,
        )
        .await
    }

    async fn create_directory(&self, session_id: Uuid, path: &str) -> Result<FileInfo> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileSystem::create_directory(
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

    // =========================================================================
    // Factory Methods
    // =========================================================================

    fn capability_registry(&self) -> CapabilityRegistry {
        self.platform_definition.capability_registry().clone()
    }

    fn driver_registry(&self) -> DriverRegistry {
        self.platform_definition.driver_registry().clone()
    }

    fn sqldb_store(&self) -> everruns_core::traits::SessionSqlDbStoreRef {
        Arc::new(GrpcSessionSqlDbStore::new(self.client.clone()))
    }

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::CompactionCheckpointStore>> {
        Some(Arc::new(GrpcMessageRetriever::new(self.client.clone())))
    }

    fn storage_store(&self) -> Arc<dyn everruns_core::traits::SessionStorageStore> {
        Arc::new(GrpcSessionStorageStore::new(self.client.clone()))
    }

    fn image_artifact_store(&self, org_id: i64) -> Arc<dyn ImageArtifactStore> {
        Arc::new(GrpcImageArtifactStore::new(self.client.clone(), org_id))
    }

    fn provider_credential_store(&self, org_id: i64) -> Arc<dyn ProviderCredentialStore> {
        Arc::new(GrpcProviderCredentialStore::new(
            self.client.clone(),
            org_id,
        ))
    }

    fn utility_llm_service(&self) -> Option<Arc<dyn UtilityLlmService>> {
        Some(self.platform_definition.utility_llm_service())
    }

    fn egress_service(&self) -> Option<Arc<dyn EgressService>> {
        Some(self.platform_definition.egress_service())
    }

    fn platform_store(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Arc<dyn everruns_platform::PlatformStore> {
        Arc::new(
            crate::grpc_adapters::GrpcPlatformStore::new_for_platform_session(
                self.client.clone(),
                org_id,
                Some(session_id),
            ),
        )
    }

    fn connection_resolver(&self) -> Arc<dyn everruns_core::traits::UserConnectionResolver> {
        Arc::new(crate::grpc_adapters::GrpcConnectionResolver::new(
            self.client.clone(),
        ))
    }

    fn leased_resource_store(&self) -> Arc<dyn everruns_core::traits::LeasedResourceStore> {
        Arc::new(GrpcLeasedResourceStore::new(self.client.clone()))
    }

    fn session_resource_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::traits::SessionResourceRegistry>> {
        Some(Arc::new(
            crate::grpc_adapters::GrpcSessionResourceRegistry::new(self.client.clone()),
        ))
    }

    fn session_task_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_task::SessionTaskRegistry>> {
        Some(Arc::new(
            crate::grpc_adapters::GrpcSessionTaskRegistry::new(self.client.clone()),
        ))
    }

    fn schedule_store(&self, org_id: i64) -> Arc<dyn everruns_core::traits::SessionScheduleStore> {
        Arc::new(crate::grpc_adapters::GrpcScheduleStore::new(
            self.client.clone(),
            org_id,
        ))
    }

    fn budget_checker(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn everruns_core::traits::BudgetChecker>> {
        Some(Arc::new(
            GrpcBudgetChecker::new(self.client.clone(), org_id)
                .with_agent_id(agent_id.map(|id| id.to_string())),
        ))
    }

    fn payment_authority(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn everruns_core::traits::PaymentAuthority>> {
        Some(Arc::new(
            GrpcPaymentAuthority::new(self.client.clone(), org_id)
                .with_agent_id(agent_id.map(|id| id.to_string())),
        ))
    }

    fn session_creation_authority(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn everruns_core::traits::SessionCreationAuthority>> {
        Some(Arc::new(GrpcSessionCreationAuthority::new(
            self.client.clone(),
            org_id,
            session_id,
        )))
    }

    fn outbound_tool_rate_limiter(
        &self,
        _org_id: i64,
    ) -> Option<Arc<dyn everruns_core::traits::OutboundToolRateLimiter>> {
        Some(Arc::new(GrpcOutboundToolRateLimiter::new(
            self.client.clone(),
        )))
    }

    fn stream_heartbeater(&self) -> Option<Arc<dyn everruns_core::StreamHeartbeater>> {
        self.stream_heartbeater.clone()
    }

    async fn invoke_scheduled_app_channel(
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
    ) -> Result<Vec<LeasedResource>> {
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
    ) -> Result<Vec<(everruns_core::SessionId, String)>> {
        self.client
            .list_orphaned_session_tasks(stale_after.num_seconds(), limit)
            .await
    }

    fn reaper_session_task_registry(
        &self,
    ) -> std::sync::Arc<dyn everruns_core::session_task::SessionTaskRegistry> {
        // Reuse the same gRPC-backed session-task registry the worker uses for
        // executor RPCs — lifecycle invariants, events, and wake_policy all
        // flow through the server's DbSessionTaskRegistry.
        Arc::new(crate::grpc_adapters::GrpcSessionTaskRegistry::new(
            self.client.clone(),
        ))
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
