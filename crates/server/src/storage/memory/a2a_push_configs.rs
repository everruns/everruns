use anyhow::Result;
use everruns_contracts::typed_id::SessionId;

use super::InMemoryDatabase;
use crate::storage::{A2aPushConfigRow, UpsertA2aPushConfig};

impl InMemoryDatabase {
    pub async fn upsert_a2a_push_config(
        &self,
        input: UpsertA2aPushConfig,
    ) -> Result<A2aPushConfigRow> {
        let now = Self::now();
        let mut configs = self.a2a_push_configs.write();
        let existing = configs
            .iter()
            .position(|c| c.session_id == input.session_id && c.config_id == input.config_id);
        let row = A2aPushConfigRow {
            id: existing.map_or_else(uuid::Uuid::now_v7, |i| configs[i].id),
            org_id: input.org_id,
            session_id: input.session_id,
            config_id: input.config_id,
            url: input.url,
            auth_scheme: input.auth_scheme,
            secrets_encrypted: input.secrets_encrypted,
            wire_version: input.wire_version,
            created_at: existing.map_or(now, |i| configs[i].created_at),
            updated_at: now,
        };
        match existing {
            Some(i) => configs[i] = row.clone(),
            None => configs.push(row.clone()),
        }
        Ok(row)
    }

    pub async fn list_a2a_push_configs(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<A2aPushConfigRow>> {
        let mut rows: Vec<_> = self
            .a2a_push_configs
            .read()
            .iter()
            .filter(|c| c.session_id == session_id)
            .cloned()
            .collect();
        rows.sort_by_key(|c| (c.created_at, c.id));
        Ok(rows)
    }

    pub async fn delete_a2a_push_config(
        &self,
        session_id: SessionId,
        config_id: &str,
    ) -> Result<bool> {
        let mut configs = self.a2a_push_configs.write();
        let before = configs.len();
        configs.retain(|c| !(c.session_id == session_id && c.config_id == config_id));
        Ok(configs.len() < before)
    }
}
