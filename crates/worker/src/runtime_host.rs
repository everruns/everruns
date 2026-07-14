// Runtime-host adapter bridge for durable/server-backed workers.
// Decision: everruns-worker exposes first-party adapters from WorkerAdapters to
// the public everruns-runtime host contract.

use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, ImageArtifactStore, ImageResolver, PaymentAuthority,
    ProviderCredentialStore, ProviderStore, SessionFileSystem, SessionMutator, SessionStore,
};
use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::{
    Agent, CapabilityRegistry, DriverRegistry, EgressService, Harness, Session, SessionStatus,
    UtilityLlmService,
};
use everruns_mcp::{
    McpClient, McpConnection, McpConnectionResolver, McpEndpoint, McpExecutor, NoAuthProvider,
};
use everruns_runtime::{RuntimeHostAdapter, RuntimeHostTurnContext};
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
/// - **OAuth** servers resolved via the *session-scoped* path arrive with the
///   token already injected into `headers` (control plane downgrades auth mode).
/// - We preserve `auth_mode`/`oauth_provider_id` on the connection so a future
///   worker `McpAuthProvider` can resolve tokens. Until then, OAuth servers
///   resolved via the non-scoped `resolve_by_prefix` fallback (which returns the
///   OAuth metadata but no token in `headers`) are not authenticated — see the
///   PR follow-up. The client uses `NoAuthProvider`, so auth must be expressed
///   via `headers`.
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
        if let Some(api_key) = info.api_key {
            let has_authorization = headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("authorization"));
            if !has_authorization {
                headers.insert("Authorization".to_string(), format!("Bearer {api_key}"));
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
        }))
    }
}

/// First-party adapter from worker backends into `everruns-runtime` host execution.
///
/// This is the bridge that lets durable workers execute the shared runtime
/// host phases without depending on in-process-only stores.
///
/// ```ignore
/// use everruns_runtime::execute_reason_activity;
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
    async fn get_agent(&self, org_id: i64, agent_id: AgentId) -> Result<Option<Agent>> {
        self.adapters.get_agent(org_id, agent_id.uuid()).await
    }

    async fn get_harness(&self, org_id: i64, harness_id: HarnessId) -> Result<Option<Harness>> {
        self.adapters.get_harness(org_id, harness_id.uuid()).await
    }

    async fn set_session_status(
        &self,
        org_id: i64,
        session_id: SessionId,
        status: SessionStatus,
    ) -> Result<Session> {
        self.adapters
            .set_session_status(org_id, session_id.uuid(), &status.to_string())
            .await
    }

    async fn load_turn_context(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Result<RuntimeHostTurnContext> {
        let context = self
            .adapters
            .load_turn_context(org_id, session_id.uuid())
            .await?;
        Ok(RuntimeHostTurnContext {
            agent: context.agent,
            session: context.session,
            messages: context.messages,
            model: context.model,
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

    fn storage_store(&self) -> Option<Arc<dyn everruns_core::traits::SessionStorageStore>> {
        Some(self.adapters.storage_store())
    }

    fn knowledge_store(&self) -> Option<Arc<dyn everruns_core::traits::KnowledgeStore>> {
        self.adapters.knowledge_store()
    }

    fn connection_resolver(
        &self,
    ) -> Option<Arc<dyn everruns_core::traits::UserConnectionResolver>> {
        Some(self.adapters.connection_resolver())
    }

    fn sqldb_store(&self) -> Option<everruns_core::traits::SessionSqlDbStoreRef> {
        Some(self.adapters.sqldb_store())
    }

    fn leased_resource_store(&self) -> Option<Arc<dyn everruns_core::traits::LeasedResourceStore>> {
        Some(self.adapters.leased_resource_store())
    }

    fn session_resource_registry(
        &self,
    ) -> Option<Arc<dyn everruns_core::traits::SessionResourceRegistry>> {
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
    ) -> Option<Arc<dyn everruns_core::traits::SessionScheduleStore>> {
        Some(self.adapters.schedule_store(org_id))
    }

    fn platform_store(
        &self,
        org_id: i64,
        session_id: SessionId,
    ) -> Option<Arc<dyn everruns_core::platform_store::PlatformStore>> {
        Some(self.adapters.platform_store(org_id, session_id))
    }

    fn knowledge_index_search(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn everruns_core::vector_store::KnowledgeIndexSearch>> {
        self.adapters.knowledge_index_search(org_id)
    }

    fn budget_checker(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn everruns_core::traits::BudgetChecker>> {
        self.adapters.budget_checker(org_id, agent_id)
    }

    fn payment_authority(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn PaymentAuthority>> {
        self.adapters.payment_authority(org_id, agent_id)
    }

    fn outbound_tool_rate_limiter(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn everruns_core::OutboundToolRateLimiter>> {
        self.adapters.outbound_tool_rate_limiter(org_id)
    }

    fn durable_tool_result_store(&self) -> Option<Arc<dyn everruns_core::DurableToolResultStore>> {
        self.adapters.durable_tool_result_store()
    }

    fn subagent_spawn_store(&self) -> Option<Arc<dyn everruns_core::SubagentSpawnStore>> {
        self.adapters.subagent_spawn_store()
    }

    fn stream_heartbeater(&self) -> Option<Arc<dyn everruns_core::StreamHeartbeater>> {
        self.adapters.stream_heartbeater()
    }

    fn provider_stall_timeout(&self) -> Option<std::time::Duration> {
        self.adapters.provider_stall_timeout()
    }

    /// Execute `mcp_*` tool calls by resolving server connections over gRPC and
    /// calling them through the shared MCP client over the platform egress
    /// boundary (SSRF-guarded). Returns `None` when no egress service is
    /// available, in which case MCP tools are not registered for execution.
    async fn mcp_executor(&self, org_id: i64, session_id: SessionId) -> Option<Arc<McpExecutor>> {
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
