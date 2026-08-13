//! Deterministic in-memory fixtures for isolated tests.
//!
//! These fixtures deliberately expose setup writes. Application runtimes use
//! `everruns-host` stores and append conversation state to an `EventLog`.

use async_trait::async_trait;
use chrono::Utc;
use everruns_core::error::Result;
use everruns_core::events::{Event, EventRequest};
use everruns_core::message::Message;
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::message_retriever::{InputMessage, MessageHistory, MessageRetriever};
use everruns_core::traits::EventEmitter;
use everruns_core::typed_id::{EventId, MessageId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Writable message fixture for isolated tests that do not exercise a host.
#[derive(Debug, Default, Clone)]
pub struct InMemoryMessageRetriever {
    messages: Arc<RwLock<HashMap<SessionId, Vec<Message>>>>,
}

impl InMemoryMessageRetriever {
    /// Create an empty message fixture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return session ids currently represented in the fixture.
    pub async fn sessions(&self) -> Vec<SessionId> {
        self.messages.read().await.keys().copied().collect()
    }

    /// Remove every seeded message.
    pub async fn clear(&self) {
        self.messages.write().await.clear();
    }

    /// Remove messages for one session.
    pub async fn clear_session(&self, session_id: SessionId) {
        self.messages.write().await.remove(&session_id);
    }

    /// Replace one session's deterministic message history.
    pub async fn seed(&self, session_id: SessionId, messages: Vec<Message>) {
        self.messages.write().await.insert(session_id, messages);
    }

    /// Construct and append a message from test input.
    pub async fn add(&self, session_id: SessionId, input: InputMessage) -> Result<Message> {
        let message = Message {
            id: MessageId::new(),
            role: input.role,
            content: input.content,
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: input.controls,
            metadata: input.metadata,
            external_actor: None,
            created_at: Utc::now(),
        };
        self.store(session_id, message.clone()).await?;
        Ok(message)
    }

    /// Append an already-constructed message.
    pub async fn store(&self, session_id: SessionId, message: Message) -> Result<()> {
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
impl MessageRetriever for InMemoryMessageRetriever {
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
        let mut messages = self.load(query.session_id).await?;
        if let Some(after) = query.after_sequence {
            messages = messages.into_iter().skip(after.max(0) as usize).collect();
        }
        for filter in &query.filters {
            match filter {
                MessageFilter::TimeRange { from, to } => messages.retain(|message| {
                    from.is_none_or(|time| message.created_at >= time)
                        && to.is_none_or(|time| message.created_at <= time)
                }),
                MessageFilter::Search(query) => {
                    let query = query.to_lowercase();
                    messages.retain(|message| {
                        message
                            .text()
                            .is_some_and(|text| text.to_lowercase().contains(&query))
                    });
                }
                MessageFilter::Custom(predicate) => messages.retain(|message| predicate(message)),
                _ => {}
            }
        }
        query.apply_windowing(&mut messages);
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
            .map(Vec::len)
            .unwrap_or(0))
    }
}

/// Event emitter fixture with deterministic, process-local sequencing.
#[derive(Debug, Default, Clone)]
pub struct InMemoryEventEmitter {
    events: Arc<RwLock<Vec<Event>>>,
    sequence: Arc<RwLock<i32>>,
}

impl InMemoryEventEmitter {
    /// Create an empty event fixture.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return all emitted events in order.
    pub async fn events(&self) -> Vec<Event> {
        self.events.read().await.clone()
    }

    /// Return the number of emitted events.
    pub async fn event_count(&self) -> usize {
        self.events.read().await.len()
    }

    /// Remove all events and reset deterministic sequencing.
    pub async fn clear(&self) {
        self.events.write().await.clear();
        *self.sequence.write().await = 0;
    }

    /// Return events with the requested protocol type.
    pub async fn events_by_type(&self, event_type: &str) -> Vec<Event> {
        self.events
            .read()
            .await
            .iter()
            .filter(|event| event.event_type == event_type)
            .cloned()
            .collect()
    }

    /// Return events emitted for one session UUID.
    pub async fn events_for_session(&self, session_id: Uuid) -> Vec<Event> {
        self.events
            .read()
            .await
            .iter()
            .filter(|event| event.session_uuid() == session_id)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl EventEmitter for InMemoryEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        let mut sequence = self.sequence.write().await;
        *sequence += 1;
        let event = request.into_event(EventId::new(), *sequence);
        self.events.write().await.push(event.clone());
        Ok(event)
    }
}
