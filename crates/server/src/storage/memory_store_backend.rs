// Database-backed `MemoryStoreBackend` implementation.
//
// Bridges the core `MemoryStoreBackend` trait (used by `remember`, `recall`,
// and `forget` tools in `crates/core/src/capabilities/persistent_memory.rs`)
// to the `StorageBackend` (Postgres or in-memory dev mode).
//
// Org-scoped: each instance is bound to a specific org_id at construction time.
// All operations enforce that store_id and memory_id belong to that org;
// foreign rows surface as `Ok(None)` or `Ok(false)` (non-disclosing).

use super::backend::StorageBackend;
use super::models::{CreateMemoryRow, CreateMemoryStoreRow, ListMemoriesFilter, MemoryDbRow};
use async_trait::async_trait;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::memory_store::{
    Memory, MemoryContentPart, MemoryKind, MemoryQuery, MemoryStoreBackend, MemoryStoreEntity,
};
use everruns_core::organization::org_public_id_from_internal;
use everruns_core::typed_id::{MemoryId, MemoryStoreId, OrgId};
use std::str::FromStr;
use std::sync::Arc;

fn org_id_to_public(internal: i64) -> Result<OrgId> {
    org_public_id_from_internal(internal)
        .parse()
        .map_err(|e: everruns_core::typed_id::IdParseError| AgentLoopError::store(e.to_string()))
}

#[derive(Clone)]
pub struct DbMemoryStore {
    db: Arc<StorageBackend>,
    org_id: i64,
}

impl DbMemoryStore {
    pub fn new(db: Arc<StorageBackend>, org_id: i64) -> Self {
        Self { db, org_id }
    }
}

fn store_err<E: std::fmt::Display>(e: E) -> AgentLoopError {
    AgentLoopError::store(e.to_string())
}

fn parse_kind(s: &str) -> MemoryKind {
    match s {
        "preference" => MemoryKind::Preference,
        "correction" => MemoryKind::Correction,
        "procedure" => MemoryKind::Procedure,
        "context" => MemoryKind::Context,
        _ => MemoryKind::Fact,
    }
}

