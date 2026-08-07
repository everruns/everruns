// In-memory storage: Knowledge Index + Document CRUD.
// See knowledge/runtime-resources/knowledge-indexes.md.

use super::super::models::*;
use super::{InMemoryDatabase, matches_search_tokens};
use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;
use uuid::Uuid;

impl InMemoryDatabase {
    // ------------- knowledge_indexes -------------

    pub async fn create_knowledge_index(
        &self,
        org_id: i64,
        input: CreateKnowledgeIndexRow,
    ) -> Result<KnowledgeIndexRow> {
        let now = Self::now();
        let mut indexes = self.knowledge_indexes.write();

        if indexes.values().any(|idx| {
            idx.org_id == org_id
                && idx.status != "deleted"
                && idx.name.eq_ignore_ascii_case(&input.name)
        }) {
            bail!("knowledge index name already exists");
        }

        let id = Uuid::now_v7();
        let row = KnowledgeIndexRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            source_type: input.source_type,
            source_config: input.source_config,
            embedding_model_id: input.embedding_model_id,
            vector_dim: None,
            vector_namespace: Some(input.vector_namespace),
            owner_principal_id: input.owner_principal_id,
            resolved_owner_user_id: input.resolved_owner_user_id,
            status: "active".to_string(),
            sync_status: "idle".to_string(),
            last_synced_at: None,
            last_sync_error: None,
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        indexes.insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_knowledge_index_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<KnowledgeIndexRow>> {
        Ok(self
            .knowledge_indexes
            .read()
            .values()
            .find(|idx| {
                idx.org_id == org_id && idx.public_id == public_id && idx.status != "deleted"
            })
            .cloned())
    }

    pub async fn get_knowledge_index_by_id(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeIndexRow>> {
        Ok(self
            .knowledge_indexes
            .read()
            .get(&id)
            .filter(|idx| idx.org_id == org_id && idx.status != "deleted")
            .cloned())
    }

    pub async fn list_knowledge_indexes(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<KnowledgeIndexRow>> {
        let mut result: Vec<_> = self
            .knowledge_indexes
            .read()
            .values()
            .filter(|idx| {
                idx.org_id == org_id
                    && if include_archived {
                        idx.status != "deleted"
                    } else {
                        idx.status == "active"
                    }
            })
            .filter(|idx| {
                matches_search_tokens(
                    search,
                    &[&idx.name, idx.description.as_deref().unwrap_or("")],
                )
            })
            .cloned()
            .collect();
        result.sort_by_key(|idx| std::cmp::Reverse(idx.created_at));
        Ok(result)
    }

    pub async fn update_knowledge_index(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateKnowledgeIndex,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let mut indexes = self.knowledge_indexes.write();
        if let Some(name) = input.name.as_ref()
            && indexes.values().any(|idx| {
                idx.org_id == org_id
                    && idx.id != id
                    && idx.status != "deleted"
                    && idx.name.eq_ignore_ascii_case(name)
            })
        {
            bail!("knowledge index name already exists");
        }

        let Some(idx) = indexes.get_mut(&id) else {
            return Ok(None);
        };
        if idx.org_id != org_id || idx.status == "deleted" {
            return Ok(None);
        }

        if let Some(name) = input.name {
            idx.name = name;
        }
        if let Some(description) = input.description {
            idx.description = description;
        }
        if let Some(source_config) = input.source_config {
            idx.source_config = source_config;
        }
        input
            .resolved_owner_user_id
            .apply(&mut idx.resolved_owner_user_id);
        if let Some(embedding_model_id) = input.embedding_model_id {
            idx.embedding_model_id = embedding_model_id;
        }
        if let Some(status) = input.status {
            idx.status = status.clone();
            // Match Postgres `COALESCE(archived_at, NOW())` semantics:
            // preserve the first archive/delete timestamp on repeats.
            match status.as_str() {
                "active" => idx.archived_at = None,
                "archived" if idx.archived_at.is_none() => {
                    idx.archived_at = Some(Self::now());
                }
                "deleted" if idx.deleted_at.is_none() => {
                    idx.deleted_at = Some(Self::now());
                }
                _ => {}
            }
        }

        idx.updated_at = Self::now();
        Ok(Some(idx.clone()))
    }

    pub async fn archive_knowledge_index(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut indexes = self.knowledge_indexes.write();
        let Some(idx) = indexes.get_mut(&id) else {
            return Ok(false);
        };
        if idx.org_id != org_id || idx.status != "active" {
            return Ok(false);
        }
        idx.status = "archived".to_string();
        if idx.archived_at.is_none() {
            idx.archived_at = Some(Self::now());
        }
        idx.updated_at = Self::now();
        Ok(true)
    }

    // ------------- knowledge_index_documents -------------

    pub async fn list_knowledge_index_documents(
        &self,
        index_id: Uuid,
    ) -> Result<Vec<KnowledgeIndexDocumentRow>> {
        let mut result: Vec<_> = self
            .knowledge_index_documents
            .read()
            .values()
            .filter(|doc| doc.index_id == index_id)
            .cloned()
            .collect();
        result.sort_by_key(|doc| std::cmp::Reverse(doc.created_at));
        Ok(result)
    }

    pub async fn count_knowledge_index_documents(
        &self,
        index_ids: &[Uuid],
    ) -> Result<HashMap<Uuid, usize>> {
        let wanted: std::collections::HashSet<_> = index_ids.iter().copied().collect();
        let mut counts = HashMap::new();
        for document in self.knowledge_index_documents.read().values() {
            if wanted.contains(&document.index_id) {
                *counts.entry(document.index_id).or_insert(0) += 1;
            }
        }
        Ok(counts)
    }

    pub async fn list_knowledge_index_chunks(
        &self,
        index_id: Uuid,
    ) -> Result<Vec<KnowledgeIndexChunkRow>> {
        let mut result: Vec<_> = self
            .knowledge_index_chunks
            .read()
            .values()
            .filter(|chunk| chunk.index_id == index_id)
            .cloned()
            .collect();
        result.sort_by_key(|a| (a.document_id, a.ordinal));
        Ok(result)
    }

    /// Hydrate chunk + owning-document citation metadata for a set of chunk
    /// `public_id`s within an index. Used by `search_index`. Missing ids are
    /// simply absent from the result.
    pub async fn get_knowledge_index_chunks_with_documents(
        &self,
        index_id: Uuid,
        chunk_public_ids: &[String],
    ) -> Result<Vec<KnowledgeIndexChunkWithDocument>> {
        if chunk_public_ids.is_empty() {
            return Ok(Vec::new());
        }
        let wanted: std::collections::HashSet<&str> =
            chunk_public_ids.iter().map(String::as_str).collect();
        let docs = self.knowledge_index_documents.read();
        let chunks = self.knowledge_index_chunks.read();
        let result = chunks
            .values()
            .filter(|chunk| chunk.index_id == index_id && wanted.contains(chunk.public_id.as_str()))
            .filter_map(|chunk| {
                let doc = docs.get(&chunk.document_id)?;
                Some(KnowledgeIndexChunkWithDocument {
                    chunk_public_id: chunk.public_id.clone(),
                    text: chunk.text.clone(),
                    location: chunk.location.clone(),
                    ordinal: chunk.ordinal,
                    source_uri: doc.source_uri.clone(),
                    document_title: doc.title.clone(),
                })
            })
            .collect();
        Ok(result)
    }

    // ------------- Syncout pipeline -------------

    /// Mark an index pending so the sync worker claims it. Idempotent: a
    /// pending/syncing index is left unchanged. Archived/deleted indexes are
    /// rejected.
    pub async fn enqueue_knowledge_index_sync(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let mut indexes = self.knowledge_indexes.write();
        let Some(idx) = indexes.get_mut(&id) else {
            return Ok(None);
        };
        if idx.org_id != org_id || idx.status != "active" {
            return Ok(None);
        }
        if idx.sync_status != "pending" && idx.sync_status != "syncing" {
            idx.sync_status = "pending".to_string();
            idx.updated_at = Self::now();
        }
        Ok(Some(idx.clone()))
    }

    /// Claim the next index needing a sync (pending, or a stale syncing claim).
    /// Flips it to `syncing` and returns the row (its `updated_at` is the claim
    /// timestamp).
    pub async fn claim_next_knowledge_index_sync(&self) -> Result<Option<KnowledgeIndexRow>> {
        let mut indexes = self.knowledge_indexes.write();
        let Some(idx) = indexes
            .values_mut()
            .filter(|idx| {
                idx.status == "active"
                    && (idx.sync_status == "pending"
                        || (idx.sync_status == "syncing"
                            && idx.updated_at < Self::now() - Duration::minutes(15)))
            })
            .min_by_key(|idx| idx.updated_at)
        else {
            return Ok(None);
        };
        idx.sync_status = "syncing".to_string();
        idx.last_sync_error = None;
        idx.updated_at = Self::now();
        Ok(Some(idx.clone()))
    }

    /// Atomically replace an index's documents + chunks and mark it synced.
    /// Guarded by `claimed_at`; a stale claim returns None and changes nothing.
    pub async fn complete_knowledge_index_sync(
        &self,
        index_id: Uuid,
        claimed_at: DateTime<Utc>,
        documents: Vec<CreateKnowledgeIndexDocumentWithChunks>,
        vector_dim: Option<i32>,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let now = Self::now();
        let mut indexes = self.knowledge_indexes.write();
        let Some(idx) = indexes.get_mut(&index_id) else {
            return Ok(None);
        };
        if idx.status != "active" || idx.sync_status != "syncing" || idx.updated_at != claimed_at {
            return Ok(None);
        }

        let mut docs = self.knowledge_index_documents.write();
        let mut chunks = self.knowledge_index_chunks.write();
        docs.retain(|_, doc| doc.index_id != index_id);
        chunks.retain(|_, chunk| chunk.index_id != index_id);

        for doc in documents {
            let doc_id = Uuid::now_v7();
            let chunk_count = doc.chunks.len() as i32;
            docs.insert(
                doc_id,
                KnowledgeIndexDocumentRow {
                    id: doc_id,
                    index_id,
                    public_id: doc.public_id,
                    source_uri: doc.source_uri,
                    title: doc.title,
                    mime_type: doc.mime_type,
                    content_hash: doc.content_hash,
                    size_bytes: doc.size_bytes,
                    chunk_count,
                    last_seen_at: Some(now),
                    created_at: now,
                    updated_at: now,
                },
            );
            for chunk in doc.chunks {
                let chunk_id = Uuid::now_v7();
                chunks.insert(
                    chunk_id,
                    KnowledgeIndexChunkRow {
                        id: chunk_id,
                        document_id: doc_id,
                        index_id,
                        public_id: chunk.public_id,
                        ordinal: chunk.ordinal,
                        text: chunk.text,
                        location: chunk.location,
                        token_count: chunk.token_count,
                        created_at: now,
                    },
                );
            }
        }

        idx.sync_status = "synced".to_string();
        idx.last_synced_at = Some(now);
        idx.last_sync_error = None;
        if let Some(dim) = vector_dim {
            idx.vector_dim = Some(dim);
        }
        idx.updated_at = now;
        Ok(Some(idx.clone()))
    }

    /// Mark an index's sync as failed, preserving previously indexed content.
    /// Guarded by `claimed_at`; a stale claim returns None and changes nothing.
    pub async fn fail_knowledge_index_sync(
        &self,
        index_id: Uuid,
        claimed_at: DateTime<Utc>,
        error: &str,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let mut indexes = self.knowledge_indexes.write();
        let Some(idx) = indexes.get_mut(&index_id) else {
            return Ok(None);
        };
        if idx.status != "active" || idx.sync_status != "syncing" || idx.updated_at != claimed_at {
            return Ok(None);
        }
        idx.sync_status = "failed".to_string();
        idx.last_sync_error = Some(error.to_string());
        idx.updated_at = Self::now();
        Ok(Some(idx.clone()))
    }
}
