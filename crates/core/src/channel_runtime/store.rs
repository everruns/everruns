//! What a channel host persists.
//!
//! Three things, and nothing about sessions themselves: which session a
//! binding key maps to, which platform deliveries were already accepted (a
//! platform retries a slow acknowledgement, and the retry must not start a
//! second turn), and which turns still owe a reply (so a restart can finish
//! them). Memory, SQLite and Postgres implement the same trait, as the durable
//! stores do.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{ChannelError, DeliveryTarget};

/// A turn that still owes its conversation a reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDelivery {
    pub channel: String,
    pub session_id: String,
    pub input_message_id: String,
    pub target: DeliveryTarget,
    /// Last durable sequence seen before the turn started; replay resumes
    /// after it.
    pub after_sequence: i64,
}

/// Channel host persistence.
#[async_trait]
pub trait ChannelStore: Send + Sync {
    /// The session bound to `key` on `channel`.
    async fn session_for(&self, channel: &str, key: &str) -> Result<Option<String>, ChannelError>;

    /// Bind `key` on `channel` to `session_id`.
    async fn bind(&self, channel: &str, key: &str, session_id: &str) -> Result<(), ChannelError>;

    /// Record a platform delivery key. `true` the first time, `false` for a
    /// retry of a delivery already accepted.
    async fn first_sighting(&self, channel: &str, dedup_key: &str) -> Result<bool, ChannelError>;

    /// Save a pending delivery.
    async fn save_pending(&self, pending: &PendingDelivery) -> Result<(), ChannelError>;

    /// Clear a pending delivery once its turn ended.
    async fn clear_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
    ) -> Result<(), ChannelError>;

    /// Every pending delivery, for recovery.
    async fn pending(&self) -> Result<Vec<PendingDelivery>, ChannelError>;
}

/// In-process store. Bindings and pending deliveries are lost on restart,
/// which is what a test or a short-lived process wants.
#[derive(Debug, Default)]
pub struct MemoryChannelStore {
    inner: Mutex<MemoryState>,
}

/// Platform retries arrive within minutes; older keys can go.
const MEMORY_SEEN_CAP: usize = 10_000;

#[derive(Debug, Default)]
struct MemoryState {
    bindings: HashMap<(String, String), String>,
    seen: HashSet<(String, String)>,
    seen_order: VecDeque<(String, String)>,
    pending: Vec<PendingDelivery>,
}

impl MemoryChannelStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, MemoryState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl ChannelStore for MemoryChannelStore {
    async fn session_for(&self, channel: &str, key: &str) -> Result<Option<String>, ChannelError> {
        Ok(self
            .state()
            .bindings
            .get(&(channel.to_string(), key.to_string()))
            .cloned())
    }

    async fn bind(&self, channel: &str, key: &str, session_id: &str) -> Result<(), ChannelError> {
        self.state().bindings.insert(
            (channel.to_string(), key.to_string()),
            session_id.to_string(),
        );
        Ok(())
    }

    async fn first_sighting(&self, channel: &str, dedup_key: &str) -> Result<bool, ChannelError> {
        let key = (channel.to_string(), dedup_key.to_string());
        let mut state = self.state();
        if !state.seen.insert(key.clone()) {
            return Ok(false);
        }
        state.seen_order.push_back(key);
        if state.seen_order.len() > MEMORY_SEEN_CAP
            && let Some(oldest) = state.seen_order.pop_front()
        {
            state.seen.remove(&oldest);
        }
        Ok(true)
    }

    async fn save_pending(&self, pending: &PendingDelivery) -> Result<(), ChannelError> {
        let mut state = self.state();
        state.pending.retain(|existing| {
            !(existing.session_id == pending.session_id
                && existing.input_message_id == pending.input_message_id)
        });
        state.pending.push(pending.clone());
        Ok(())
    }

    async fn clear_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
    ) -> Result<(), ChannelError> {
        self.state().pending.retain(|existing| {
            !(existing.session_id == session_id && existing.input_message_id == input_message_id)
        });
        Ok(())
    }

    async fn pending(&self) -> Result<Vec<PendingDelivery>, ChannelError> {
        Ok(self.state().pending.clone())
    }
}
