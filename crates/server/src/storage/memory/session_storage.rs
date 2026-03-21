// In-memory storage: Session Storage (Key-Value & Secrets), SessionStorageStore implementation for in-memory backend

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use everruns_core::SessionId;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Session Storage (Key-Value & Secrets)
    // ============================================

    pub async fn list_session_keys(&self, session_id: Uuid) -> Result<Vec<SessionKeyInfoRow>> {
        let session_id = SessionId::from_uuid(session_id);
        let storage = self.session_key_values.read();
        let mut keys: Vec<_> = storage
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|((_, _), row)| SessionKeyInfoRow {
                key: row.key.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();
        keys.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(keys)
    }

    pub async fn get_session_key_value(
        &self,
        session_id: Uuid,
        key: &str,
    ) -> Result<Option<SessionKeyValueRow>> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .session_key_values
            .read()
            .get(&(session_id, key.to_string()))
            .cloned())
    }

    pub async fn list_session_secrets(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionSecretInfoRow>> {
        let session_id = SessionId::from_uuid(session_id);
        let storage = self.session_secrets.read();
        let mut secrets: Vec<_> = storage
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|((_, _), row)| SessionSecretInfoRow {
                name: row.name.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();
        secrets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(secrets)
    }

    pub async fn upsert_session_secret(
        &self,
        input: UpsertSessionSecret,
    ) -> Result<SessionSecretRow> {
        let now = Self::now();
        let map_key = (input.session_id, input.name.clone());
        let mut storage = self.session_secrets.write();

        let row = if let Some(existing) = storage.get_mut(&map_key) {
            existing.value_encrypted = input.value_encrypted;
            existing.updated_at = now;
            existing.clone()
        } else {
            let row = SessionSecretRow {
                id: Uuid::now_v7(),
                session_id: input.session_id,
                name: input.name,
                value_encrypted: input.value_encrypted,
                created_at: now,
                updated_at: now,
            };
            storage.insert(map_key, row.clone());
            row
        };

        Ok(row)
    }
}

// ============================================================================
// SessionStorageStore implementation for in-memory backend
// ============================================================================

#[async_trait::async_trait]
impl everruns_core::traits::SessionStorageStore for InMemoryDatabase {
    async fn set_value(
        &self,
        session_id: SessionId,
        key: &str,
        value: &str,
    ) -> everruns_core::Result<()> {
        let now = Self::now();
        let mut storage = self.session_key_values.write();
        let map_key = (session_id, key.to_string());
        storage
            .entry(map_key)
            .and_modify(|row| {
                row.value = value.to_string();
                row.updated_at = now;
            })
            .or_insert_with(|| SessionKeyValueRow {
                id: Uuid::now_v7(),
                session_id,
                key: key.to_string(),
                value: value.to_string(),
                created_at: now,
                updated_at: now,
            });
        Ok(())
    }

    async fn get_value(
        &self,
        session_id: SessionId,
        key: &str,
    ) -> everruns_core::Result<Option<String>> {
        let storage = self.session_key_values.read();
        Ok(storage
            .get(&(session_id, key.to_string()))
            .map(|r| r.value.clone()))
    }

    async fn delete_value(&self, session_id: SessionId, key: &str) -> everruns_core::Result<bool> {
        let mut storage = self.session_key_values.write();
        Ok(storage.remove(&(session_id, key.to_string())).is_some())
    }

    async fn list_keys(
        &self,
        session_id: SessionId,
    ) -> everruns_core::Result<Vec<everruns_core::KeyInfo>> {
        let storage = self.session_key_values.read();
        let mut keys: Vec<_> = storage
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|(_, row)| everruns_core::KeyInfo {
                key: row.key.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();
        keys.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(keys)
    }

    async fn set_secret(
        &self,
        session_id: SessionId,
        name: &str,
        value: &str,
    ) -> everruns_core::Result<()> {
        let now = Self::now();
        let mut storage = self.session_secrets.write();
        let map_key = (session_id, name.to_string());
        // In-memory: store plain text as bytes (no encryption in dev mode)
        storage
            .entry(map_key)
            .and_modify(|row| {
                row.value_encrypted = value.as_bytes().to_vec();
                row.updated_at = now;
            })
            .or_insert_with(|| SessionSecretRow {
                id: Uuid::now_v7(),
                session_id,
                name: name.to_string(),
                value_encrypted: value.as_bytes().to_vec(),
                created_at: now,
                updated_at: now,
            });
        Ok(())
    }

    async fn get_secret(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> everruns_core::Result<Option<String>> {
        let storage = self.session_secrets.read();
        Ok(storage
            .get(&(session_id, name.to_string()))
            .map(|r| String::from_utf8_lossy(&r.value_encrypted).to_string()))
    }

    async fn delete_secret(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> everruns_core::Result<bool> {
        let mut storage = self.session_secrets.write();
        Ok(storage.remove(&(session_id, name.to_string())).is_some())
    }

    async fn list_secrets(
        &self,
        session_id: SessionId,
    ) -> everruns_core::Result<Vec<everruns_core::SecretInfo>> {
        let storage = self.session_secrets.read();
        let mut secrets: Vec<_> = storage
            .iter()
            .filter(|((sid, _), _)| *sid == session_id)
            .map(|(_, row)| everruns_core::SecretInfo {
                name: row.name.clone(),
                created_at: row.created_at,
                updated_at: row.updated_at,
            })
            .collect();
        secrets.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(secrets)
    }
}
