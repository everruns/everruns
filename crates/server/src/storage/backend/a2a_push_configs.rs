use anyhow::Result;
use everruns_contracts::typed_id::SessionId;

use super::StorageBackend;
use crate::storage::{A2aPushConfigRow, UpsertA2aPushConfig};

impl StorageBackend {
    pub async fn upsert_a2a_push_config(
        &self,
        input: UpsertA2aPushConfig,
    ) -> Result<A2aPushConfigRow> {
        dispatch!(self, upsert_a2a_push_config, input)
    }

    pub async fn list_a2a_push_configs(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<A2aPushConfigRow>> {
        dispatch!(self, list_a2a_push_configs, session_id)
    }

    pub async fn delete_a2a_push_config(
        &self,
        session_id: SessionId,
        config_id: &str,
    ) -> Result<bool> {
        dispatch!(self, delete_a2a_push_config, session_id, config_id)
    }
}
