// In-memory storage: Knowledge Base + Entry CRUD

use super::super::models::*;
use super::{InMemoryDatabase, matches_search_tokens};
use anyhow::{Result, bail};
use uuid::Uuid;

impl InMemoryDatabase {
    // ------------- knowledge_bases -------------

    pub async fn create_knowledge_base(
        &self,
        org_id: i64,
        input: CreateKnowledgeBaseRow,
    ) -> Result<KnowledgeBaseRow> {
        let now = Self::now();
        let mut bases = self.knowledge_bases.write();

        if bases.values().any(|kb| {
            kb.org_id == org_id
                && kb.status != "deleted"
                && kb.name.eq_ignore_ascii_case(&input.name)
        }) {
            bail!("knowledge base name already exists");
        }

        let id = Uuid::now_v7();
        let row = KnowledgeBaseRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            owner_principal_id: input.owner_principal_id,
            resolved_owner_user_id: input.resolved_owner_user_id,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        bases.insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_knowledge_base_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<KnowledgeBaseRow>> {
        Ok(self
            .knowledge_bases
            .read()
            .values()
            .find(|kb| kb.org_id == org_id && kb.public_id == public_id && kb.status != "deleted")
            .cloned())
    }

    pub async fn get_knowledge_base_by_id(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeBaseRow>> {
        Ok(self
            .knowledge_bases
            .read()
            .get(&id)
            .filter(|kb| kb.org_id == org_id && kb.status != "deleted")
            .cloned())
    }

    pub async fn list_knowledge_bases(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<KnowledgeBaseRow>> {
        let mut result: Vec<_> = self
            .knowledge_bases
            .read()
            .values()
            .filter(|kb| {
                kb.org_id == org_id
                    && if include_archived {
                        kb.status != "deleted"
                    } else {
                        kb.status == "active"
                    }
            })
            .filter(|kb| {
                matches_search_tokens(search, &[&kb.name, kb.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        result.sort_by_key(|kb| std::cmp::Reverse(kb.created_at));
        Ok(result)
    }

    pub async fn update_knowledge_base(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateKnowledgeBase,
    ) -> Result<Option<KnowledgeBaseRow>> {
        let mut bases = self.knowledge_bases.write();
        if let Some(name) = input.name.as_ref()
            && bases.values().any(|kb| {
                kb.org_id == org_id
                    && kb.id != id
                    && kb.status != "deleted"
                    && kb.name.eq_ignore_ascii_case(name)
            })
        {
            bail!("knowledge base name already exists");
        }

        let Some(kb) = bases.get_mut(&id) else {
            return Ok(None);
        };
        if kb.org_id != org_id || kb.status == "deleted" {
            return Ok(None);
        }

        if let Some(name) = input.name {
            kb.name = name;
        }
        if let Some(description) = input.description {
            kb.description = description;
        }
        if let Some(status) = input.status {
            kb.status = status.clone();
            match status.as_str() {
                "active" => kb.archived_at = None,
                "archived" => kb.archived_at = Some(Self::now()),
                "deleted" => kb.deleted_at = Some(Self::now()),
                _ => {}
            }
        }
        kb.updated_at = Self::now();
        Ok(Some(kb.clone()))
    }

    pub async fn archive_knowledge_base(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut bases = self.knowledge_bases.write();
        let Some(kb) = bases.get_mut(&id) else {
            return Ok(false);
        };
        if kb.org_id != org_id || kb.status != "active" {
            return Ok(false);
        }
        kb.status = "archived".to_string();
        kb.archived_at = Some(Self::now());
        kb.updated_at = Self::now();
        Ok(true)
    }

    // ------------- knowledge_entries -------------

    pub async fn create_knowledge_entry(
        &self,
        kb_id: Uuid,
        input: CreateKnowledgeEntryRow,
    ) -> Result<KnowledgeEntryRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = KnowledgeEntryRow {
            id,
            kb_id,
            public_id: input.public_id,
            title: input.title,
            body: input.body,
            kind: input.kind,
            tags: input.tags,
            created_at: now,
            updated_at: now,
        };
        self.knowledge_entries.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_knowledge_entry_by_public_id(
        &self,
        kb_id: Uuid,
        public_id: &str,
    ) -> Result<Option<KnowledgeEntryRow>> {
        Ok(self
            .knowledge_entries
            .read()
            .values()
            .find(|e| e.kb_id == kb_id && e.public_id == public_id)
            .cloned())
    }

    pub async fn list_knowledge_entries(
        &self,
        kb_id: Uuid,
        search: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<KnowledgeEntryRow>> {
        let mut result: Vec<_> = self
            .knowledge_entries
            .read()
            .values()
            .filter(|e| e.kb_id == kb_id)
            .filter(|e| kind.map(|k| e.kind == k).unwrap_or(true))
            .filter(|e| matches_search_tokens(search, &[&e.title, &e.body]))
            .cloned()
            .collect();
        result.sort_by_key(|e| std::cmp::Reverse(e.created_at));
        Ok(result)
    }

    pub async fn search_knowledge_entries(
        &self,
        kb_ids: &[Uuid],
        query: &str,
        kind: Option<&str>,
        tags: &[String],
        limit: usize,
    ) -> Result<Vec<KnowledgeEntryRow>> {
        if kb_ids.is_empty() {
            return Ok(Vec::new());
        }
        let lowered_tags: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
        let mut result: Vec<_> = self
            .knowledge_entries
            .read()
            .values()
            .filter(|e| kb_ids.contains(&e.kb_id))
            .filter(|e| kind.map(|k| e.kind == k).unwrap_or(true))
            .filter(|e| {
                lowered_tags
                    .iter()
                    .all(|t| e.tags.iter().any(|et| et.eq_ignore_ascii_case(t)))
            })
            .filter(|e| matches_search_tokens(Some(query), &[&e.title, &e.body]))
            .cloned()
            .collect();
        result.sort_by_key(|e| std::cmp::Reverse(e.updated_at));
        result.truncate(limit);
        Ok(result)
    }

    pub async fn update_knowledge_entry(
        &self,
        kb_id: Uuid,
        id: Uuid,
        input: UpdateKnowledgeEntry,
    ) -> Result<Option<KnowledgeEntryRow>> {
        let mut entries = self.knowledge_entries.write();
        let Some(entry) = entries.get_mut(&id) else {
            return Ok(None);
        };
        if entry.kb_id != kb_id {
            return Ok(None);
        }
        if let Some(title) = input.title {
            entry.title = title;
        }
        if let Some(body) = input.body {
            entry.body = body;
        }
        if let Some(kind) = input.kind {
            entry.kind = kind;
        }
        if let Some(tags) = input.tags {
            entry.tags = tags;
        }
        entry.updated_at = Self::now();
        Ok(Some(entry.clone()))
    }

    pub async fn delete_knowledge_entry(&self, kb_id: Uuid, id: Uuid) -> Result<bool> {
        let mut entries = self.knowledge_entries.write();
        let Some(entry) = entries.get(&id) else {
            return Ok(false);
        };
        if entry.kb_id != kb_id {
            return Ok(false);
        }
        entries.remove(&id);
        Ok(true)
    }
}
