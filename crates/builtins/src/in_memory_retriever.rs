//! In-memory `MessageRetriever` for embedders and isolated tests.
//!
//! A process-local, writable message store implementing the neutral
//! [`MessageRetriever`] contract from `everruns-core`. It is a production API
//! (not a test-only fixture): embedders that run without a host-backed store —
//! or that supply their own conversation history — can use it directly in the
//! build path. `everruns-test-support` re-exports it for existing test users.

use async_trait::async_trait;
use chrono::Utc;
use everruns_core::message::Message;
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::message_retriever::{InputMessage, MessageHistory, MessageRetriever};
use everruns_provider::error::Result;
use everruns_provider::typed_id::{MessageId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Writable, process-local message store implementing [`MessageRetriever`].
#[derive(Debug, Default, Clone)]
pub struct InMemoryMessageRetriever {
    messages: Arc<RwLock<HashMap<SessionId, Vec<Message>>>>,
}

impl InMemoryMessageRetriever {
    /// Create an empty message store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Return session ids currently represented in the store.
    pub async fn sessions(&self) -> Vec<SessionId> {
        self.messages.read().await.keys().copied().collect()
    }

    /// Remove every stored message.
    pub async fn clear(&self) {
        self.messages.write().await.clear();
    }

    /// Remove messages for one session.
    pub async fn clear_session(&self, session_id: SessionId) {
        self.messages.write().await.remove(&session_id);
    }

    /// Replace one session's message history.
    pub async fn seed(&self, session_id: SessionId, messages: Vec<Message>) {
        self.messages.write().await.insert(session_id, messages);
    }

    /// Construct and append a message from input.
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
