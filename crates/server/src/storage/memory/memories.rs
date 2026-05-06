// In-memory storage: org-scoped memory stores and memories.

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::{Result, bail};
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Memory Stores
    // ============================================

    pub async fn create_memory_store(
        &self,
        org_id: i64,
        input: CreateMemoryStoreRow,
    ) -> Result<MemoryStoreDbRow> {
        let now = Self::now();
        let mut stores = self.memory_stores.write();

        if stores
            .values()
            .any(|s| s.org_id == org_id && s.name.eq_ignore_ascii_case(&input.name))
        {
            bail!("memory store name already exists");
        }
        if input.is_default && stores.values().any(|s| s.org_id == org_id && s.is_default) {
            bail!("org already has a default memory store");
        }

        let id = Uuid::now_v7();
        let row = MemoryStoreDbRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            is_default: input.is_default,
            created_at: now,
            updated_at: now,
        };
        stores.insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_memory_store_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<MemoryStoreDbRow>> {
        Ok(self
            .memory_stores
            .read()
            .values()
            .find(|s| s.org_id == org_id && s.public_id == public_id)
            .cloned())
    }

    pub async fn get_default_memory_store(&self, org_id: i64) -> Result<Option<MemoryStoreDbRow>> {
        Ok(self
            .memory_stores
            .read()
            .values()
            .find(|s| s.org_id == org_id && s.is_default)
            .cloned())
    }

    pub async fn list_memory_stores(&self, org_id: i64) -> Result<Vec<MemoryStoreDbRow>> {
        let mut rows: Vec<_> = self
            .memory_stores
            .read()
            .values()
            .filter(|s| s.org_id == org_id)
            .cloned()
            .collect();
        rows.sort_by(|a, b| {
            b.is_default
                .cmp(&a.is_default)
                .then_with(|| a.created_at.cmp(&b.created_at))
        });
        Ok(rows)
    }

    pub async fn count_memory_stores(&self, org_id: i64) -> Result<i64> {
        Ok(self
            .memory_stores
            .read()
            .values()
            .filter(|s| s.org_id == org_id)
            .count() as i64)
    }

    // ============================================
    // Memories
    // ============================================

    pub async fn create_memory(&self, input: CreateMemoryRow) -> Result<MemoryDbRow> {
        // Verify store exists in this org.
        let stores = self.memory_stores.read();
        let Some(store) = stores
            .values()
            .find(|s| s.id == input.store_id && s.org_id == input.org_id)
            .cloned()
        else {
            bail!("memory store not found in this org");
        };
        drop(stores);

        let now = Self::now();
        let id = Uuid::now_v7();
        let row = MemoryDbRow {
            id,
            public_id: input.public_id,
            store_id: input.store_id,
            store_public_id: store.public_id,
            org_id: input.org_id,
            content: input.content,
            content_parts: input.content_parts,
            kind: input.kind,
            importance: input.importance.clamp(1, 10),
            tags: input.tags,
            active: true,
            created_at: now,
            updated_at: now,
        };
        self.memories.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_memory_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<MemoryDbRow>> {
        Ok(self
            .memories
            .read()
            .values()
            .find(|m| m.org_id == org_id && m.public_id == public_id)
            .cloned())
    }

    pub async fn list_memories(
        &self,
        org_id: i64,
        store_id: Option<Uuid>,
        filter: ListMemoriesFilter,
    ) -> Result<(Vec<MemoryDbRow>, i64)> {
        let memories = self.memories.read();
        let q_lower = filter
            .query
            .as_ref()
            .map(|q| q.trim().to_lowercase())
            .filter(|q| !q.is_empty());

        let mut matches: Vec<MemoryDbRow> = memories
            .values()
            .filter(|m| m.org_id == org_id)
            .filter(|m| store_id.is_none_or(|sid| m.store_id == sid))
            .filter(|m| filter.include_inactive || m.active)
            .filter(|m| filter.kind.as_deref().is_none_or(|k| m.kind == k))
            .filter(|m| {
                filter
                    .tags
                    .as_ref()
                    .is_none_or(|tags| tags.iter().all(|t| m.tags.contains(t)))
            })
            .filter(|m| {
                q_lower
                    .as_ref()
                    .is_none_or(|q| m.content.to_lowercase().contains(q))
            })
            .cloned()
            .collect();
        matches.sort_by(|a, b| {
            b.importance
                .cmp(&a.importance)
                .then_with(|| b.created_at.cmp(&a.created_at))
        });

        let total = matches.len() as i64;
        let limit = if filter.limit == 0 {
            10
        } else {
            filter.limit.min(200)
        };
        matches.truncate(limit);
        Ok((matches, total))
    }

    pub async fn count_active_memories(&self, store_id: Uuid) -> Result<i64> {
        Ok(self
            .memories
            .read()
            .values()
            .filter(|m| m.store_id == store_id && m.active)
            .count() as i64)
    }

    pub async fn deactivate_memory(
        &self,
        org_id: i64,
        store_id: Uuid,
        memory_public_id: &str,
    ) -> Result<bool> {
        let mut memories = self.memories.write();
        if let Some(m) = memories.values_mut().find(|m| {
            m.org_id == org_id
                && m.store_id == store_id
                && m.public_id == memory_public_id
                && m.active
        }) {
            m.active = false;
            m.updated_at = Self::now();
            Ok(true)
        } else {
            Ok(false)
        }
    }
}
