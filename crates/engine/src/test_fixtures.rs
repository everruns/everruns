use async_trait::async_trait;
use chrono::Utc;
use everruns_core::durability::{PartialStreamState, PartialStreamStore};
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{Event, EventRequest};
use everruns_core::message::Message;
use everruns_core::message_filter::MessageQuery;
use everruns_core::message_retriever::{InputMessage, MessageHistory, MessageRetriever};
use everruns_provider::error::Result;
use everruns_provider::typed_id::{EventId, MessageId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Default, Clone)]
pub(crate) struct TestMessageRetriever {
    messages: Arc<RwLock<HashMap<SessionId, Vec<Message>>>>,
}

impl TestMessageRetriever {
    pub(crate) fn new() -> Self {
        Self::default()
    }

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
}

#[async_trait]
impl MessageRetriever for TestMessageRetriever {
    async fn get(&self, session_id: SessionId, message_id: MessageId) -> Result<Option<Message>> {
        Ok(self
            .messages
            .read()
            .await
            .get(&session_id)
            .and_then(|messages| {
                messages
                    .iter()
                    .find(|message| message.id == message_id)
                    .cloned()
            }))
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
        Ok(self.load(query.session_id).await?)
    }

    async fn load_filtered_history(&self, query: MessageQuery) -> Result<MessageHistory> {
        let messages = self.load(query.session_id).await?;
        Ok(MessageHistory {
            source_sequence: Some(messages.len() as i64),
            messages,
        })
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct TestEventEmitter {
    events: Arc<RwLock<Vec<Event>>>,
    sequence: Arc<RwLock<i32>>,
}

impl TestEventEmitter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn events(&self) -> Vec<Event> {
        self.events.read().await.clone()
    }
}

#[async_trait]
impl EventEmitter for TestEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        let mut sequence = self.sequence.write().await;
        *sequence += 1;
        let event = request.into_event(EventId::new(), *sequence);
        self.events.write().await.push(event.clone());
        Ok(event)
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct NoopEventEmitter;

#[async_trait]
impl EventEmitter for NoopEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        Ok(request.into_event(EventId::new(), 0))
    }
}

pub(crate) struct NoopPartialStreamStore;

#[async_trait]
impl PartialStreamStore for NoopPartialStreamStore {
    async fn get_partial_stream(
        &self,
        _session_id: SessionId,
        _turn_id: &str,
    ) -> Result<Option<PartialStreamState>> {
        Ok(None)
    }
}
