//! What a channel host persists.
//!
//! Four things, and nothing about sessions themselves: which session a
//! binding key maps to, which platform deliveries were already accepted (a
//! platform retries a slow acknowledgement, and the retry must not start a
//! second turn), which turns still owe a reply (so a restart can finish
//! them), and which platform message is which stored message (so a reaction
//! on the platform finds its message, and the agent's reaction finds the
//! platform's). Memory, SQLite and Postgres implement the same trait, as the durable
//! stores do.
//!
//! Decisions:
//! - `channel` is the channel's stable name: a host whose channels are rows
//!   uses the row's id, so keys never cross tenants.
//! - Binding is first-writer-wins. Two first messages of one thread can both
//!   create a session; the store keeps one and the host discards the other.
//! - Pending deliveries are claimed with a lease, so several processes sharing
//!   a store recover each delivery once. A single-process store may ignore the
//!   lease.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{ChannelError, DeliveryCheckpoint, DeliveryTarget};

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
    /// Whether this backend can contain output ledgers. Stores implementing the
    /// checkpoint methods must return true, including when a legacy driver is
    /// active, so a rollback cannot ignore previously persisted output.
    fn supports_delivery_checkpoints(&self) -> bool {
        false
    }

    /// Load the output ledger, separately from credentials and routing configuration.
    async fn delivery_checkpoint(
        &self,
        _session_id: &str,
        _input_message_id: &str,
    ) -> Result<Option<DeliveryCheckpoint>, ChannelError> {
        Err(ChannelError::Session(
            "channel store does not implement output checkpoints".into(),
        ))
    }

    /// Atomically fence by the current, unexpired delivery lease before every write.
    async fn save_delivery_checkpoint(
        &self,
        _session_id: &str,
        _input_message_id: &str,
        _holder: &str,
        _checkpoint: &DeliveryCheckpoint,
    ) -> Result<(), ChannelError> {
        Err(ChannelError::Session(
            "channel store does not implement output checkpoints".into(),
        ))
    }

    /// A stale holder must not clear a successor's pending delivery and output ledger.
    async fn complete_delivery(
        &self,
        session_id: &str,
        input_message_id: &str,
        _holder: &str,
    ) -> Result<(), ChannelError> {
        let _ = (session_id, input_message_id);
        Err(ChannelError::Session(
            "channel store does not implement fenced delivery completion".into(),
        ))
    }
    /// Start delivery only after its durable state is committed. Transactional
    /// stores override this to discard the task on rollback.
    fn spawn_delivery(&self, task: futures::future::BoxFuture<'static, ()>) {
        tokio::spawn(task);
    }

    /// The session bound to `key` on `channel`.
    async fn session_for(&self, channel: &str, key: &str) -> Result<Option<String>, ChannelError>;

    /// Bind `key` on `channel` to `session_id` unless it is bound already.
    /// Returns the session the key is bound to afterwards: `session_id`, or
    /// the one an earlier writer bound.
    async fn bind(
        &self,
        channel: &str,
        key: &str,
        session_id: &str,
    ) -> Result<String, ChannelError>;

    /// Record a platform delivery key. `true` the first time, `false` for a
    /// retry of a delivery already accepted.
    async fn first_sighting(&self, channel: &str, dedup_key: &str) -> Result<bool, ChannelError>;

    /// Insert a pending delivery, held by `holder` for `lease`. Existing
    /// deliveries retain their owner and checkpoint; takeover uses `claim_pending`.
    async fn save_pending(
        &self,
        pending: &PendingDelivery,
        holder: &str,
        lease: Duration,
    ) -> Result<(), ChannelError>;

    /// Extend `holder`'s lease on a pending delivery it is running.
    async fn renew_pending(
        &self,
        _session_id: &str,
        _input_message_id: &str,
        _holder: &str,
        _lease: Duration,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Clear a pending delivery once its turn ended.
    async fn clear_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
    ) -> Result<(), ChannelError>;

    /// Claim every pending delivery no live holder has, for recovery: each
    /// one is held by `holder` for `lease` afterwards.
    async fn claim_pending(
        &self,
        holder: &str,
        lease: Duration,
    ) -> Result<Vec<PendingDelivery>, ChannelError>;

    /// Give a claimed delivery back without finishing it (its channel is
    /// unavailable for now), so a later recovery can take it.
    async fn release_pending(
        &self,
        _session_id: &str,
        _input_message_id: &str,
        _holder: &str,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Link a platform message key to the stored message it is. The first
    /// link of a key wins. The default keeps no links, so reactions stay
    /// inside the host.
    async fn link_message(
        &self,
        _channel: &str,
        _message_key: &str,
        _message: &LinkedMessage,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// The stored message a platform message key was linked to.
    async fn linked_message(
        &self,
        _channel: &str,
        _message_key: &str,
    ) -> Result<Option<LinkedMessage>, ChannelError> {
        Ok(None)
    }

    /// The platform key a stored message was linked under.
    async fn message_key(
        &self,
        _channel: &str,
        _message: &LinkedMessage,
    ) -> Result<Option<String>, ChannelError> {
        Ok(None)
    }
}

/// A stored message, as a platform message key links to it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LinkedMessage {
    pub session_id: String,
    pub message_id: String,
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
    checkpoints: HashMap<(String, String), DeliveryCheckpoint>,
    holders: HashMap<(String, String), (String, web_time::Instant)>,
    links: HashMap<(String, String), LinkedMessage>,
}

impl MemoryChannelStore {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(super) async fn pending_for_test(&self) -> Result<Vec<PendingDelivery>, ChannelError> {
        Ok(self.state().pending.clone())
    }

