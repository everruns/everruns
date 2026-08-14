// Runtime-host adapter bridge for durable/server-backed workers.
// Decision: everruns-worker exposes first-party adapters from WorkerAdapters to
// the neutral everruns-host execution contract.

use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::typed_id::{AgentId, SessionId};
use everruns_core::{
    CapabilityRegistry, DriverRegistry, EgressService, ResolvedExecutionSnapshot,
    SessionExecutionState, UtilityLlmService,
};
use everruns_core::{
    connection_services::ProviderCredentialStore, delegation_services::SessionCreationAuthority,
    event_emitter::EventEmitter, execution_loading::AgentStore, execution_loading::HarnessStore,
    execution_loading::SessionStore, image_services::ImageArtifactStore,
    image_services::ImageResolver, provider_resolution::ProviderStore,
    session_files::SessionFileSystem, tool_execution::PaymentAuthority,
};
use everruns_host::{ResolvedTurnInputs, RuntimeHostAdapter};
use everruns_mcp::{
    McpClient, McpConnection, McpConnectionResolver, McpEndpoint, McpExecutor, NoAuthProvider,
};
use everruns_platform::SessionMutator;
use std::sync::Arc;
use uuid::Uuid;

use crate::worker_adapters::{
    AdapterAgentStore, AdapterEventEmitter, AdapterHarnessStore, AdapterImageResolver,
    AdapterMessageRetriever, AdapterProviderStore, AdapterSessionFileStore, AdapterSessionMutator,
    AdapterSessionStore, WorkerAdapters,
};

/// Resolves an `mcp_*` server prefix to a connection by asking the control
/// plane over gRPC (`get_mcp_server_by_prefix`). The control plane returns a
/// fully resolved descriptor.
///
/// Credential handling:
/// - **API-key** servers carry the decrypted key, which we bake in as a
///   `Bearer` Authorization header.
/// - **OAuth** servers resolve the session's connection token for
///   `oauth_provider_id` via the host's `UserConnectionResolver` (same lookup
///   scoped tool discovery uses) and bake it in as a `Bearer` header. When no
///   token is connected, the connection is marked `pending_oauth_provider` so
///   the executor returns a `connection_required` tool result instead of a
///   raw 401.
/// - The client uses `NoAuthProvider`, so auth is always expressed via
///   `headers`.
struct WorkerMcpResolver<A: WorkerAdapters> {
    adapters: A,
    org_id: i64,
    session_id: Uuid,
}

#[async_trait]
impl<A: WorkerAdapters> McpConnectionResolver for WorkerMcpResolver<A> {
    async fn resolve(&self, server_prefix: &str) -> anyhow::Result<Option<McpConnection>> {
        let info = self
            .adapters
            .get_mcp_server_by_prefix(self.org_id, Some(self.session_id), server_prefix)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

        let mut headers = info.headers;
        let has_authorization = |headers: &std::collections::HashMap<String, String>| {
            headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("authorization"))
        };
        if let Some(api_key) = info.api_key
            && !has_authorization(&headers)
        {
            headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));
        }

        let mut pending_oauth_provider = None;
        if info.auth_mode == everruns_core::McpServerAuthMode::OAuth
            && !has_authorization(&headers)
            && let Some(provider) = info.oauth_provider_id.as_deref()
        {
            match self
                .adapters
                .connection_resolver()
                .get_connection_token(self.session_id.into(), provider)
                .await
            {
                Ok(Some(token)) => {
                    headers.insert("Authorization".to_string(), format!("Bearer {token}"));
                }
                Ok(None) => pending_oauth_provider = Some(provider.to_string()),
                Err(error) => {
                    tracing::warn!(
                        server = %info.name,
                        provider,
                        %error,
                        "failed to resolve MCP OAuth connection token"
                    );
                    pending_oauth_provider = Some(provider.to_string());
                }
            }
        }

        Ok(Some(McpConnection {
            name: info.name,
            endpoint: McpEndpoint::Http {
                url: info.url,
                headers,
            },
            auth_mode: info.auth_mode,
            protocol_mode: info.protocol_mode,
            oauth_provider_id: info.oauth_provider_id,
            pending_oauth_provider,
            secret_bindings: info.secret_bindings,
        }))
    }
}

