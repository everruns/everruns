// Runtime-host adapter bridge for durable/server-backed workers.
// Decision: everruns-worker exposes first-party adapters from WorkerAdapters to
// the public everruns-runtime host contract.

use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, ImageResolver, LlmProviderStore, SessionFileStore,
    SessionMutator, SessionStore,
};
use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::{Agent, CapabilityRegistry, DriverRegistry, Harness, Session, SessionStatus};
use everruns_runtime::{RuntimeHostAdapter, RuntimeHostTurnContext};
use std::sync::Arc;

use crate::worker_adapters::{
    AdapterAgentStore, AdapterEventEmitter, AdapterHarnessStore, AdapterImageResolver,
    AdapterLlmProviderStore, AdapterMessageRetriever, AdapterSessionFileStore,
    AdapterSessionMutator, AdapterSessionStore, WorkerAdapters,
};

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
}

impl<A: WorkerAdapters> WorkerRuntimeHost<A> {
    pub fn new(adapters: A) -> Self {
        Self { adapters }
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

    async fn build_tool_registry_for_agent(
        &self,
        org_id: i64,
        agent_id: AgentId,
    ) -> Result<everruns_core::ToolRegistry> {
        self.adapters
            .build_tool_registry(org_id, agent_id.uuid())
            .await
    }

    async fn build_tool_registry_for_harness(
        &self,
        org_id: i64,
        harness_id: HarnessId,
    ) -> Result<everruns_core::ToolRegistry> {
        self.adapters
            .build_tool_registry_for_harness(org_id, harness_id.uuid())
            .await
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

    fn provider_store(&self, org_id: i64) -> Arc<dyn LlmProviderStore> {
        Arc::new(AdapterLlmProviderStore::new(self.adapters.clone(), org_id))
    }

    fn message_store(&self) -> Arc<dyn everruns_core::MessageRetriever> {
        Arc::new(AdapterMessageRetriever::new(self.adapters.clone()))
    }

    fn event_emitter(&self) -> Arc<dyn EventEmitter> {
        Arc::new(AdapterEventEmitter::new(self.adapters.clone()))
    }

    fn file_store(&self) -> Arc<dyn SessionFileStore> {
        Arc::new(AdapterSessionFileStore::new(self.adapters.clone()))
    }

    fn image_resolver(&self, org_id: i64) -> Option<Arc<dyn ImageResolver>> {
        Some(Arc::new(AdapterImageResolver::new(
            self.adapters.clone(),
            org_id,
        )))
    }

    fn storage_store(&self) -> Option<Arc<dyn everruns_core::traits::SessionStorageStore>> {
        Some(self.adapters.storage_store())
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

    fn schedule_store(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn everruns_core::traits::SessionScheduleStore>> {
        Some(self.adapters.schedule_store(org_id))
    }

    fn platform_store(
        &self,
        org_id: i64,
    ) -> Option<Arc<dyn everruns_core::platform_store::PlatformStore>> {
        Some(self.adapters.platform_store(org_id))
    }

    fn budget_checker(
        &self,
        org_id: i64,
        agent_id: Option<AgentId>,
    ) -> Option<Arc<dyn everruns_core::traits::BudgetChecker>> {
        self.adapters.budget_checker(org_id, agent_id)
    }
}
