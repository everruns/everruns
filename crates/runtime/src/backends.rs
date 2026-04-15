// Public backend contract for the embedded runtime.
// Decision: runtime extends core's read-only traits with the minimal write
// operations needed for seeding and message persistence.

use crate::in_memory::{InMemorySessionFileStore, InMemorySessionStore};
use async_trait::async_trait;
use everruns_core::agent::Agent;
use everruns_core::error::Result;
use everruns_core::events::{Event, EventRequest};
use everruns_core::harness::Harness;
use everruns_core::memory::{
    InMemoryAgentStore, InMemoryEventEmitter, InMemoryHarnessStore, InMemoryLlmProviderStore,
    InMemoryMessageRetriever,
};
use everruns_core::memory_store::MemoryStoreBackend;
use everruns_core::message::Message;
use everruns_core::message_retriever::{InputMessage, MessageRetriever};
use everruns_core::session::Session;
use everruns_core::session_file::{InitialFile, SessionFile};
use everruns_core::traits::{
    AgentStore, EventEmitter, HarnessStore, LlmProviderStore, ModelWithProvider, SessionFileStore,
    SessionMutator, SessionStorageStore, SessionStore,
};
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, ModelId, SessionId};
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

/// Filesystem contract for runtime seeding and lookup.
#[async_trait]
pub trait RuntimeFileStore: SessionFileStore + Send + Sync {
    /// Seed a starter file into a session workspace.
    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()>;
}

/// Provider store contract for runtime lookup and default-model configuration.
#[async_trait]
pub trait RuntimeProviderStore: LlmProviderStore + Send + Sync {
    /// Set the runtime default model.
    async fn set_default_model(&self, model: ModelWithProvider) -> Result<()>;
}

/// Optional event collector for embedders that want to inspect emitted events.
#[async_trait]
pub trait RuntimeEventCollector: Send + Sync {
    /// Return all collected events.
    async fn events(&self) -> Vec<Event>;
}

/// Backend bundle supplied to the embedded runtime.
///
/// Use this when you want the public runtime orchestration but your own store
/// implementations instead of the built-in in-memory ones.
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
    /// Event sink used during turn execution.
    pub event_emitter: Arc<dyn EventEmitter>,
    /// Optional collector for `InProcessRuntime::events()`.
    pub event_collector: Option<Arc<dyn RuntimeEventCollector>>,
    /// Session filesystem backend.
    pub file_store: Arc<dyn RuntimeFileStore>,
    /// Session key/value + secret storage backend.
    pub storage_store: Arc<dyn SessionStorageStore>,
    /// Persistent cross-session memory backend.
    pub memory_store: Arc<dyn MemoryStoreBackend>,
}

#[derive(Clone)]
pub(crate) struct DynAgentStore(pub Arc<dyn RuntimeAgentStore>);

#[async_trait]
impl AgentStore for DynAgentStore {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<Agent>> {
        self.0.get_agent(agent_id).await
    }
}

#[derive(Clone)]
pub(crate) struct DynHarnessStore(pub Arc<dyn RuntimeHarnessStore>);

#[async_trait]
impl HarnessStore for DynHarnessStore {
    async fn get_harness_chain(&self, harness_id: HarnessId) -> Result<Vec<Harness>> {
        self.0.get_harness_chain(harness_id).await
    }
}

#[derive(Clone)]
pub(crate) struct DynSessionStore(pub Arc<dyn RuntimeSessionStore>);

#[async_trait]
impl SessionStore for DynSessionStore {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>> {
        self.0.get_session(session_id).await
    }
}

#[async_trait]
impl SessionMutator for DynSessionStore {
    async fn update_session_title(&self, session_id: SessionId, title: String) -> Result<Session> {
        self.0.update_session_title(session_id, title).await
    }
}

#[derive(Clone)]
pub(crate) struct DynMessageStore(pub Arc<dyn RuntimeMessageStore>);

#[async_trait]
impl MessageRetriever for DynMessageStore {
    async fn get(&self, session_id: SessionId, message_id: MessageId) -> Result<Option<Message>> {
        self.0.get(session_id, message_id).await
    }

    async fn load(&self, session_id: SessionId) -> Result<Vec<Message>> {
        self.0.load(session_id).await
    }

    async fn load_filtered(
        &self,
        query: everruns_core::message_filter::MessageQuery,
    ) -> Result<Vec<Message>> {
        self.0.load_filtered(query).await
    }

    async fn load_page(
        &self,
        session_id: SessionId,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Message>> {
        self.0.load_page(session_id, offset, limit).await
    }

    async fn count(&self, session_id: SessionId) -> Result<usize> {
        self.0.count(session_id).await
    }
}

#[derive(Clone)]
pub(crate) struct DynFileStore(pub Arc<dyn RuntimeFileStore>);

#[async_trait]
impl SessionFileStore for DynFileStore {
    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        self.0.read_file(session_id, path).await
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        self.0.write_file(session_id, path, content, encoding).await
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        self.0.delete_file(session_id, path, recursive).await
    }

    async fn list_directory(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> Result<Vec<everruns_core::session_file::FileInfo>> {
        self.0.list_directory(session_id, path).await
    }

    async fn stat_file(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> Result<Option<everruns_core::session_file::FileStat>> {
        self.0.stat_file(session_id, path).await
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<everruns_core::session_file::GrepMatch>> {
        self.0.grep_files(session_id, pattern, path_pattern).await
    }

    async fn create_directory(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> Result<everruns_core::session_file::FileInfo> {
        self.0.create_directory(session_id, path).await
    }

    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        self.0
            .write_file_if_content_matches(
                session_id,
                path,
                expected_content,
                expected_encoding,
                content,
                encoding,
            )
            .await
    }
}

#[derive(Clone)]
pub(crate) struct DynProviderStore(pub Arc<dyn RuntimeProviderStore>);

#[async_trait]
impl LlmProviderStore for DynProviderStore {
    async fn get_model_with_provider(
        &self,
        model_id: ModelId,
    ) -> Result<Option<ModelWithProvider>> {
        self.0.get_model_with_provider(model_id).await
    }

    async fn get_default_model(&self) -> Result<Option<ModelWithProvider>> {
        self.0.get_default_model().await
    }
}

#[derive(Clone)]
pub(crate) struct DynEventEmitter(pub Arc<dyn EventEmitter>);

#[async_trait]
impl EventEmitter for DynEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        self.0.emit(request).await
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
impl RuntimeFileStore for InMemorySessionFileStore {
    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        InMemorySessionFileStore::seed_initial_file(self, session_id, file).await
    }
}

#[async_trait]
impl RuntimeProviderStore for InMemoryLlmProviderStore {
    async fn set_default_model(&self, model: ModelWithProvider) -> Result<()> {
        InMemoryLlmProviderStore::set_default_model(self, model).await;
        Ok(())
    }
}

#[async_trait]
impl RuntimeEventCollector for InMemoryEventEmitter {
    async fn events(&self) -> Vec<Event> {
        InMemoryEventEmitter::events(self).await
    }
}
