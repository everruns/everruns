//! Conversation controls for application-owned channel hosts.
use crate::channels::{ChannelError, ControlAction};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
};
/// Platform conversation metadata, separate from executable configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConversationState {
    /// Last platform title.
    pub title: Option<String>,
    /// Latest platform context per key. Treat values as untrusted reference data.
    pub context: BTreeMap<String, serde_json::Value>,
}
/// Application persistence for pane title/view updates. Apply must atomically merge one action.
#[async_trait]
pub trait ConversationStore: Send + Sync {
    /// Read current metadata.
    async fn load(&self, session: &str) -> Result<ConversationState, ChannelError>;
    /// Merge Rename or Context. Cancel is handled by the Engine.
    async fn apply(&self, session: &str, action: &ControlAction) -> Result<(), ChannelError>;
}
/// Bounded process-local conversation metadata. Supply a durable store for recovery across restarts.
#[derive(Clone, Default)]
pub struct MemoryConversationStore(Arc<Mutex<HashMap<String, ConversationState>>>);
#[async_trait]
impl ConversationStore for MemoryConversationStore {
    async fn load(&self, session: &str) -> Result<ConversationState, ChannelError> {
        Ok(self
            .0
            .lock()
            .map_err(|_| ChannelError::Session("Conversation store unavailable".into()))?
            .get(session)
            .cloned()
            .unwrap_or_default())
    }
    async fn apply(&self, session: &str, action: &ControlAction) -> Result<(), ChannelError> {
        let mut store = self
            .0
            .lock()
            .map_err(|_| ChannelError::Session("Conversation store unavailable".into()))?;
        if store.len() >= 10000 && !store.contains_key(session) {
            return Err(ChannelError::Session(
                "Conversation metadata store is full".into(),
            ));
        }
        let mut state = store.get(session).cloned().unwrap_or_default();
        match action {
            ControlAction::Rename { title } => {
                if title.len() > 1024 {
                    return Err(ChannelError::BadRequest("Title exceeds limit".into()));
                }
                state.title = Some(title.clone());
            }
            ControlAction::Context { key, value } => {
                if key.len() > 128 {
                    return Err(ChannelError::BadRequest("Context key exceeds limit".into()));
                }
                state.context.insert(key.clone(), value.clone());
            }
            ControlAction::Cancel => return Ok(()),
        }
        if state.context.len() > 16
            || serde_json::to_vec(&state).map_or(true, |v| v.len() > 64 * 1024)
        {
            return Err(ChannelError::BadRequest(
                "Conversation context exceeds limit".into(),
            ));
        }
        store.insert(session.into(), state);
        Ok(())
    }
}
