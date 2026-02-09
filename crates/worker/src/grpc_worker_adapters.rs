// gRPC implementation of WorkerAdapters
//
// Decision: Wraps GrpcClient and implements WorkerAdapters trait
// Decision: Used by external workers that connect to control-plane via gRPC

use async_trait::async_trait;
use everruns_core::capabilities::{CapabilityRegistry, collect_capabilities, is_mcp_capability};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{Event, EventRequest};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::traits::{AgentStore, ResolvedImage};
use everruns_core::typed_id::{AgentId, MessageId, ModelId, SessionId};
use everruns_core::{Agent, DriverRegistry, Message, Session, ToolRegistry};
use std::collections::HashMap;
use uuid::Uuid;

use crate::adapters::create_driver_registry;
use crate::grpc_adapters::{
    GrpcAgentStore, GrpcClient, GrpcEventEmitter, GrpcImageResolver, GrpcLlmProviderStore,
    GrpcMessageRetriever, GrpcSessionFileStore, GrpcSessionStore,
};
use crate::mcp_executor::McpServerInfo;
use crate::worker_adapters::{ModelWithProvider, TurnContext, WorkerAdapters};

// =============================================================================
// GrpcWorkerAdapters Implementation
// =============================================================================

/// gRPC-backed worker adapters for external workers
#[derive(Clone)]
pub struct GrpcWorkerAdapters {
    client: GrpcClient,
}

impl GrpcWorkerAdapters {
    /// Create new gRPC adapters by connecting to control-plane
    pub async fn connect(grpc_address: &str) -> Result<Self> {
        let client = GrpcClient::connect(grpc_address).await?;
        Ok(Self { client })
    }

    /// Create from an existing GrpcClient
    pub fn from_client(client: GrpcClient) -> Self {
        Self { client }
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
        store.get_agent(AgentId::from_uuid(agent_id)).await
    }

    // =========================================================================
    // Session Operations
    // =========================================================================

    async fn get_session(&self, org_id: i64, session_id: Uuid) -> Result<Option<Session>> {
        let store = GrpcSessionStore::new(self.client.clone(), org_id);
        everruns_core::traits::SessionStore::get_session(&store, SessionId::from_uuid(session_id))
            .await
    }

    async fn set_session_status(
        &self,
        org_id: i64,
        session_id: Uuid,
        status: &str,
    ) -> Result<Session> {
        self.client
            .set_session_status(org_id, SessionId::from_uuid(session_id), status)
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

    async fn get_model_with_provider(
        &self,
        org_id: i64,
        model_id: Uuid,
    ) -> Result<Option<ModelWithProvider>> {
        let store = GrpcLlmProviderStore::new(self.client.clone(), org_id);
        let result = everruns_core::traits::LlmProviderStore::get_model_with_provider(
            &store,
            ModelId::from_uuid(model_id),
        )
        .await?;
        Ok(result.map(|m| ModelWithProvider {
            model: m.model,
            provider_type: m.provider_type,
            api_key: m.api_key,
            base_url: m.base_url,
            provider_settings: m.provider_settings.clone(),
        }))
    }

    async fn get_default_model(&self, org_id: i64) -> Result<Option<ModelWithProvider>> {
        let store = GrpcLlmProviderStore::new(self.client.clone(), org_id);
        let result = everruns_core::traits::LlmProviderStore::get_default_model(&store).await?;
        Ok(result.map(|m| ModelWithProvider {
            model: m.model,
            provider_type: m.provider_type,
            api_key: m.api_key,
            base_url: m.base_url,
            provider_settings: m.provider_settings.clone(),
        }))
    }

    // =========================================================================
    // Image Resolution Operations
    // =========================================================================

    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        let resolver = GrpcImageResolver::new(self.client.clone());
        everruns_core::traits::ImageResolver::resolve_image(&resolver, image_id).await
    }

    async fn resolve_images_batch(
        &self,
        image_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, ResolvedImage>> {
        let resolver = GrpcImageResolver::new(self.client.clone());
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
        everruns_core::traits::SessionFileStore::read_file(
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
        everruns_core::traits::SessionFileStore::write_file(
            &store,
            SessionId::from_uuid(session_id),
            path,
            content,
            encoding,
        )
        .await
    }

    async fn delete_file(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileStore::delete_file(
            &store,
            SessionId::from_uuid(session_id),
            path,
            recursive,
        )
        .await
    }

    async fn list_directory(&self, session_id: Uuid, path: &str) -> Result<Vec<FileInfo>> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileStore::list_directory(
            &store,
            SessionId::from_uuid(session_id),
            path,
        )
        .await
    }

    async fn stat_file(&self, session_id: Uuid, path: &str) -> Result<Option<FileStat>> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileStore::stat_file(
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
        everruns_core::traits::SessionFileStore::grep_files(
            &store,
            SessionId::from_uuid(session_id),
            pattern,
            path_pattern,
        )
        .await
    }

    async fn create_directory(&self, session_id: Uuid, path: &str) -> Result<FileInfo> {
        let store = GrpcSessionFileStore::new(self.client.clone());
        everruns_core::traits::SessionFileStore::create_directory(
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
        server_prefix: &str,
    ) -> Result<McpServerInfo> {
        self.client
            .get_mcp_server_by_prefix(org_id, server_prefix)
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
            model: ctx.model.map(|m| ModelWithProvider {
                model: m.model,
                provider_type: m.provider_type,
                api_key: m.api_key,
                base_url: m.base_url,
                provider_settings: m.provider_settings.clone(),
            }),
            mcp_tool_definitions: ctx.mcp_tool_definitions,
        })
    }

    // =========================================================================
    // Factory Methods
    // =========================================================================

    fn capability_registry(&self) -> CapabilityRegistry {
        CapabilityRegistry::with_builtins()
    }

    fn driver_registry(&self) -> DriverRegistry {
        create_driver_registry()
    }

    async fn build_tool_registry(&self, agent_id: Uuid) -> Result<ToolRegistry> {
        let mut registry = ToolRegistry::with_defaults();

        // Get agent to access capabilities
        // Note: We use DEFAULT_ORG_ID here since tool building doesn't need org context
        // The agent lookup will work because agent_id is globally unique
        if let Ok(Some(agent)) = self
            .get_agent(everruns_core::DEFAULT_ORG_ID, agent_id)
            .await
        {
            let builtin_cap_ids: Vec<String> = agent
                .capabilities
                .iter()
                .map(|c| c.capability_id().to_string())
                .filter(|id| !is_mcp_capability(id))
                .collect();

            if !builtin_cap_ids.is_empty() {
                let cap_registry = self.capability_registry();
                let collected = collect_capabilities(&builtin_cap_ids, &cap_registry);
                for tool in collected.tools {
                    registry.register_boxed(tool);
                }
                tracing::debug!(
                    capability_count = builtin_cap_ids.len(),
                    tool_count = collected.tool_definitions.len(),
                    "Registered capability tools"
                );
            }
        }

        Ok(registry)
    }
}
