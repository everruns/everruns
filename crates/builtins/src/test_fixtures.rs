use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::message::Message;
use everruns_core::message_filter::{MessageFilter, MessageQuery};
use everruns_core::message_retriever::{MessageHistory, MessageRetriever};
use everruns_core::typed_id::{MessageId, SessionId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Private fixture for collocated built-in capability tests.
#[derive(Clone, Default)]
pub(crate) struct TestMessageRetriever {
    messages: Arc<RwLock<HashMap<SessionId, Vec<Message>>>>,
}

impl TestMessageRetriever {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) async fn seed(&self, session_id: SessionId, messages: Vec<Message>) {
        self.messages.write().await.insert(session_id, messages);
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
