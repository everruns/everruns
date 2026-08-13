#![allow(dead_code)]

// Private, cfg(test)-only doubles for collocated core unit tests. These are not
// public backends; host owns application stores and test-support owns reusable
// deterministic fixtures.

use crate::agent_definition::AgentDefinition;
use crate::credential_provider::CredentialProvider;
use crate::harness_definition::HarnessDefinition;
use crate::provider::DriverId;
use crate::session::ExecutionSession;

use crate::traits::ResolvedModel;
use crate::typed_id::{AgentId, EventId, HarnessId, MessageId, ModelId, SessionId};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::Result;
use crate::message::Message;
use crate::message_filter::MessageQuery;
use crate::message_retriever::{InputMessage, MessageHistory, MessageRetriever};
use crate::traits::{AgentStore, HarnessStore, ProviderStore, SessionStore};
use chrono::Utc;

// ============================================================================
// TestMessageRetriever - In-memory message storage for testing
// ============================================================================

/// In-memory message retriever
///
/// Stores messages in a HashMap keyed by session ID.
/// Implements the `MessageRetriever` trait for retrieval operations.
///
/// Note: Write operations (add, store) are provided as inherent methods
/// for testing purposes. In production, messages are stored via EventEmitter.
#[derive(Debug, Default, Clone)]
pub(crate) struct TestMessageRetriever {
    messages: Arc<RwLock<HashMap<SessionId, Vec<Message>>>>,
}

