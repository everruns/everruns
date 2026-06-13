// Public backend contract for the embedded runtime.
// Decision: runtime extends core's read-only traits with the minimal write
// operations needed for seeding and message persistence. Trait upcasting +
// blanket `Arc<T>: T` impls in core let the runtime forward to atoms without
// shim wrappers.

use crate::in_memory::{InMemorySessionStorageStore, InMemorySessionStore};
use async_trait::async_trait;
use everruns_core::agent::Agent;
use everruns_core::error::Result;
use everruns_core::events::Event;
use everruns_core::harness::Harness;
use everruns_core::in_memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryProviderStore,
    InMemoryMessageRetriever,
};
use everruns_core::message::Message;
use everruns_core::message_retriever::{InputMessage, MessageRetriever};
use everruns_core::session::Session;
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, ProviderStore, ResolvedModel, SessionMutator,
    SessionStorageStore, SessionStore, UserConnectionResolver,
};
use everruns_core::typed_id::SessionId;
use std::sync::Arc;

/// Agent store contract for runtime seeding and lookup.
#[async_trait]
pub trait RuntimeAgentStore: AgentStore + Send + Sync {
    /// Insert or replace an agent definition.
    async fn add_agent(&self, agent: Agent) -> Result<()>;
}

/// Harness store contract for runtime seeding and lookup.
#[async_trait]
pub trait RuntimeHarnessStore: HarnessStore + Send + Sync {
    /// Insert or replace a harness definition.
    async fn add_harness(&self, harness: Harness) -> Result<()>;
}

/// Session store contract for runtime seeding, lookup, and mutation.
#[async_trait]
pub trait RuntimeSessionStore: SessionStore + SessionMutator + Send + Sync {
    /// Insert or replace a session definition.
    async fn add_session(&self, session: Session) -> Result<()>;
}

/// Message store contract for runtime persistence and lookup.
#[async_trait]
pub trait RuntimeMessageStore: MessageRetriever + Send + Sync {
    /// Store a new input message and return the generated message record.
    async fn add_input_message(
        &self,
        session_id: SessionId,
        input: InputMessage,
    ) -> Result<Message>;

    /// Persist an existing message record.
    async fn store_message(&self, session_id: SessionId, message: Message) -> Result<()>;
}

/// Provider store contract for runtime lookup and default-model configuration.
#[async_trait]
pub trait RuntimeProviderStore: ProviderStore + Send + Sync {
    /// Set the runtime default model.
    async fn set_default_model(&self, model: ResolvedModel) -> Result<()>;
}

/// Event sink that supports emission and optional collection.
///
/// Every `EventBus` is an `EventEmitter`. Embedders that want to inspect
/// emitted events override `collected_events`; production hosts inherit the
/// no-op default.
#[async_trait]
pub trait EventBus: EventEmitter {
    /// Return all collected events. Defaults to an empty `Vec` for buses
    /// that do not retain events.
    async fn collected_events(&self) -> Vec<Event> {
        Vec::new()
    }
}

#[async_trait]
impl<T: EventBus + ?Sized> EventBus for Arc<T> {
    async fn collected_events(&self) -> Vec<Event> {
        (**self).collected_events().await
    }
}

/// Non-filesystem backend bundle supplied to the embedded runtime.
///
/// Use this when you want the public runtime orchestration but your own store
/// implementations instead of the built-in in-memory ones. Session filesystem
/// selection is always resolved from `PlatformDefinition`.
#[derive(Clone)]
pub struct RuntimeBackends {
    /// Harness definitions available to the runtime.
    pub harness_store: Arc<dyn RuntimeHarnessStore>,
    /// Agent definitions available to the runtime.
    pub agent_store: Arc<dyn RuntimeAgentStore>,
    /// Session records and mutable session metadata.
    pub session_store: Arc<dyn RuntimeSessionStore>,
    /// Conversation message persistence and retrieval.
    pub message_store: Arc<dyn RuntimeMessageStore>,
    /// Model/provider resolution and default-model configuration.
    pub provider_store: Arc<dyn RuntimeProviderStore>,
    /// Event sink (emit + optional collection).
    pub event_bus: Arc<dyn EventBus>,
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
}

impl RuntimeBackends {
    /// Backend bundle with in-memory implementations for every store.
    ///
    /// Suitable for tests, examples, and the default runtime configuration.
    /// Use the chainable `with_*` setters to override individual stores.
    pub fn in_memory() -> Self {
        let event_bus = Arc::new(InMemoryEventEmitter::new());
        Self {
            harness_store: Arc::new(InMemoryHarnessStore::new()),
            agent_store: Arc::new(InMemoryAgentStore::new()),
            session_store: Arc::new(InMemorySessionStore::new()),
            message_store: Arc::new(InMemoryMessageRetriever::new()),
            provider_store: Arc::new(InMemoryProviderStore::new()),
            event_bus,
            storage_store: Arc::new(InMemorySessionStorageStore::new()),
            connection_resolver: None,
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

    pub fn with_message_store(mut self, store: Arc<dyn RuntimeMessageStore>) -> Self {
        self.message_store = store;
        self
    }

    pub fn with_provider_store(mut self, store: Arc<dyn RuntimeProviderStore>) -> Self {
        self.provider_store = store;
        self
    }

    pub fn with_event_bus(mut self, bus: Arc<dyn EventBus>) -> Self {
        self.event_bus = bus;
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
}

#[async_trait]
impl RuntimeAgentStore for InMemoryAgentStore {
    async fn add_agent(&self, agent: Agent) -> Result<()> {
        InMemoryAgentStore::add_agent(self, agent).await;
        Ok(())
    }
}

#[async_trait]
impl RuntimeHarnessStore for InMemoryHarnessStore {
    async fn add_harness(&self, harness: Harness) -> Result<()> {
        InMemoryHarnessStore::add_harness(self, harness).await;
        Ok(())
    }
}

#[async_trait]
impl RuntimeSessionStore for InMemorySessionStore {
    async fn add_session(&self, session: Session) -> Result<()> {
        InMemorySessionStore::add_session(self, session).await;
        Ok(())
    }
}

#[async_trait]
impl RuntimeMessageStore for InMemoryMessageRetriever {
    async fn add_input_message(
        &self,
        session_id: SessionId,
        input: InputMessage,
    ) -> Result<Message> {
        self.add(session_id, input).await
    }

    async fn store_message(&self, session_id: SessionId, message: Message) -> Result<()> {
        self.store(session_id, message).await
    }
}

#[async_trait]
impl RuntimeProviderStore for InMemoryProviderStore {
    async fn set_default_model(&self, model: ResolvedModel) -> Result<()> {
        InMemoryProviderStore::set_default_model(self, model).await;
        Ok(())
    }
}

#[async_trait]
impl EventBus for InMemoryEventEmitter {
    async fn collected_events(&self) -> Vec<Event> {
        InMemoryEventEmitter::events(self).await
    }
}
