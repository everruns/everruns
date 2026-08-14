use anyhow::Result;
use everruns_provider::typed_id::SessionId;
use uuid::Uuid;

use super::InMemoryDatabase;
use crate::storage::models::{CompactionCheckpointRow, InstallCompactionCheckpointRow};

impl InMemoryDatabase {
    pub async fn get_compaction_checkpoint(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        format_version: i32,
    ) -> Result<Option<CompactionCheckpointRow>> {
        Ok(self
            .compaction_checkpoints
            .read()
            .get(&(
                session_id,
                provider_type.to_string(),
                model.to_string(),
                format_version,
            ))
            .cloned())
    }

    pub async fn install_compaction_checkpoint(
        &self,
        input: InstallCompactionCheckpointRow,
    ) -> Result<bool> {
        let key = (
            input.session_id,
            input.provider_type.clone(),
            input.model.clone(),
            input.format_version,
        );
        let mut checkpoints = self.compaction_checkpoints.write();
        if checkpoints
            .get(&key)
            .is_some_and(|current| current.source_sequence >= input.source_sequence)
        {
            return Ok(false);
        }
        checkpoints.insert(
            key,
            CompactionCheckpointRow {
                id: input.id,
                session_id: input.session_id,
                source_sequence: input.source_sequence,
                provider_type: input.provider_type,
                model: input.model,
                format_version: input.format_version,
                payload_encrypted: input.payload_encrypted,
            },
        );
        Ok(true)
    }

    pub async fn copy_compaction_checkpoints(
        &self,
        source_session_id: SessionId,
        target_session_id: SessionId,
        through_sequence: i32,
    ) -> Result<u64> {
        let source: Vec<_> = self
            .compaction_checkpoints
            .read()
            .values()
            .filter(|row| {
                row.session_id == source_session_id && row.source_sequence <= through_sequence
            })
            .cloned()
            .collect();
        let mut checkpoints = self.compaction_checkpoints.write();
        let mut copied = 0;
        for mut row in source {
            row.id = Uuid::now_v7();
            row.session_id = target_session_id;
            let key = (
                target_session_id,
                row.provider_type.clone(),
                row.model.clone(),
                row.format_version,
            );
            if let std::collections::hash_map::Entry::Vacant(entry) = checkpoints.entry(key) {
                entry.insert(row);
                copied += 1;
            }
        }
        Ok(copied)
    }
}
