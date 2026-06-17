// In-memory storage: Knowledge Index + Document CRUD.
// See specs/knowledge-indexes.md.

use super::super::models::*;
use super::{InMemoryDatabase, matches_search_tokens};
use anyhow::{Result, bail};
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
}
