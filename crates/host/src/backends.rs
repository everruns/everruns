// Public backend contract for the embedded host.
// Seeding stores remain writable host configuration. Conversation history is
// different: canonical events are the only write path and EventHistory is the
// rebuildable read projection.

use crate::events::{EventLog, EventSink, InMemoryEventLog, NoopEventSink};
use crate::in_memory::{InMemorySessionStorageStore, InMemorySessionStore};
use async_trait::async_trait;
use everruns_core::agent_definition::AgentDefinition;
use everruns_core::error::Result;
use everruns_core::harness_definition::HarnessDefinition;
use everruns_core::in_memory::{InMemoryAgentStore, InMemoryHarnessStore, InMemoryProviderStore};
use everruns_core::session::ExecutionSession;
use everruns_core::session_task::SessionTaskRegistry;
use everruns_core::traits::{
    AgentStore, HarnessStore, ProviderStore, ResolvedModel, SessionMutator, SessionScheduleStore,
    SessionStorageStore, SessionStore, UserConnectionResolver,
};
use everruns_core::typed_id::{HarnessId, SessionId};
use everruns_platform::PlatformStore;
use std::sync::Arc;

/// Factory producing a per-org [`SessionScheduleStore`]. Embedders that have a
/// single global store can ignore the `org_id` argument and return the same
/// `Arc` every time.
pub type ScheduleStoreFactory = Arc<dyn Fn(i64) -> Arc<dyn SessionScheduleStore> + Send + Sync>;

/// Factory producing a per-(org, session) [`PlatformStore`]. The session id is
/// supplied because platform stores are scoped to the calling session for
/// subagent spawning and platform-management tools.
pub type PlatformStoreFactory = Arc<dyn Fn(i64, SessionId) -> Arc<dyn PlatformStore> + Send + Sync>;

/// Agent store contract for runtime seeding and lookup.
///
/// Seeds portable [`AgentDefinition`] values (EVE-877): the embedded host
/// carries no stored Agent persistence records.
#[async_trait]
pub trait RuntimeAgentStore: AgentStore + Send + Sync {
    /// Insert or replace an agent definition.
    async fn add_agent(&self, agent: AgentDefinition) -> Result<()>;
}

/// Harness store contract for runtime seeding and lookup.
///
/// Seeds portable [`HarnessDefinition`] values under an embedder-chosen id
/// (EVE-881): the embedded host carries no stored Harness persistence records.
#[async_trait]
pub trait RuntimeHarnessStore: HarnessStore + Send + Sync {
    /// Insert or replace a harness definition under the given id.
    async fn add_harness(&self, harness_id: HarnessId, harness: HarnessDefinition) -> Result<()>;
}

/// Session store contract for runtime seeding, lookup, and mutation.
///
/// Seeds portable [`ExecutionSession`] values (EVE-882): the embedded host
/// carries no stored Session persistence records.
#[async_trait]
pub trait RuntimeSessionStore: SessionStore + SessionMutator + Send + Sync {
    /// Insert or replace a session's portable execution view.
    async fn add_session(&self, session: ExecutionSession) -> Result<()>;
}

/// Provider store contract for runtime lookup and default-model configuration.
#[async_trait]
pub trait RuntimeProviderStore: ProviderStore + Send + Sync {
    /// Set the runtime default model.
    async fn set_default_model(&self, model: ResolvedModel) -> Result<()>;
}

/// Non-filesystem backend bundle supplied to an execution host.
///
/// Use this when you want the public runtime orchestration but your own store
/// implementations instead of the built-in in-memory ones. Session filesystem
/// selection is always resolved from `HostComposition`.
#[derive(Clone)]
pub struct HostBackends {
    /// Harness definitions available to the runtime.
    pub harness_store: Arc<dyn RuntimeHarnessStore>,
    /// Agent definitions available to the runtime.
    pub agent_store: Arc<dyn RuntimeAgentStore>,
    /// Session records and mutable session metadata.
    pub session_store: Arc<dyn RuntimeSessionStore>,
    /// Coherent canonical event log. This is the sole conversation write path.
    pub event_log: Arc<dyn EventLog>,
    /// Durable replacement context used to reconstruct compacted model input.
    pub compaction_checkpoint_store: Arc<dyn everruns_core::CompactionCheckpointStore>,
    /// Model/provider resolution and default-model configuration.
    pub provider_store: Arc<dyn RuntimeProviderStore>,
    /// Optional, non-blocking live observation sink notified after commit.
    pub event_sink: Arc<dyn EventSink>,
    /// Session key/value + secret storage backend.
    pub storage_store: Arc<dyn SessionStorageStore>,
    /// Optional resolver for user connection tokens (e.g. GitHub, Daytona).
    ///
    /// When set, the runtime exposes it through `ToolContext.connection_resolver`
    /// so connection-aware capabilities can resolve tokens lazily at tool time.
    /// `None` (the default) leaves the resolver unset, matching prior behavior.
    /// There is no in-memory default because a connection resolver implies a
    /// real credential source the embedder must supply.
    pub connection_resolver: Option<Arc<dyn UserConnectionResolver>>,
    /// Optional session-task registry injected into the act path so background
    /// tools, subagents, and monitors persist their lifecycle. `None` (the
    /// default) leaves `RuntimeHostAdapter::session_task_registry` returning
    /// `None`, matching prior in-memory behavior.
    pub session_task_registry: Option<Arc<dyn SessionTaskRegistry>>,
    /// Optional per-org schedule store factory. `None` (the default) leaves
    /// `RuntimeHostAdapter::schedule_store` returning `None`.
    pub schedule_store_factory: Option<ScheduleStoreFactory>,
    /// Optional per-(org, session) platform store factory. `None` (the
    /// default) leaves `RuntimeHostAdapter::platform_store` returning `None`.
    pub platform_store_factory: Option<PlatformStoreFactory>,
}