    #[cfg(test)]
    pub(super) fn expire_leases_for_test(&self) {
        for (_, until) in self.state().holders.values_mut() {
            *until = web_time::Instant::now();
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, MemoryState> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl ChannelStore for MemoryChannelStore {
    fn supports_delivery_checkpoints(&self) -> bool {
        true
    }

    async fn delivery_checkpoint(
        &self,
        session_id: &str,
        input_message_id: &str,
    ) -> Result<Option<DeliveryCheckpoint>, ChannelError> {
        Ok(self
            .state()
            .checkpoints
            .get(&(session_id.into(), input_message_id.into()))
            .cloned())
    }

    async fn save_delivery_checkpoint(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
        checkpoint: &DeliveryCheckpoint,
    ) -> Result<(), ChannelError> {
        let mut state = self.state();
        let key = (session_id.into(), input_message_id.into());
        if !state
            .holders
            .get(&key)
            .is_some_and(|(owner, until)| owner == holder && *until > web_time::Instant::now())
        {
            return Err(ChannelError::Session(
                "delivery lease is no longer held".into(),
            ));
        }
        state.checkpoints.insert(key, checkpoint.clone());
        Ok(())
    }

    async fn complete_delivery(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
    ) -> Result<(), ChannelError> {
        let mut state = self.state();
        let key = (session_id.into(), input_message_id.into());
        if !state
            .holders
            .get(&key)
            .is_some_and(|(owner, until)| owner == holder && *until > web_time::Instant::now())
        {
            return Err(ChannelError::Session(
                "delivery lease is no longer held".into(),
            ));
        }
        state.pending.retain(|pending| {
            pending.session_id != session_id || pending.input_message_id != input_message_id
        });
        state.holders.remove(&key);
        state.checkpoints.remove(&key);
        Ok(())
    }
    async fn session_for(&self, channel: &str, key: &str) -> Result<Option<String>, ChannelError> {
        Ok(self
            .state()
            .bindings
            .get(&(channel.to_string(), key.to_string()))
            .cloned())
    }

    async fn bind(
        &self,
        channel: &str,
        key: &str,
        session_id: &str,
    ) -> Result<String, ChannelError> {
        Ok(self
            .state()
            .bindings
            .entry((channel.to_string(), key.to_string()))
            .or_insert_with(|| session_id.to_string())
            .clone())
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

    // The in-process store also fences independent hosts sharing this instance.
    async fn save_pending(
        &self,
        pending: &PendingDelivery,
        holder: &str,
        lease: Duration,
    ) -> Result<(), ChannelError> {
        let mut state = self.state();
        if state.pending.iter().any(|existing| {
            existing.session_id == pending.session_id
                && existing.input_message_id == pending.input_message_id
        }) {
            return Ok(());
        }
        state.pending.push(pending.clone());
        state.holders.insert(
            (pending.session_id.clone(), pending.input_message_id.clone()),
            (holder.into(), web_time::Instant::now() + lease),
        );
        Ok(())
    }

    async fn renew_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
        lease: Duration,
    ) -> Result<(), ChannelError> {
        let mut state = self.state();
        let key = (session_id.into(), input_message_id.into());
        let Some((owner, until)) = state.holders.get_mut(&key) else {
            return Err(ChannelError::Session("delivery lease is absent".into()));
        };
        if owner != holder || *until <= web_time::Instant::now() {
            return Err(ChannelError::Session(
                "delivery lease is no longer held".into(),
            ));
        }
        *until = web_time::Instant::now() + lease;
        Ok(())
    }

    async fn clear_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
    ) -> Result<(), ChannelError> {
        let mut state = self.state();
        state.pending.retain(|existing| {
            !(existing.session_id == session_id && existing.input_message_id == input_message_id)
        });
        let key = (session_id.into(), input_message_id.into());
        state.holders.remove(&key);
        state.checkpoints.remove(&key);
        Ok(())
    }

    async fn claim_pending(
        &self,
        holder: &str,
        lease: Duration,
    ) -> Result<Vec<PendingDelivery>, ChannelError> {
        let mut state = self.state();
        let now = web_time::Instant::now();
        let pending = state
            .pending
            .iter()
            .filter(|pending| {
                state
                    .holders
                    .get(&(pending.session_id.clone(), pending.input_message_id.clone()))
                    .is_none_or(|(_, until)| *until <= now)
            })
            .cloned()
            .collect::<Vec<_>>();
        for pending in &pending {
            state.holders.insert(
                (pending.session_id.clone(), pending.input_message_id.clone()),
                (holder.into(), now + lease),
            );
        }
        Ok(pending)
    }

    async fn release_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
    ) -> Result<(), ChannelError> {
        let mut state = self.state();
        if let Some((owner, until)) = state
            .holders
            .get_mut(&(session_id.into(), input_message_id.into()))
            && owner == holder
        {
            *until = web_time::Instant::now();
        }
        Ok(())
    }

    async fn link_message(
        &self,
        channel: &str,
        message_key: &str,
        message: &LinkedMessage,
    ) -> Result<(), ChannelError> {
        self.state()
            .links
            .entry((channel.to_string(), message_key.to_string()))
            .or_insert_with(|| message.clone());
        Ok(())
    }

    async fn linked_message(
        &self,
        channel: &str,
        message_key: &str,
    ) -> Result<Option<LinkedMessage>, ChannelError> {
        Ok(self
            .state()
            .links
            .get(&(channel.to_string(), message_key.to_string()))
            .cloned())
    }

    async fn message_key(
        &self,
        channel: &str,
        message: &LinkedMessage,
    ) -> Result<Option<String>, ChannelError> {
        Ok(self
            .state()
            .links
            .iter()
            .find(|((linked_channel, _), linked)| linked_channel == channel && *linked == message)
            .map(|((_, key), _)| key.clone()))
    }
}
