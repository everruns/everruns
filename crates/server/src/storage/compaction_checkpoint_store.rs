use crate::kernel_imports::{
    COMPACTION_CHECKPOINT_FORMAT_VERSION, CompactionCheckpoint, CompactionCheckpointPayload,
    CompactionCheckpointStore, ProactiveCompactionAttempt, ProactiveCompactionAttemptTracker,
    everruns_provider::error::AgentLoopError, everruns_provider::typed_id::SessionId,
};
use async_trait::async_trait;
use std::sync::Arc;

use super::{EncryptionService, InstallCompactionCheckpointRow, StorageBackend};

const MAX_CHECKPOINT_PAYLOAD_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct DbCompactionCheckpointStore {
    db: Arc<StorageBackend>,
    encryption: Arc<EncryptionService>,
    proactive_attempts: Arc<ProactiveCompactionAttemptTracker>,
}

impl DbCompactionCheckpointStore {
    pub fn new(db: Arc<StorageBackend>, encryption: Arc<EncryptionService>) -> Self {
        Self::with_proactive_attempt_tracker(
            db,
            encryption,
            Arc::new(ProactiveCompactionAttemptTracker::default()),
        )
    }

    pub fn with_proactive_attempt_tracker(
        db: Arc<StorageBackend>,
        encryption: Arc<EncryptionService>,
        proactive_attempts: Arc<ProactiveCompactionAttemptTracker>,
    ) -> Self {
        Self {
            db,
            encryption,
            proactive_attempts,
        }
    }
}

#[async_trait]
impl CompactionCheckpointStore for DbCompactionCheckpointStore {
    async fn get_latest(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> everruns_provider::error::Result<Option<CompactionCheckpoint>> {
        let row = self
            .db
            .get_compaction_checkpoint(
                session_id,
                provider_type,
                model,
                COMPACTION_CHECKPOINT_FORMAT_VERSION as i32,
            )
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let plaintext = self
            .encryption
            .decrypt(&row.payload_encrypted)
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        let payload: CompactionCheckpointPayload = serde_json::from_slice(&plaintext)
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        Ok(Some(CompactionCheckpoint {
            id: row.id,
            session_id: row.session_id,
            source_sequence: i64::from(row.source_sequence),
            provider_type: row.provider_type,
            model: row.model,
            format_version: row.format_version as u32,
            payload,
        }))
    }

    async fn install(
        &self,
        checkpoint: CompactionCheckpoint,
    ) -> everruns_provider::error::Result<bool> {
        let source_sequence = i32::try_from(checkpoint.source_sequence).map_err(|_| {
            AgentLoopError::store("checkpoint source sequence exceeds storage range")
        })?;
        let plaintext = serde_json::to_vec(&checkpoint.payload)
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        if plaintext.len() > MAX_CHECKPOINT_PAYLOAD_BYTES {
            return Err(AgentLoopError::store(
                "compaction checkpoint exceeds 32 MiB",
            ));
        }
        let payload_encrypted = self
            .encryption
            .encrypt(&plaintext)
            .map_err(|error| AgentLoopError::store(error.to_string()))?;
        self.db
            .install_compaction_checkpoint(InstallCompactionCheckpointRow {
                id: checkpoint.id,
                session_id: checkpoint.session_id,
                source_sequence,
                provider_type: checkpoint.provider_type,
                model: checkpoint.model,
                format_version: checkpoint.format_version as i32,
                payload_encrypted,
            })
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))
    }

    async fn get_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> everruns_provider::error::Result<Option<ProactiveCompactionAttempt>> {
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
    ) -> everruns_provider::error::Result<()> {
        self.proactive_attempts
            .record(session_id, provider_type, model, attempt)
            .await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::{CompactOutputItem, ProviderOpaqueContext};
    use uuid::Uuid;

    fn encryption() -> Arc<EncryptionService> {
        Arc::new(
            EncryptionService::new("test:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
                .unwrap(),
        )
    }

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
                        encrypted_content: "provider-secret-blob".to_string(),
                    }],
                },
            },
        }
    }

    #[tokio::test]
    async fn encrypted_checkpoint_round_trips_and_forks_at_source_boundary() {
        let db = Arc::new(StorageBackend::in_memory());
        let store = DbCompactionCheckpointStore::new(db.clone(), encryption());
        let source = SessionId::new();
        let child = SessionId::new();

        assert!(store.install(checkpoint(source, 7)).await.unwrap());
        assert!(!store.install(checkpoint(source, 6)).await.unwrap());
        let raw = db
            .get_compaction_checkpoint(
                source,
                "openai",
                "gpt-5.4",
                COMPACTION_CHECKPOINT_FORMAT_VERSION as i32,
            )
            .await
            .unwrap()
            .unwrap();
        assert!(!String::from_utf8_lossy(&raw.payload_encrypted).contains("provider-secret-blob"));

        assert_eq!(
            db.copy_compaction_checkpoints(source, child, 6)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.copy_compaction_checkpoints(source, child, 7)
                .await
                .unwrap(),
            1
        );
        let forked = store
            .get_latest(child, "openai", "gpt-5.4")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(forked.source_sequence, 7);
        assert_eq!(forked.payload, checkpoint(source, 7).payload);
    }
}
