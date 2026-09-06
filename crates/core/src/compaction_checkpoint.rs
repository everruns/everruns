//! Durable replacement context for compacted model history.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use tokio::sync::Mutex;
#[cfg(test)]
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::ProviderOpaqueContext;
use crate::error::Result;
use crate::typed_id::SessionId;

pub const COMPACTION_CHECKPOINT_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompactionCheckpointPayload {
    ProviderOpaque { context: ProviderOpaqueContext },
    Summary { text: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionCheckpoint {
    pub id: Uuid,
    pub session_id: SessionId,
    pub source_sequence: i64,
    pub provider_type: String,
    pub model: String,
    pub format_version: u32,
    pub payload: CompactionCheckpointPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProactiveCompactionAttempt {
    pub source_sequence: i64,
    pub estimated_input_tokens: u64,
    pub input_message_count: usize,
    pub source_fingerprint: [u8; 32],
}

impl CompactionCheckpoint {
    pub fn is_compatible(&self, provider_type: &str, model: &str) -> bool {
        self.format_version == COMPACTION_CHECKPOINT_FORMAT_VERSION
            && self.provider_type == provider_type
            && self.model == model
    }
}

#[async_trait]
pub trait CompactionCheckpointStore: Send + Sync {
    async fn get_latest(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<CompactionCheckpoint>>;

    /// Install a checkpoint only if no newer source boundary is canonical.
    async fn install(&self, checkpoint: CompactionCheckpoint) -> Result<bool>;

    /// Latest pressure snapshot where proactive native compaction was attempted.
    ///
    /// This process-lifetime watermark prevents an ineffective or failed
    /// provider call from being repeated on every reasoning iteration. The
    /// source fingerprint makes it lineage-local across rollback/branch
    /// selection. Durable checkpoints remain the semantic replacement
    /// contract; this marker only controls retry pressure and never substitutes
    /// for a checkpoint.
    async fn get_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<ProactiveCompactionAttempt>>;

    async fn record_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) -> Result<()>;
}

#[cfg(test)]
type CheckpointKey = (SessionId, String, String, u32);
type AttemptKey = (SessionId, String, String);

const DEFAULT_PROACTIVE_ATTEMPT_CAPACITY: usize = 4_096;

/// Bounded process-local retry watermarks shared by successive reason atoms.
/// Oldest session/provider/model entries are evicted once capacity is reached.
#[derive(Debug)]
pub struct ProactiveCompactionAttemptTracker {
    capacity: usize,
    state: Mutex<ProactiveAttemptState>,
}

#[derive(Debug, Default)]
struct ProactiveAttemptState {
    attempts: HashMap<AttemptKey, ProactiveCompactionAttempt>,
    insertion_order: VecDeque<AttemptKey>,
}

impl Default for ProactiveCompactionAttemptTracker {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_PROACTIVE_ATTEMPT_CAPACITY,
            state: Mutex::new(ProactiveAttemptState::default()),
        }
    }
}

impl ProactiveCompactionAttemptTracker {
    pub async fn get(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Option<ProactiveCompactionAttempt> {
        self.state
            .lock()
            .await
            .attempts
            .get(&(session_id, provider_type.to_string(), model.to_string()))
            .copied()
    }

    pub async fn record(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) {
        let key = (session_id, provider_type.to_string(), model.to_string());
        let mut state = self.state.lock().await;
        if let Some(current) = state.attempts.get_mut(&key) {
            if attempt.source_sequence >= current.source_sequence {
                *current = attempt;
            }
            return;
        }
        while state.attempts.len() >= self.capacity {
            if let Some(oldest) = state.insertion_order.pop_front() {
                state.attempts.remove(&oldest);
            }
        }
        state.insertion_order.push_back(key.clone());
        state.attempts.insert(key, attempt);
    }
}

/// In-memory implementation used by collocated unit tests.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct InMemoryCompactionCheckpointStore {
    checkpoints: RwLock<HashMap<CheckpointKey, CompactionCheckpoint>>,
    proactive_attempts: ProactiveCompactionAttemptTracker,
}

#[cfg(test)]
#[async_trait]
impl CompactionCheckpointStore for InMemoryCompactionCheckpointStore {
    async fn get_latest(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<CompactionCheckpoint>> {
        Ok(self
            .checkpoints
            .read()
            .await
            .get(&(
                session_id,
                provider_type.to_string(),
                model.to_string(),
                COMPACTION_CHECKPOINT_FORMAT_VERSION,
            ))
            .cloned())
    }

    async fn install(&self, checkpoint: CompactionCheckpoint) -> Result<bool> {
        let key = (
            checkpoint.session_id,
            checkpoint.provider_type.clone(),
            checkpoint.model.clone(),
            checkpoint.format_version,
        );
        let mut checkpoints = self.checkpoints.write().await;
        if checkpoints
            .get(&key)
            .is_some_and(|current| current.source_sequence >= checkpoint.source_sequence)
        {
            return Ok(false);
        }
        checkpoints.insert(key, checkpoint);
        Ok(true)
    }

    async fn get_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> Result<Option<ProactiveCompactionAttempt>> {
        Ok(self
            .proactive_attempts
            .get(session_id, provider_type, model)
            .await)
    }

    async fn record_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) -> Result<()> {
        self.proactive_attempts
            .record(session_id, provider_type, model, attempt)
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CompactOutputItem, ProviderOpaqueContext};

    fn checkpoint(session_id: SessionId, source_sequence: i64) -> CompactionCheckpoint {
        CompactionCheckpoint {
            id: Uuid::now_v7(),
            session_id,
            source_sequence,
            provider_type: "openai".to_string(),
            model: "gpt-5.4".to_string(),
            format_version: 1,
            payload: CompactionCheckpointPayload::ProviderOpaque {
                context: ProviderOpaqueContext::OpenResponsesCompact {
                    output: vec![CompactOutputItem::Compaction {
                        encrypted_content: format!("opaque-{source_sequence}"),
                    }],
                },
            },
        }
    }

    #[tokio::test]
    async fn install_is_monotonic_and_failed_cas_leaves_current_checkpoint_unchanged() {
        let store = InMemoryCompactionCheckpointStore::default();
        let session_id = SessionId::new();
        let current = checkpoint(session_id, 12);
        assert!(store.install(current.clone()).await.unwrap());

        assert!(!store.install(checkpoint(session_id, 11)).await.unwrap());
        assert!(!store.install(checkpoint(session_id, 12)).await.unwrap());
        assert_eq!(
            store
                .get_latest(session_id, "openai", "gpt-5.4")
                .await
                .unwrap(),
            Some(current)
        );
    }

    #[test]
    fn compatibility_requires_exact_provider_model_and_format() {
        let mut checkpoint = checkpoint(SessionId::new(), 1);
        assert!(checkpoint.is_compatible("openai", "gpt-5.4"));
        assert!(!checkpoint.is_compatible("openrouter", "gpt-5.4"));
        assert!(!checkpoint.is_compatible("openai", "gpt-5.5"));
        checkpoint.format_version += 1;
        assert!(!checkpoint.is_compatible("openai", "gpt-5.4"));
    }

    #[tokio::test]
    async fn proactive_attempt_updates_preserve_monotonicity_and_fifo_eviction() {
        let tracker = ProactiveCompactionAttemptTracker {
            capacity: 2,
            state: Mutex::default(),
        };
        let session = SessionId::from_seed(1);
        let original = ProactiveCompactionAttempt {
            source_sequence: 2,
            estimated_input_tokens: 10_000,
            input_message_count: 1,
            source_fingerprint: [7; 32],
        };
        let newer = ProactiveCompactionAttempt {
            source_sequence: 3,
            estimated_input_tokens: 20_000,
            input_message_count: 2,
            source_fingerprint: [8; 32],
        };
        assert_eq!(tracker.get(session, "openai", "a").await, None);
        tracker.record(session, "openai", "a", original).await;
        tracker.record(session, "openai", "b", original).await;
        tracker.record(session, "openai", "a", newer).await;
        tracker.record(session, "openai", "a", original).await;
        assert_eq!(tracker.get(session, "openai", "a").await, Some(newer));
        let equal = ProactiveCompactionAttempt {
            estimated_input_tokens: 30_000,
            source_fingerprint: [9; 32],
            ..newer
        };
        tracker.record(session, "openai", "a", equal).await;
        assert_eq!(tracker.get(session, "openai", "a").await, Some(equal));
        tracker.record(session, "other", "a", original).await;
        assert_eq!(tracker.get(session, "openai", "a").await, None);
        assert_eq!(tracker.get(session, "openai", "b").await, Some(original));
        assert_eq!(tracker.get(session, "other", "a").await, Some(original));
        tracker
            .record(SessionId::from_seed(2), "other", "a", newer)
            .await;
        assert_eq!(tracker.get(session, "openai", "b").await, None);
        assert_eq!(tracker.get(session, "other", "a").await, Some(original));
        assert_eq!(
            tracker.get(SessionId::from_seed(2), "other", "a").await,
            Some(newer)
        );
        let state = tracker.state.lock().await;
        assert_eq!(state.attempts.len(), 2);
        assert_eq!(state.insertion_order.len(), 2);
    }
}