fn row_to_memory(row: MemoryDbRow) -> Result<Memory> {
    let id = MemoryId::from_str(&row.public_id).map_err(store_err)?;
    let store_id = MemoryStoreId::from_str(&row.store_public_id).map_err(store_err)?;
    let content_parts: Vec<MemoryContentPart> = if row.content_parts.is_null() {
        Vec::new()
    } else {
        serde_json::from_value(row.content_parts).map_err(store_err)?
    };
    Ok(Memory {
        id,
        store_id,
        content: row.content,
        content_parts,
        kind: parse_kind(&row.kind),
        importance: row.importance.clamp(1, 10) as u8,
        tags: row.tags,
        active: row.active,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

fn store_row_to_entity(row: super::models::MemoryStoreDbRow) -> Result<MemoryStoreEntity> {
    Ok(MemoryStoreEntity {
        id: MemoryStoreId::from_str(&row.public_id).map_err(store_err)?,
        org_id: org_id_to_public(row.org_id)?,
        name: row.name,
        is_default: row.is_default,
        created_at: row.created_at,
    })
}

#[async_trait]
impl MemoryStoreBackend for DbMemoryStore {
    async fn get_or_create_default_store(&self, org_id: OrgId) -> Result<MemoryStoreEntity> {
        let expected = org_id_to_public(self.org_id)?;
        if org_id != expected {
            return Err(AgentLoopError::tool(
                "Memory store backend is scoped to a different org",
            ));
        }

        if let Some(existing) = self
            .db
            .get_default_memory_store(self.org_id)
            .await
            .map_err(store_err)?
        {
            return store_row_to_entity(existing);
        }

        // Pick a name that won't collide with an existing user-created store
        // named "default" (store names are unique per org). The first attempt
        // uses the friendly "default"; on conflict we fall back to a name
        // suffixed with a short hex slice of the new store ID.
        let public_id = MemoryStoreId::new().to_string();
        let primary_input = CreateMemoryStoreRow {
            public_id: public_id.clone(),
            name: "default".to_string(),
            is_default: true,
        };
        match self
            .db
            .create_memory_store(self.org_id, primary_input)
            .await
        {
            Ok(row) => store_row_to_entity(row),
            Err(_) => {
                // Likely a uniqueness conflict on the "default" name. Retry
                // with a unique name derived from the store's public_id so
                // initialisation cannot get stuck on a user-created collision.
                let suffix = public_id
                    .strip_prefix("mst_")
                    .map(|s| &s[..8.min(s.len())])
                    .unwrap_or("0")
                    .to_string();
                let row = self
                    .db
                    .create_memory_store(
                        self.org_id,
                        CreateMemoryStoreRow {
                            public_id,
                            name: format!("default-{suffix}"),
                            is_default: true,
                        },
                    )
                    .await
                    .map_err(store_err)?;
                store_row_to_entity(row)
            }
        }
    }

    async fn get_store(&self, store_id: MemoryStoreId) -> Result<Option<MemoryStoreEntity>> {
        let Some(row) = self
            .db
            .get_memory_store_by_public_id(self.org_id, &store_id.to_string())
            .await
            .map_err(store_err)?
        else {
            return Ok(None);
        };
        Ok(Some(store_row_to_entity(row)?))
    }

    async fn create_memory(
        &self,
        store_id: MemoryStoreId,
        content: String,
        content_parts: Vec<MemoryContentPart>,
        kind: MemoryKind,
        importance: u8,
        tags: Vec<String>,
    ) -> Result<Memory> {
        let store = self
            .db
            .get_memory_store_by_public_id(self.org_id, &store_id.to_string())
            .await
            .map_err(store_err)?
            .ok_or_else(|| AgentLoopError::tool("memory store not found"))?;

        let parts_json = serde_json::to_value(&content_parts).map_err(store_err)?;
        let public_id = MemoryId::new().to_string();
        let cap = everruns_core::memory_store::MemoryLimits::MAX_MEMORIES_PER_STORE as i64;
        // Atomic check+insert: serialises against concurrent creates targeting
        // the same store so the per-store cap cannot be exceeded under load.
        let row = self
            .db
            .create_memory_with_cap(
                CreateMemoryRow {
                    public_id,
                    store_id: store.id,
                    org_id: self.org_id,
                    content,
                    content_parts: parts_json,
                    kind: kind.to_string(),
                    importance: importance.clamp(1, 10) as i16,
                    tags,
                },
                cap,
            )
            .await
            .map_err(store_err)?
            .ok_or_else(|| {
                AgentLoopError::tool(format!("memory store full ({cap} active memories)"))
            })?;
        row_to_memory(row)
    }

    async fn recall(&self, query: MemoryQuery) -> Result<(Vec<Memory>, usize)> {
        let store_uuid = if let Some(sid) = query.store_id {
            let Some(store) = self
                .db
                .get_memory_store_by_public_id(self.org_id, &sid.to_string())
                .await
                .map_err(store_err)?
            else {
                return Ok((Vec::new(), 0));
            };
            Some(store.id)
        } else {
            None
        };

        let filter = ListMemoriesFilter {
            query: query.query.clone(),
            kind: query.kind.map(|k| k.to_string()),
            tags: query.tags.clone(),
            include_inactive: false,
            limit: query.limit,
        };
        let (rows, total) = self
            .db
            .list_memories(self.org_id, store_uuid, filter)
            .await
            .map_err(store_err)?;

        let memories = rows
            .into_iter()
            .map(row_to_memory)
            .collect::<Result<Vec<_>>>()?;
        Ok((memories, total as usize))
    }

    async fn forget(&self, store_id: MemoryStoreId, memory_id: MemoryId) -> Result<bool> {
        let Some(store) = self
            .db
            .get_memory_store_by_public_id(self.org_id, &store_id.to_string())
            .await
            .map_err(store_err)?
        else {
            return Ok(false);
        };
        self.db
            .deactivate_memory(self.org_id, store.id, &memory_id.to_string())
            .await
            .map_err(store_err)
    }

    async fn count_active(&self, store_id: MemoryStoreId) -> Result<usize> {
        let Some(store) = self
            .db
            .get_memory_store_by_public_id(self.org_id, &store_id.to_string())
            .await
            .map_err(store_err)?
        else {
            return Ok(0);
        };
        Ok(self
            .db
            .count_active_memories(store.id)
            .await
            .map_err(store_err)? as usize)
    }
}
