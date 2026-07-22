//! Durable replacement context for compacted model history.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
}

type CheckpointKey = (SessionId, String, String, u32);

/// In-memory implementation used by embedded runtimes and entry-point tests.
#[derive(Debug, Default)]
pub struct InMemoryCompactionCheckpointStore {
    checkpoints: RwLock<HashMap<CheckpointKey, CompactionCheckpoint>>,
}

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
            format_version: COMPACTION_CHECKPOINT_FORMAT_VERSION,
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
}
