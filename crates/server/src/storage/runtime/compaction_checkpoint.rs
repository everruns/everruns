use crate::kernel_imports::{
    COMPACTION_CHECKPOINT_FORMAT_VERSION, CompactionCheckpoint, CompactionCheckpointPayload,
    CompactionCheckpointStore, ProactiveCompactionAttempt, ProactiveCompactionAttemptTracker,
    contracts::error::AgentLoopError, contracts::typed_id::SessionId,
};
use async_trait::async_trait;
use std::sync::Arc;

use crate::storage::{EncryptionService, InstallCompactionCheckpointRow, StorageBackend};

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
    async fn latest_source_sequence(
        &self,
        session_id: SessionId,
    ) -> everruns_contracts::error::Result<Option<i64>> {
        self.db
            .latest_compaction_source_sequence(session_id)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))
    }

    async fn get_latest(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> everruns_contracts::error::Result<Option<CompactionCheckpoint>> {
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

    async fn get_latest_format(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        format_version: u32,
    ) -> everruns_contracts::error::Result<Option<CompactionCheckpoint>> {
        let row = self
            .db
            .get_compaction_checkpoint(session_id, provider_type, model, format_version as i32)
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
    ) -> everruns_contracts::error::Result<bool> {
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
    ) -> everruns_contracts::error::Result<Option<ProactiveCompactionAttempt>> {
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
    ) -> everruns_contracts::error::Result<()> {
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
                    reasoning_state: None,
                    output: vec![CompactOutputItem::Compaction {
                        encrypted_content: "provider-secret-blob".to_string(),
                    }],
                },
            },
        }
    }

    #[tokio::test]
    async fn encrypted_checkpoint_round_trips_and_forks_at_source_boundary() {
        let db = Arc::new(StorageBackend::test_database());
        let store = DbCompactionCheckpointStore::new(db.clone(), encryption());
        let source = db.create_test_session().await;
        let child = db.create_test_session().await;

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

    #[tokio::test]
    async fn portable_candidates_are_encrypted_and_do_not_advance_the_history_floor() {
        use everruns_core::compaction_checkpoint::{
            PORTABLE_COMPACTION_FORMAT_VERSION as FORMAT, PORTABLE_COMPACTION_MODEL as MODEL,
            PORTABLE_COMPACTION_PROVIDER as PROVIDER, PortableContinuity,
        };
        let db = Arc::new(StorageBackend::test_database());
        let store = DbCompactionCheckpointStore::new(db.clone(), encryption());
        let session = db.create_test_session().await;
        let child = db.create_test_session().await;
        let mut canonical = checkpoint(session, 7);
        canonical.provider_type = PROVIDER.into();
        canonical.model = MODEL.into();
        canonical.format_version = FORMAT;
        canonical.payload = CompactionCheckpointPayload::Portable {
            continuity: PortableContinuity {
                text: "private continuity state".into(),
                pinned: vec!["verbatim skill".into()],
                producer_provider: "openrouter".into(),
                producer_model: "model-a".into(),
                source_fingerprint: [7; 32],
                prompt_version: 1,
                usage: serde_json::json!({}),
            },
        };
        let mut candidate = canonical.clone();
        candidate.id = Uuid::now_v7();
        candidate.model = "continuity-candidate-v1".into();
        assert!(store.install(candidate.clone()).await.unwrap());
        assert_eq!(store.latest_source_sequence(session).await.unwrap(), None);
        assert!(store.install(canonical.clone()).await.unwrap());
        assert_eq!(
            store.latest_source_sequence(session).await.unwrap(),
            Some(7)
        );
        let restored = store
            .get_latest_format(session, PROVIDER, MODEL, FORMAT)
            .await
            .unwrap()
            .unwrap();
        assert!(restored.is_portable());
        assert_eq!(restored.payload, canonical.payload);
        assert_ne!(restored.id, candidate.id);
        let raw = db
            .get_compaction_checkpoint(session, PROVIDER, MODEL, FORMAT as i32)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&raw.payload_encrypted).contains("private continuity state")
        );
        assert_eq!(
            db.copy_compaction_checkpoints(session, child, 6)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.copy_compaction_checkpoints(session, child, 7)
                .await
                .unwrap(),
            2
        );
        assert!(
            store
                .get_latest_format(child, PROVIDER, MODEL, FORMAT)
                .await
                .unwrap()
                .unwrap()
                .is_portable()
        );
    }

    #[tokio::test]
    async fn oversized_anthropic_prefix_is_rejected_before_storage() {
        let db = Arc::new(StorageBackend::test_database());
        let session_id = db.create_test_session().await;
        let store = DbCompactionCheckpointStore::new(db, encryption());
        let checkpoint = CompactionCheckpoint {
            id: Uuid::now_v7(),
            session_id,
            source_sequence: 8,
            provider_type: "anthropic".to_string(),
            model: "claude-opus-4-8".to_string(),
            format_version: 2,
            payload: CompactionCheckpointPayload::ProviderOpaque {
                context: ProviderOpaqueContext::AnthropicMessagesPrefix {
                    messages_json: "x".repeat(MAX_CHECKPOINT_PAYLOAD_BYTES),
                },
            },
        };

        let error = store.install(checkpoint).await.unwrap_err();
        assert!(error.to_string().contains("exceeds 32 MiB"));
        assert!(
            store
                .get_latest_format(session_id, "anthropic", "claude-opus-4-8", 2)
                .await
                .unwrap()
                .is_none()
        );
    }
}