impl TestMessageRetriever {
    /// Create a new in-memory message retriever
    pub(crate) fn new() -> Self {
        Self {
            messages: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get all sessions
    pub(crate) async fn sessions(&self) -> Vec<SessionId> {
        self.messages.read().await.keys().copied().collect()
    }

    /// Clear all messages
    pub(crate) async fn clear(&self) {
        self.messages.write().await.clear();
    }

    /// Clear messages for a specific session
    pub(crate) async fn clear_session(&self, session_id: SessionId) {
        self.messages.write().await.remove(&session_id);
    }

    /// Pre-populate with messages (useful for testing)
    pub(crate) async fn seed(&self, session_id: SessionId, messages: Vec<Message>) {
        self.messages.write().await.insert(session_id, messages);
    }

    /// Add a new message and return it with generated ID (for testing)
    ///
    /// Note: In production, messages are stored via EventService.
    /// This method is provided for test setup and in-memory usage.
    pub(crate) async fn add(&self, session_id: SessionId, input: InputMessage) -> Result<Message> {
        let message = Message {
            id: MessageId::new(),
            role: input.role,
            content: input.content,
            phase: None,
            thinking: None, // InputMessage doesn't include thinking (user messages don't have thinking)
            thinking_signature: None,
            controls: input.controls,
            metadata: input.metadata,
            external_actor: None,
            created_at: Utc::now(),
        };

        self.messages
            .write()
            .await
            .entry(session_id)
            .or_default()
            .push(message.clone());

        Ok(message)
    }

    /// Store an existing message (for testing)
    ///
    /// Note: In production, messages are stored via EventEmitter.
    /// This method is provided for test setup and in-memory usage.
    pub(crate) async fn store(&self, session_id: SessionId, message: Message) -> Result<()> {
        self.messages
            .write()
            .await
            .entry(session_id)
            .or_default()
            .push(message);
        Ok(())
    }
}

#[async_trait]
impl MessageRetriever for TestMessageRetriever {
    async fn get(&self, session_id: SessionId, message_id: MessageId) -> Result<Option<Message>> {
        Ok(self
            .messages
            .read()
            .await
            .get(&session_id)
            .and_then(|messages| messages.iter().find(|m| m.id == message_id).cloned()))
    }

    async fn load(&self, session_id: SessionId) -> Result<Vec<Message>> {
        Ok(self
            .messages
            .read()
            .await
            .get(&session_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn load_filtered(&self, query: MessageQuery) -> Result<Vec<Message>> {
        use crate::message_filter::MessageFilter;

        let mut messages = self.load(query.session_id).await?;
        if let Some(after) = query.after_sequence {
            messages = messages.into_iter().skip(after.max(0) as usize).collect();
        }

        // Apply filters
        for filter in &query.filters {
            match filter {
                MessageFilter::TimeRange { from, to } => {
                    messages.retain(|m| {
                        let after_from = from.is_none_or(|t| m.created_at >= t);
                        let before_to = to.is_none_or(|t| m.created_at <= t);
                        after_from && before_to
                    });
                }
                MessageFilter::Search(q) => {
                    let q_lower = q.to_lowercase();
                    messages.retain(|m| {
                        m.text()
                            .is_some_and(|t| t.to_lowercase().contains(&q_lower))
                    });
                }
                MessageFilter::Custom(predicate) => {
                    messages.retain(|m| predicate(m));
                }
                // Other filters not commonly used in-memory
                _ => {}
            }
        }

        query.apply_windowing(&mut messages);

        // Apply injections
        if query.has_injections() {
            query.apply_injections(&mut messages);
        }

        Ok(messages)
    }

    async fn load_filtered_history(&self, query: MessageQuery) -> Result<MessageHistory> {
        let source_sequence = self
            .messages
            .read()
            .await
            .get(&query.session_id)
            .map(|messages| messages.len() as i64)
            .unwrap_or(0);
        Ok(MessageHistory {
            messages: self.load_filtered(query).await?,
            source_sequence: Some(source_sequence),
        })
    }

    async fn count(&self, session_id: SessionId) -> Result<usize> {
        Ok(self
            .messages
            .read()
            .await
            .get(&session_id)
            .map(|m| m.len())
            .unwrap_or(0))
    }
}

// ============================================================================
// TestAgentStore - Stores agents in memory
// ============================================================================

/// In-memory agent store
///
/// Stores agents in a HashMap keyed by agent ID.
/// Useful for testing and examples where you want to configure agents without a database.
#[derive(Debug, Default, Clone)]
pub(crate) struct TestAgentStore {
    agents: Arc<RwLock<HashMap<AgentId, AgentDefinition>>>,
}

impl TestAgentStore {
    /// Create a new in-memory agent store
    pub(crate) fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add an agent to the store
    pub(crate) async fn add_agent(&self, agent: AgentDefinition) {
        self.agents.write().await.insert(agent.id, agent);
    }

    /// Get all agent IDs
    pub(crate) async fn agent_ids(&self) -> Vec<AgentId> {
        self.agents.read().await.keys().copied().collect()
    }

    /// Clear all agents
    pub(crate) async fn clear(&self) {
        self.agents.write().await.clear();
    }
}

#[async_trait]
impl AgentStore for TestAgentStore {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<AgentDefinition>> {
        Ok(self.agents.read().await.get(&agent_id).cloned())
    }
}

// ============================================================================
// TestHarnessStore - Stores harnesses in memory
// ============================================================================

/// In-memory harness store
///
/// Stores effective harness execution definitions in a HashMap keyed by
/// harness ID (EVE-881: the id keys the association with sessions; the value
/// itself is the portable, id-free configuration). Useful for testing and
/// examples where you want to configure harnesses without a database.
#[derive(Debug, Default, Clone)]
pub(crate) struct TestHarnessStore {
    harnesses: Arc<RwLock<HashMap<HarnessId, HarnessDefinition>>>,
}

impl TestHarnessStore {
    /// Create a new in-memory harness store
    pub(crate) fn new() -> Self {
        Self {
            harnesses: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a harness definition to the store under the given id.
    pub(crate) async fn add_harness(&self, harness_id: HarnessId, harness: HarnessDefinition) {
        self.harnesses.write().await.insert(harness_id, harness);
    }
}

#[async_trait]
impl HarnessStore for TestHarnessStore {
    async fn get_harness(&self, harness_id: HarnessId) -> Result<Option<HarnessDefinition>> {
        Ok(self.harnesses.read().await.get(&harness_id).cloned())
    }
}

// ============================================================================
// TestSessionStore - Stores sessions in memory
// ============================================================================

/// In-memory session store
///
/// Stores sessions in a HashMap keyed by session ID.
/// Useful for testing and examples where you want to configure sessions without a database.
#[derive(Debug, Default, Clone)]
pub(crate) struct TestSessionStore {
    sessions: Arc<RwLock<HashMap<SessionId, ExecutionSession>>>,
}

impl TestSessionStore {
    /// Create a new in-memory session store
    pub(crate) fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Add a session to the store
    pub(crate) async fn add_session(&self, session: ExecutionSession) {
        self.sessions.write().await.insert(session.id, session);
    }

    /// Get all session IDs
    pub(crate) async fn session_ids(&self) -> Vec<SessionId> {
        self.sessions.read().await.keys().copied().collect()
    }

    /// Clear all sessions
    pub(crate) async fn clear(&self) {
        self.sessions.write().await.clear();
    }
}

#[async_trait]
impl SessionStore for TestSessionStore {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        Ok(self.sessions.read().await.get(&session_id).cloned())
    }
}

// ============================================================================
// TestProviderStore - Stores LLM provider configurations in memory
// ============================================================================

/// In-memory LLM provider store
///
/// Stores model configurations in a HashMap keyed by model UUID.
/// Useful for testing and examples where you want to configure providers without a database.
///
/// # Example
///
/// ```ignore
/// use everruns_core::test_fixtures::TestProviderStore;
/// use everruns_core::EnvCredentialProvider;
///
/// let store = TestProviderStore::from_credential_provider(&EnvCredentialProvider).await;
/// // Uses OPENAI_API_KEY or ANTHROPIC_API_KEY via the injected provider
/// ```
#[derive(Debug, Default, Clone)]
pub(crate) struct TestProviderStore {
    models: Arc<RwLock<HashMap<ModelId, ResolvedModel>>>,
    default_model: Arc<RwLock<Option<ResolvedModel>>>,
}

impl TestProviderStore {
    /// Create a new empty in-memory provider store
    pub(crate) fn new() -> Self {
        Self {
            models: Arc::new(RwLock::new(HashMap::new())),
            default_model: Arc::new(RwLock::new(None)),
        }
    }

    /// Create a provider store from an injected [`CredentialProvider`].
    ///
    /// Checks OpenAI first, then Anthropic, and configures a default model for
    /// whichever the provider resolves credentials for. The store never reads
    /// the process environment itself; standalone/dev callers pass
    /// [`EnvCredentialProvider`](crate::credential_provider::EnvCredentialProvider)
    /// to opt into env-based credentials.
    pub(crate) async fn from_credential_provider(provider: &dyn CredentialProvider) -> Self {
        let store = Self::new();

        // Check for OpenAI first, then Anthropic.
        if let Some(creds) = provider
            .resolve(&DriverId::OpenAI)
            .filter(|c| c.api_key.is_some())
        {
            store
                .set_default_model(ResolvedModel {
                    model: "gpt-5.4".to_string(),
                    provider_type: DriverId::OpenAI,
                    api_key: creds.api_key,
                    base_url: creds.base_url,
                    provider_metadata: None,
                })
                .await;
        } else if let Some(creds) = provider
            .resolve(&DriverId::Anthropic)
            .filter(|c| c.api_key.is_some())
        {
            store
                .set_default_model(ResolvedModel {
                    model: "claude-sonnet-4-20250514".to_string(),
                    provider_type: DriverId::Anthropic,
                    api_key: creds.api_key,
                    base_url: creds.base_url,
                    provider_metadata: None,
                })
                .await;
        }

        store
    }

    /// Create a provider store with a specific default model
    pub(crate) async fn with_default(model: ResolvedModel) -> Self {
        let store = Self::new();
        store.set_default_model(model).await;
        store
    }

    /// Add a model to the store
    pub(crate) async fn add_model(&self, model_id: ModelId, model: ResolvedModel) {
        self.models.write().await.insert(model_id, model);
    }

    /// Set the default model
    pub(crate) async fn set_default_model(&self, model: ResolvedModel) {
        *self.default_model.write().await = Some(model);
    }

    /// Clear all models
    pub(crate) async fn clear(&self) {
        self.models.write().await.clear();
        *self.default_model.write().await = None;
    }
}

#[async_trait]
impl ProviderStore for TestProviderStore {
    async fn get_resolved_model(&self, model_id: ModelId) -> Result<Option<ResolvedModel>> {
        Ok(self.models.read().await.get(&model_id).cloned())
    }

    async fn get_default_model(&self) -> Result<Option<ResolvedModel>> {
        Ok(self.default_model.read().await.clone())
    }
}

// ============================================================================
// TestEventEmitter - Stores events in memory for testing
// ============================================================================

use crate::events::{Event, EventRequest};
use crate::traits::EventEmitter;

/// In-memory event emitter for testing
///
/// Stores emitted events in memory for inspection.
/// Useful for testing and examples where you want to verify events without a database.
///
/// # Example
///
/// ```ignore
/// use everruns_core::test_fixtures::TestEventEmitter;
///
/// let emitter = TestEventEmitter::new();
///
/// // Emit events...
///
/// // Check emitted events
/// let events = emitter.events().await;
/// assert_eq!(events.len(), 2);
/// ```
#[derive(Debug, Default, Clone)]
pub(crate) struct TestEventEmitter {
    events: Arc<RwLock<Vec<Event>>>,
    sequence: Arc<RwLock<i32>>,
}

impl TestEventEmitter {
    /// Create a new in-memory event emitter
    pub(crate) fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
            sequence: Arc::new(RwLock::new(0)),
        }
    }

    /// Get all emitted events
    pub(crate) async fn events(&self) -> Vec<Event> {
        self.events.read().await.clone()
    }

    /// Get the count of emitted events
    pub(crate) async fn event_count(&self) -> usize {
        self.events.read().await.len()
    }

    /// Clear all events
    pub(crate) async fn clear(&self) {
        self.events.write().await.clear();
        *self.sequence.write().await = 0;
    }

    /// Get events by type
    pub(crate) async fn events_by_type(&self, event_type: &str) -> Vec<Event> {
        self.events
            .read()
            .await
            .iter()
            .filter(|e| e.event_type == event_type)
            .cloned()
            .collect()
    }

    /// Get events for a specific session
    pub(crate) async fn events_for_session(&self, session_id: Uuid) -> Vec<Event> {
        self.events
            .read()
            .await
            .iter()
            .filter(|e| e.session_uuid() == session_id)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl EventEmitter for TestEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        let mut sequence = self.sequence.write().await;
        *sequence += 1;
        let seq = *sequence;
        drop(sequence);

        // Convert EventRequest to Event with generated id and sequence
        let event = request.into_event(EventId::new(), seq);
        self.events.write().await.push(event.clone());
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_in_memory_message_retriever() {
        let store = TestMessageRetriever::new();
        let session_id: SessionId = Uuid::now_v7().into();

        store
            .store(session_id, Message::user("Hello"))
            .await
            .unwrap();

        let messages = store.load(session_id).await.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text(), Some("Hello"));
    }

    #[tokio::test]
    async fn test_in_memory_message_retriever_add_and_get() {
        let store = TestMessageRetriever::new();
        let session_id: SessionId = Uuid::now_v7().into();

        // Add a message using the add method
        let message = store
            .add(session_id, InputMessage::user("Hello via add"))
            .await
            .unwrap();

        // Get the message by ID
        let retrieved = store.get(session_id, message.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().text(), Some("Hello via add"));

        // Get non-existent message
        let missing = store.get(session_id, MessageId::new()).await.unwrap();
        assert!(missing.is_none());
    }

    /// Regression test: add() must return message with ID usable for get()
    ///
    /// This test documents a critical invariant: the ID in the message returned by
    /// add() must match the ID stored internally, so that get(returned_id) succeeds.
    #[tokio::test]
    async fn test_message_retriever_add_returns_consistent_id() {
        let store = TestMessageRetriever::new();
        let session_id: SessionId = Uuid::now_v7().into();

        // Add a message
        let added = store
            .add(session_id, InputMessage::user("Test consistency"))
            .await
            .unwrap();

        // The returned message ID must be retrievable
        let retrieved = store.get(session_id, added.id).await.unwrap();
        assert!(
            retrieved.is_some(),
            "Message must be retrievable by the ID returned from add()"
        );

        // The retrieved message must have the same ID
        let retrieved = retrieved.unwrap();
        assert_eq!(
            retrieved.id, added.id,
            "Retrieved message ID must match the ID returned from add()"
        );

        // The message must also appear in load() with the same ID
        let all_messages = store.load(session_id).await.unwrap();
        let found = all_messages.iter().find(|m| m.id == added.id);
        assert!(
            found.is_some(),
            "Message with returned ID must appear in load() results"
        );
    }

    #[tokio::test]
    async fn test_in_memory_event_emitter() {
        use crate::events::{EventContext, EventRequest, InputMessageData};

        let emitter = TestEventEmitter::new();
        let session_id: SessionId = Uuid::now_v7().into();
        let event_context = EventContext::empty();

        // Emit an event
        let event1 = emitter
            .emit(EventRequest::new(
                session_id,
                event_context.clone(),
                InputMessageData::new(Message::user("test1")),
            ))
            .await
            .unwrap();
        assert_eq!(event1.sequence, Some(1));

        // Emit another event
        let event2 = emitter
            .emit(EventRequest::new(
                session_id,
                event_context,
                InputMessageData::new(Message::user("test2")),
            ))
            .await
            .unwrap();
        assert_eq!(event2.sequence, Some(2));

        // Check events
        let events = emitter.events().await;
        assert_eq!(events.len(), 2);
        assert_eq!(emitter.event_count().await, 2);
    }

    #[tokio::test]
    async fn test_in_memory_event_emitter_filter_by_type() {
        use crate::events::{
            EventContext, EventRequest, INPUT_MESSAGE, InputMessageData, REASON_STARTED,
            ReasonStartedData,
        };

        let emitter = TestEventEmitter::new();
        let session_id: SessionId = Uuid::now_v7().into();
        let event_context = EventContext::empty();

        // Emit different event types
        emitter
            .emit(EventRequest::new(
                session_id,
                event_context.clone(),
                InputMessageData::new(Message::user("test")),
            ))
            .await
            .unwrap();

        emitter
            .emit(EventRequest::new(
                session_id,
                event_context,
                ReasonStartedData {
                    harness_id: HarnessId::from_seed(1),
                    agent_id: Some(AgentId::new()),
                    metadata: None,
                },
            ))
            .await
            .unwrap();

        // Filter by type
        let received_events = emitter.events_by_type(INPUT_MESSAGE).await;
        assert_eq!(received_events.len(), 1);

        let started_events = emitter.events_by_type(REASON_STARTED).await;
        assert_eq!(started_events.len(), 1);
    }

    #[tokio::test]
    async fn test_in_memory_event_emitter_filter_by_session() {
        use crate::events::{EventContext, EventRequest, InputMessageData};

        let emitter = TestEventEmitter::new();
        let session1: SessionId = Uuid::now_v7().into();
        let session2: SessionId = Uuid::now_v7().into();

        // Emit events for different sessions
        let context = EventContext::empty();

        emitter
            .emit(EventRequest::new(
                session1,
                context.clone(),
                InputMessageData::new(Message::user("session1")),
            ))
            .await
            .unwrap();
        emitter
            .emit(EventRequest::new(
                session2,
                context,
                InputMessageData::new(Message::user("session2")),
            ))
            .await
            .unwrap();

        // Filter by session
        let session1_events = emitter.events_for_session(session1.uuid()).await;
        assert_eq!(session1_events.len(), 1);

        let session2_events = emitter.events_for_session(session2.uuid()).await;
        assert_eq!(session2_events.len(), 1);
    }

    #[tokio::test]
    async fn test_in_memory_event_emitter_clear() {
        use crate::events::{EventContext, EventRequest, InputMessageData};

        let emitter = TestEventEmitter::new();
        let session_id: SessionId = Uuid::now_v7().into();
        let event_context = EventContext::empty();

        emitter
            .emit(EventRequest::new(
                session_id,
                event_context,
                InputMessageData::new(Message::user("test")),
            ))
            .await
            .unwrap();

        assert_eq!(emitter.event_count().await, 1);

        emitter.clear().await;

        assert_eq!(emitter.event_count().await, 0);
    }
}