/// First-party adapter from worker backends into `everruns-host` execution.
///
/// This is the bridge that lets durable workers execute the shared runtime
/// host phases without depending on in-process-only stores.
///
/// ```ignore
/// use everruns_host::execute_reason_activity;
/// use everruns_worker::{GrpcWorkerAdapters, WorkerRuntimeHost};
///
/// let adapters = GrpcWorkerAdapters::connect("127.0.0.1:9001").await?;
/// let host = WorkerRuntimeHost::new(adapters);
/// let result = execute_reason_activity(&host, org_id, reason_input).await?;
/// # Ok::<(), everruns_core::AgentLoopError>(())
/// ```
#[derive(Clone)]
pub struct WorkerRuntimeHost<A: WorkerAdapters> {
    adapters: A,
    event_metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

impl<A: WorkerAdapters> WorkerRuntimeHost<A> {
    pub fn new(adapters: A) -> Self {
        Self {
            adapters,
            event_metadata: None,
        }
    }

    pub fn with_event_metadata(
        adapters: A,
        metadata: Option<serde_json::Map<String, serde_json::Value>>,
    ) -> Self {
        Self {
            adapters,
            event_metadata: metadata,
        }
    }
}

#[async_trait]
impl<A: WorkerAdapters> RuntimeHostAdapter for WorkerRuntimeHost<A> {
    async fn set_session_status(
        &self,
        org_id: i64,
        session_id: SessionId,
        status: SessionExecutionState,
    ) -> Result<()> {
        // Status mutation is an acknowledged effect end to end (EVE-882):
        // neither the adapter nor the host contract exposes a session record.
        self.adapters
            .set_session_status(org_id, session_id.uuid(), &status.to_string())
            .await?;
        Ok(())
    }

