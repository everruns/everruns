// In-memory storage: HTTP signing keys CRUD

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn upsert_http_signing_key(
        &self,
        org_id: i64,
        input: UpsertHttpSigningKey,
    ) -> Result<HttpSigningKeyRow> {
        let now = Self::now();
        let mut keys = self.http_signing_keys.write();

        // Check for existing key with same org_id + key_id
        let existing_id = keys
            .iter()
            .find(|(_, k)| k.org_id == org_id && k.key_id == input.key_id)
            .map(|(id, _)| *id);

        if let Some(id) = existing_id {
            let row = keys.get_mut(&id).unwrap();
            row.public_key = input.public_key;
            row.label = input.label;
            row.expires_at = input.expires_at;
            Ok(row.clone())
        } else {
            let id = Uuid::now_v7();
            let row = HttpSigningKeyRow {
                id,
                org_id,
                key_id: input.key_id,
                public_key: input.public_key,
                label: input.label,
                created_at: now,
                expires_at: input.expires_at,
            };
            keys.insert(id, row.clone());
            Ok(row)
        }
    }

    pub async fn list_http_signing_keys(&self, org_id: i64) -> Result<Vec<HttpSigningKeyRow>> {
        let now = Utc::now();
        let keys = self.http_signing_keys.read();
        let mut result: Vec<HttpSigningKeyRow> = keys
            .values()
            .filter(|k| k.org_id == org_id && k.expires_at.is_none_or(|exp| exp > now))
            .cloned()
            .collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result)
    }

    pub async fn delete_http_signing_key(&self, org_id: i64, key_id: &str) -> Result<bool> {
        let mut keys = self.http_signing_keys.write();
        let to_remove: Vec<Uuid> = keys
            .iter()
            .filter(|(_, k)| k.org_id == org_id && k.key_id == key_id)
            .map(|(id, _)| *id)
            .collect();
        let found = !to_remove.is_empty();
        for id in to_remove {
            keys.remove(&id);
        }
        Ok(found)
    }

    pub async fn list_all_http_signing_keys(&self) -> Result<Vec<HttpSigningKeyRow>> {
        let now = Utc::now();
        let keys = self.http_signing_keys.read();
        let mut result: Vec<HttpSigningKeyRow> = keys
            .values()
            .filter(|k| k.expires_at.is_none_or(|exp| exp > now))
            .cloned()
            .collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result)
    }
}