impl HostBackends {
    /// Backend bundle with in-memory implementations for every store.
    ///
    /// Suitable for tests, examples, and the default runtime configuration.
    /// Use the chainable `with_*` setters to override individual stores.
    pub fn in_memory() -> Self {
        Self {
            harness_store: Arc::new(InMemoryHarnessStore::new()),
            agent_store: Arc::new(InMemoryAgentStore::new()),
            session_store: Arc::new(InMemorySessionStore::new()),
            event_log: Arc::new(InMemoryEventLog::new()),
            compaction_checkpoint_store: Arc::new(
                everruns_core::InMemoryCompactionCheckpointStore::default(),
            ),
            provider_store: Arc::new(InMemoryProviderStore::new()),
            event_sink: Arc::new(NoopEventSink),
            storage_store: Arc::new(InMemorySessionStorageStore::new()),
            connection_resolver: None,
            session_task_registry: None,
            schedule_store_factory: None,
            platform_store_factory: None,
        }
    }

    pub fn with_harness_store(mut self, store: Arc<dyn RuntimeHarnessStore>) -> Self {
        self.harness_store = store;
        self
    }

    pub fn with_agent_store(mut self, store: Arc<dyn RuntimeAgentStore>) -> Self {
        self.agent_store = store;
        self
    }

    pub fn with_session_store(mut self, store: Arc<dyn RuntimeSessionStore>) -> Self {
        self.session_store = store;
        self
    }

    /// Replace the coherent canonical event log.
    pub fn with_event_log(mut self, log: Arc<dyn EventLog>) -> Self {
        self.event_log = log;
        self
    }

    pub fn with_compaction_checkpoint_store(
        mut self,
        store: Arc<dyn everruns_core::CompactionCheckpointStore>,
    ) -> Self {
        self.compaction_checkpoint_store = store;
        self
    }

    pub fn with_provider_store(mut self, store: Arc<dyn RuntimeProviderStore>) -> Self {
        self.provider_store = store;
        self
    }

    /// Install a non-blocking post-commit observation sink.
    pub fn with_event_sink(mut self, sink: Arc<dyn EventSink>) -> Self {
        self.event_sink = sink;
        self
    }

    pub fn with_storage_store(mut self, store: Arc<dyn SessionStorageStore>) -> Self {
        self.storage_store = store;
        self
    }

    /// Supply a resolver for user connection tokens (e.g. GitHub, Daytona).
    ///
    /// The runtime forwards it into `ToolContext` so connection-aware
    /// capabilities resolve tokens lazily at tool execution time.
    pub fn with_connection_resolver(mut self, resolver: Arc<dyn UserConnectionResolver>) -> Self {
        self.connection_resolver = Some(resolver);
        self
    }

    /// Inject a session-task registry so background tools / subagents / monitors
    /// persist their lifecycle through the act path. Additive: leaving this unset
    /// keeps the prior behavior (the adapter returns `None`).
    pub fn with_session_task_registry(mut self, registry: Arc<dyn SessionTaskRegistry>) -> Self {
        self.session_task_registry = Some(registry);
        self
    }

    /// Inject a per-org schedule store factory. The closure is invoked with the
    /// internal org id each time the act path needs a schedule store.
    pub fn with_schedule_store_factory(mut self, factory: ScheduleStoreFactory) -> Self {
        self.schedule_store_factory = Some(factory);
        self
    }

    /// Inject a per-(org, session) platform store factory. The closure is
    /// invoked with the internal org id and the calling session id.
    pub fn with_platform_store_factory(mut self, factory: PlatformStoreFactory) -> Self {
        self.platform_store_factory = Some(factory);
        self
    }
}

#[async_trait]
impl RuntimeAgentStore for InMemoryAgentStore {
    async fn add_agent(&self, agent: AgentDefinition) -> Result<()> {
        InMemoryAgentStore::add_agent(self, agent).await;
        Ok(())
    }
}

#[async_trait]
impl RuntimeHarnessStore for InMemoryHarnessStore {
    async fn add_harness(&self, harness_id: HarnessId, harness: HarnessDefinition) -> Result<()> {
        InMemoryHarnessStore::add_harness(self, harness_id, harness).await;
        Ok(())
    }
}

#[async_trait]
impl RuntimeSessionStore for InMemorySessionStore {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        InMemorySessionStore::add_session(self, session).await;
        Ok(())
    }
}

#[async_trait]
impl RuntimeProviderStore for InMemoryProviderStore {
    async fn set_default_model(&self, model: ResolvedModel) -> Result<()> {
        InMemoryProviderStore::set_default_model(self, model).await;
        Ok(())
    }
}