    async fn load_resolved_turn(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<ResolvedTurnInputs> {
        // The batched control-plane call still ships stored records
        // (compatibility adapter, see `WorkerAdapters::load_turn_context`).
        // They are projected into the canonical resolved execution snapshot
        // here, at the platform boundary, so host execution never sees them
        // (EVE-872). The control plane returns the harness pre-merged, so the
        // effective definition folds identically to the in-process runtime.
        let context = self
            .adapters
            .load_turn_context(org_id, session_id.uuid())
            .await?;
        // Loading seam (EVE-877/EVE-881): project the stored records into the
        // portable execution definitions; archived/deleted harnesses and
        // agents fail here, before the snapshot is built.
        let harness_definition = self
            .adapters
            .get_harness(org_id, context.session.harness_id.uuid())
            .await?
            .ok_or_else(|| {
                everruns_core::AgentLoopError::harness_not_found(context.session.harness_id)
            })?
            .execution_definition()?;
        let agent_definition = context
            .agent
            .as_ref()
            .map(|agent| agent.execution_definition())
            .transpose()?;
        let snapshot = ResolvedExecutionSnapshot::project(
            &harness_definition,
            agent_definition.as_ref(),
            &context.session,
        )?;
        Ok(ResolvedTurnInputs {
            snapshot,
            messages: context.messages,
            mcp_tool_definitions: context.mcp_tool_definitions,
        })
    }

    fn capability_registry(&self) -> CapabilityRegistry {
        self.adapters.capability_registry()
    }

    fn driver_registry(&self) -> DriverRegistry {
        self.adapters.driver_registry()
    }

    fn harness_store(&self, org_id: i64) -> Arc<dyn HarnessStore> {
        Arc::new(AdapterHarnessStore::new(self.adapters.clone(), org_id))
    }

    fn agent_store(&self, org_id: i64) -> Arc<dyn AgentStore> {
        Arc::new(AdapterAgentStore::new(self.adapters.clone(), org_id))
    }

    fn session_store(&self, org_id: i64) -> Arc<dyn SessionStore> {
        Arc::new(AdapterSessionStore::new(self.adapters.clone(), org_id))
    }

    fn session_mutator(&self, org_id: i64) -> Arc<dyn SessionMutator> {
        Arc::new(AdapterSessionMutator::new(self.adapters.clone(), org_id))
    }

    fn provider_store(&self, org_id: i64) -> Arc<dyn ProviderStore> {
        Arc::new(AdapterProviderStore::new(self.adapters.clone(), org_id))
    }

    fn message_store(&self) -> Arc<dyn everruns_core::MessageRetriever> {
        Arc::new(AdapterMessageRetriever::new(self.adapters.clone()))
    }

    fn compaction_checkpoint_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::CompactionCheckpointStore>> {
        self.adapters.compaction_checkpoint_store()
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        Arc::new(
            AdapterEventEmitter::new(self.adapters.clone())
                .with_event_metadata(self.event_metadata.clone()),
        )
    }

    fn file_store(&self) -> Arc<dyn SessionFileSystem> {
        Arc::new(AdapterSessionFileStore::new(self.adapters.clone()))
    }

    fn image_resolver(&self, org_id: i64) -> Option<Arc<dyn ImageResolver>> {
        Some(Arc::new(AdapterImageResolver::new(
            self.adapters.clone(),
            org_id,
        )))
    }

    fn image_artifact_store(&self, org_id: i64) -> Option<Arc<dyn ImageArtifactStore>> {
        Some(self.adapters.image_artifact_store(org_id))
    }

    fn provider_credential_store(&self, org_id: i64) -> Option<Arc<dyn ProviderCredentialStore>> {
        Some(self.adapters.provider_credential_store(org_id))
    }

    fn utility_llm_service(&self) -> Option<Arc<dyn UtilityLlmService>> {
        self.adapters.utility_llm_service()
    }

    fn egress_service(&self) -> Option<Arc<dyn EgressService>> {
        self.adapters.egress_service()
    }

    fn storage_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_services::SessionStorageStore>> {
        Some(self.adapters.storage_store())
    }

    fn knowledge_store(&self) -> Option<Arc<dyn everruns_platform::KnowledgeStore>> {
        self.adapters.knowledge_store()
    }

    fn connection_resolver(
        &self,
    ) -> Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>> {
        Some(self.adapters.connection_resolver())
    }

    fn sqldb_store(
        &self,
    ) -> Option<std::sync::Arc<dyn everruns_platform::session_sqldb::SessionSqlDbStore>> {
        Some(self.adapters.sqldb_store())
    }

    fn leased_resource_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_services::LeasedResourceStore>> {
        Some(self.adapters.leased_resource_store())
    }

    fn session_resource_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_services::SessionResourceRegistry>> {
        self.adapters.session_resource_registry()
    }

    fn session_task_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::session_task::SessionTaskRegistry>> {
        self.adapters.session_task_registry()
    }

    fn schedule_store(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn everruns_core::session_services::SessionScheduleStore>> {
        Some(self.adapters.schedule_store(org_id))
    }

    fn platform_store(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn everruns_platform::PlatformStore>> {
        Some(self.adapters.platform_store(org_id, session_id))
    }

    fn knowledge_index_search(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn everruns_platform::vector_store::KnowledgeIndexSearch>> {
        self.adapters.knowledge_index_search(org_id)
    }

    fn budget_checker(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn everruns_core::tool_execution::BudgetChecker>> {
        self.adapters.budget_checker(org_id, agent_id)
    }

    fn payment_authority(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn PaymentAuthority>> {
        self.adapters.payment_authority(org_id, agent_id)
    }

    fn session_creation_authority(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn SessionCreationAuthority>> {
        self.adapters.session_creation_authority(org_id, session_id)
    }

    fn outbound_tool_rate_limiter(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn everruns_core::tool_execution::OutboundToolRateLimiter>> {
        self.adapters.outbound_tool_rate_limiter(org_id)
    }

    fn durable_tool_result_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::durability::DurableToolResultStore>> {
        self.adapters.durable_tool_result_store()
    }

    fn subagent_spawn_store(
        &self,
    ) -> Option<Arc<dyn everruns_core::delegation_services::SubagentSpawnStore>> {
        self.adapters.subagent_spawn_store()
    }

    fn stream_heartbeater(&self) -> Option<Arc<dyn everruns_core::durability::StreamHeartbeater>> {
        self.adapters.stream_heartbeater()
    }

    fn provider_stall_timeout(&self) -> Option<std::time::Duration> {
        self.adapters.provider_stall_timeout()
    }

    /// Execute `mcp_*` tool calls by resolving server connections over gRPC and
    /// calling them through the shared MCP client over the platform egress
    /// boundary (SSRF-guarded). Returns `None` when no egress service is
    /// available, in which case MCP tools are not registered for execution.
    async fn mcp_executor(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn everruns_core::McpToolInvoker>> {
        let egress = self.adapters.egress_service()?;
        let client = Arc::new(McpClient::new(egress, Arc::new(NoAuthProvider)));
        let resolver = Arc::new(WorkerMcpResolver {
            adapters: self.adapters.clone(),
            org_id,
            session_id: session_id.uuid(),
        });
        Some(Arc::new(McpExecutor::new(client, resolver)))
    }
}
