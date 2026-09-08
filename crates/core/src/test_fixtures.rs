#![allow(dead_code)]

// Private, cfg(test)-only doubles for collocated core unit tests. These are not
// public backends; host owns application stores and test-support owns reusable
// deterministic fixtures.

use crate::agent_definition::AgentDefinition;
use crate::harness_definition::HarnessDefinition;
use crate::session::ExecutionSession;

use crate::typed_id::{AgentId, EventId, HarnessId, MessageId, SessionId};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::error::Result;
use crate::message::Message;
use crate::message_filter::MessageQuery;
use crate::message_retriever::{InputMessage, MessageHistory, MessageRetriever};
use crate::{
    execution_loading::AgentStore, execution_loading::HarnessStore, execution_loading::SessionStore,
};
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
            phase_source: None,
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
// TestEventEmitter - Stores events in memory for testing
// ============================================================================

use crate::event_emitter::EventEmitter;
use crate::events::{Event, EventRequest};

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

    fn values<T: serde::Serialize>(values: T) -> serde_json::Value {
        serde_json::to_value(values).unwrap()
    }

    #[tokio::test]
    async fn message_store_preserves_complete_records_and_session_isolation() {
        let store = TestMessageRetriever::new();
        let first = SessionId::from_uuid(Uuid::from_u128(1));
        let second = SessionId::from_uuid(Uuid::from_u128(2));
        let stored = Message::user("stored");
        store.store(first, stored.clone()).await.unwrap();
        let added = store.add(first, InputMessage::user("added")).await.unwrap();
        let other = store
            .add(second, InputMessage::user("other session"))
            .await
            .unwrap();
        assert_eq!(
            values(store.get(first, stored.id).await.unwrap()),
            values(Some(&stored))
        );
        assert_eq!(
            values(store.get(first, added.id).await.unwrap()),
            values(Some(&added))
        );
        assert!(store.get(second, added.id).await.unwrap().is_none());
        assert_eq!(
            values(store.load(first).await.unwrap()),
            values([&stored, &added])
        );
        assert_eq!(values(store.load(second).await.unwrap()), values([&other]));
        assert_eq!(store.count(first).await.unwrap(), 2);
        store.clear_session(first).await;
        assert!(store.load(first).await.unwrap().is_empty());
        assert_eq!(values(store.load(second).await.unwrap()), values([&other]));
        store.seed(second, vec![stored.clone()]).await;
        assert_eq!(values(store.load(second).await.unwrap()), values([&stored]));
        store.clear().await;
        assert!(store.sessions().await.is_empty());
        assert!(store.get(first, added.id).await.unwrap().is_none());
        assert_eq!(store.count(second).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn event_store_preserves_records_filters_exactly_and_resets_sequence() {
        use crate::events::{EventContext, EventRequest, InputMessageData, ReasonStartedData};
        let emitter = TestEventEmitter::new();
        let first = SessionId::from_uuid(Uuid::from_u128(1));
        let second = SessionId::from_uuid(Uuid::from_u128(2));
        let input = emitter
            .emit(EventRequest::new(
                first,
                EventContext::empty(),
                InputMessageData::new(Message::user("first")),
            ))
            .await
            .unwrap();
        let reason = emitter
            .emit(EventRequest::new(
                second,
                EventContext::empty(),
                ReasonStartedData {
                    harness_id: HarnessId::from_seed(1),
                    agent_id: None,
                    metadata: None,
                },
            ))
            .await
            .unwrap();
        let last = emitter
            .emit(EventRequest::new(
                first,
                EventContext::empty(),
                InputMessageData::new(Message::user("last")),
            ))
            .await
            .unwrap();
        assert_eq!(
            [input.sequence, reason.sequence, last.sequence],
            [Some(1), Some(2), Some(3)]
        );
        assert_eq!(
            values(emitter.events().await),
            values([&input, &reason, &last])
        );
        assert_eq!(emitter.event_count().await, 3);
        assert_eq!(
            values(emitter.events_by_type("input.message").await),
            values([&input, &last])
        );
        assert_eq!(
            values(emitter.events_by_type("reason.started").await),
            values([&reason])
        );
        assert!(emitter.events_by_type("missing").await.is_empty());
        assert_eq!(
            values(emitter.events_for_session(first.uuid()).await),
            values([&input, &last])
        );
        assert_eq!(
            values(emitter.events_for_session(second.uuid()).await),
            values([&reason])
        );
        assert!(
            emitter
                .events_for_session(Uuid::from_u128(3))
                .await
                .is_empty()
        );
        emitter.clear().await;
        assert!(emitter.events().await.is_empty());
        assert_eq!(emitter.event_count().await, 0);
        let reset = emitter
            .emit(EventRequest::new(
                first,
                EventContext::empty(),
                InputMessageData::new(Message::user("after reset")),
            ))
            .await
            .unwrap();
        assert_eq!(reset.sequence, Some(1));
        assert_eq!(values(emitter.events().await), values([&reset]));
    }
}
