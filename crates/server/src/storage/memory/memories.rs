// In-memory storage: Workspace Memory CRUD

use super::super::models::*;
use super::{InMemoryDatabase, matches_search_tokens};
use anyhow::{Result, bail};
use chrono::{DateTime, Duration, Utc};
use uuid::Uuid;

impl InMemoryDatabase {
    pub async fn create_memory(&self, org_id: i64, input: CreateMemoryRow) -> Result<MemoryRow> {
        let now = Self::now();
        let mut memories = self.memories.write();

        if memories.values().any(|memory| {
            memory.org_id == org_id
                && memory.status != "deleted"
                && memory.name.eq_ignore_ascii_case(&input.name)
        }) {
            bail!("memory name already exists");
        }
        if input.scope == "agent"
            && memories.values().any(|memory| {
                memory.org_id == org_id
                    && memory.status != "deleted"
                    && memory.scope == "agent"
                    && memory.owner_agent_id == input.owner_agent_id
            })
        {
            bail!("agent memory already exists");
        }
        if input.scope == "user"
            && memories.values().any(|memory| {
                memory.org_id == org_id
                    && memory.status != "deleted"
                    && memory.scope == "user"
                    && memory.owner_user_id == input.owner_user_id
            })
        {
            bail!("user memory already exists");
        }

        let id = Uuid::now_v7();
        let row = MemoryRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            scope: input.scope,
            owner_agent_id: input.owner_agent_id,
            owner_user_id: input.owner_user_id,
            source_type: input.source_type,
            source_config: input.source_config,
            is_readonly: input.is_readonly,
            sync_status: input.sync_status,
            last_synced_at: None,
            last_sync_error: None,
            owner_principal_id: input.owner_principal_id,
            resolved_owner_user_id: input.resolved_owner_user_id,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        memories.insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_memory_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<MemoryRow>> {
        Ok(self
            .memories
            .read()
            .values()
            .find(|memory| {
                memory.org_id == org_id
                    && memory.public_id == public_id
                    && memory.status != "deleted"
            })
            .cloned())
    }

    pub async fn get_memory_by_id(&self, org_id: i64, id: Uuid) -> Result<Option<MemoryRow>> {
        Ok(self
            .memories
            .read()
            .get(&id)
            .filter(|memory| memory.org_id == org_id && memory.status != "deleted")
            .cloned())
    }

    pub async fn get_memory_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        // The new postgres unique index on memories(public_id) makes duplicate public_ids
        // impossible at the storage layer, but in-memory state can briefly contain
        // overlapping rows in tests that build fixtures across orgs. Pick the newest by
        // created_at to keep behavior deterministic across HashMap iteration orders.
        Ok(self
            .memories
            .read()
            .values()
            .filter(|memory| memory.public_id == public_id && memory.status != "deleted")
            .max_by_key(|memory| memory.created_at)
            .map(|memory| memory.org_id))
    }

    pub async fn list_memories(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<MemoryRow>> {
        let mut result: Vec<_> = self
            .memories
            .read()
            .values()
            .filter(|memory| {
                memory.org_id == org_id
                    && if include_archived {
                        memory.status != "deleted"
                    } else {
                        memory.status == "active"
                    }
                    && memory.scope == "org"
            })
            .filter(|memory| {
                matches_search_tokens(
                    search,
                    &[&memory.name, memory.description.as_deref().unwrap_or("")],
                )
            })
            .cloned()
            .collect();
        result.sort_by_key(|memory| std::cmp::Reverse(memory.created_at));
        Ok(result)
    }

    pub async fn get_memory_by_scope_owner(
        &self,
        org_id: i64,
        scope: &str,
        owner_agent_id: Option<everruns_provider::typed_id::AgentId>,
        owner_user_id: Option<Uuid>,
    ) -> Result<Option<MemoryRow>> {
        Ok(self
            .memories
            .read()
            .values()
            .find(|memory| {
                memory.org_id == org_id
                    && memory.status != "deleted"
                    && memory.scope == scope
                    && memory.owner_agent_id == owner_agent_id
                    && memory.owner_user_id == owner_user_id
            })
            .cloned())
    }

    pub async fn update_memory(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMemory,
    ) -> Result<Option<MemoryRow>> {
        let mut memories = self.memories.write();
        if let Some(name) = input.name.as_ref()
            && memories.values().any(|memory| {
                memory.org_id == org_id
                    && memory.id != id
                    && memory.status != "deleted"
                    && memory.name.eq_ignore_ascii_case(name)
            })
        {
            bail!("memory name already exists");
        }

        let Some(memory) = memories.get_mut(&id) else {
            return Ok(None);
        };
        if memory.org_id != org_id || memory.status == "deleted" {
            return Ok(None);
        }

        if let Some(name) = input.name {
            memory.name = name;
        }
        if let Some(description) = input.description {
            memory.description = description;
        }
        if let Some(source_type) = input.source_type {
            memory.source_type = source_type;
        }
        if let Some(source_config) = input.source_config {
            memory.source_config = source_config;
        }
        if let Some(is_readonly) = input.is_readonly {
            memory.is_readonly = is_readonly;
        }
        if let Some(sync_status) = input.sync_status {
            memory.sync_status = sync_status;
        }
        if let Some(last_synced_at) = input.last_synced_at {
            memory.last_synced_at = last_synced_at;
        }
        if let Some(last_sync_error) = input.last_sync_error {
            memory.last_sync_error = last_sync_error;
        }
        if let Some(status) = input.status {
            memory.status = status.clone();
            match status.as_str() {
                "active" => memory.archived_at = None,
                "archived" => memory.archived_at = Some(Self::now()),
                "deleted" => memory.deleted_at = Some(Self::now()),
                _ => {}
            }
        }
        memory.updated_at = Self::now();
        Ok(Some(memory.clone()))
    }

    pub async fn archive_memory(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut memories = self.memories.write();
        let Some(memory) = memories.get_mut(&id) else {
            return Ok(false);
        };
        if memory.org_id != org_id || memory.status != "active" {
            return Ok(false);
        }
        memory.status = "archived".to_string();
        memory.archived_at = Some(Self::now());
        memory.updated_at = Self::now();
        Ok(true)
    }

    pub async fn claim_next_memory_sync(&self) -> Result<Option<MemoryRow>> {
        let mut memories = self.memories.write();
        let Some(memory) = memories
            .values_mut()
            .filter(|memory| {
                memory.status == "active"
                    && memory.source_type != "manual"
                    && (memory.sync_status == "pending"
                        || (memory.sync_status == "syncing"
                            && memory.updated_at < Self::now() - Duration::minutes(15))
                        || is_due_for_scheduled_sync(memory))
            })
            .min_by_key(|memory| memory.updated_at)
        else {
            return Ok(None);
        };

        memory.sync_status = "syncing".to_string();
        memory.last_sync_error = None;
        memory.updated_at = Self::now();
        Ok(Some(memory.clone()))
    }

    pub async fn complete_memory_sync(
        &self,
        memory_id: Uuid,
        claimed_at: DateTime<Utc>,
        files: Vec<CreateMemoryFileRow>,
    ) -> Result<Option<MemoryRow>> {
        let now = Self::now();
        let mut memories = self.memories.write();
        let Some(memory) = memories.get_mut(&memory_id) else {
            return Ok(None);
        };
        if memory.status != "active"
            || memory.source_type == "manual"
            || memory.sync_status != "syncing"
            || memory.updated_at != claimed_at
        {
            return Ok(None);
        }

        let mut memory_files = self.memory_files.write();
        memory_files.retain(|_, file| file.memory_id != memory_id);
        for file in files {
            let id = Uuid::now_v7();
            let size_bytes = file.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);
            memory_files.insert(
                id,
                MemoryFileRow {
                    id,
                    memory_id,
                    path: file.path,
                    content: file.content,
                    is_directory: file.is_directory,
                    size_bytes,
                    content_hash: file.content_hash,
                    created_at: now,
                    updated_at: now,
                },
            );
        }

        memory.sync_status = "synced".to_string();
        memory.last_synced_at = Some(now);
        memory.last_sync_error = None;
        memory.updated_at = now;
        Ok(Some(memory.clone()))
    }

    pub async fn fail_memory_sync(
        &self,
        memory_id: Uuid,
        claimed_at: DateTime<Utc>,
        error: &str,
    ) -> Result<Option<MemoryRow>> {
        let mut memories = self.memories.write();
        let Some(memory) = memories.get_mut(&memory_id) else {
            return Ok(None);
        };
        if memory.status != "active"
            || memory.source_type == "manual"
            || memory.sync_status != "syncing"
            || memory.updated_at != claimed_at
        {
            return Ok(None);
        }

        memory.sync_status = "failed".to_string();
        memory.last_sync_error = Some(error.to_string());
        memory.updated_at = Self::now();
        Ok(Some(memory.clone()))
    }

    pub async fn list_all_memory_files(&self, memory_id: Uuid) -> Result<Vec<MemoryFileRow>> {
        let mut result: Vec<_> = self
            .memory_files
            .read()
            .values()
            .filter(|file| file.memory_id == memory_id)
            .cloned()
            .collect();
        result.sort_by_key(|file| file.path.clone());
        Ok(result)
    }

    // ============================================
    // Memory Files CRUD (virtual filesystem under a Memory)
    // ============================================

    pub async fn create_memory_file(
        &self,
        memory_id: Uuid,
        input: CreateMemoryFileRow,
    ) -> Result<MemoryFileRow> {
        let now = Self::now();
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);
        let mut files = self.memory_files.write();
        if files
            .values()
            .any(|f| f.memory_id == memory_id && f.path == input.path)
        {
            bail!("memory file already exists at path");
        }
        let id = Uuid::now_v7();
        let row = MemoryFileRow {
            id,
            memory_id,
            path: input.path,
            content: input.content,
            is_directory: input.is_directory,
            size_bytes,
            content_hash: input.content_hash,
            created_at: now,
            updated_at: now,
        };
        files.insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_memory_file(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<Option<MemoryFileRow>> {
        Ok(self
            .memory_files
            .read()
            .values()
            .find(|f| f.memory_id == memory_id && f.path == path)
            .cloned())
    }

    pub async fn get_memory_file_info(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<Option<MemoryFileInfoRow>> {
        Ok(self
            .memory_files
            .read()
            .values()
            .find(|f| f.memory_id == memory_id && f.path == path)
            .map(file_to_info))
    }

    pub async fn list_memory_files(
        &self,
        memory_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<MemoryFileInfoRow>> {
        let prefix = if parent_path == "/" {
            String::from("/")
        } else {
            format!("{}/", parent_path)
        };
        let mut rows: Vec<MemoryFileInfoRow> = self
            .memory_files
            .read()
            .values()
            .filter(|f| {
                if f.memory_id != memory_id || !f.path.starts_with(&prefix) {
                    return false;
                }
                let tail = &f.path[prefix.len()..];
                !tail.is_empty() && !tail.contains('/')
            })
            .map(file_to_info)
            .collect();
        rows.sort_by(|a, b| {
            b.is_directory
                .cmp(&a.is_directory)
                .then_with(|| a.path.cmp(&b.path))
        });
        Ok(rows)
    }

    pub async fn update_memory_file(
        &self,
        memory_id: Uuid,
        path: &str,
        input: UpdateMemoryFile,
    ) -> Result<Option<MemoryFileRow>> {
        let mut files = self.memory_files.write();
        let Some(file) = files
            .values_mut()
            .find(|f| f.memory_id == memory_id && f.path == path && !f.is_directory)
        else {
            return Ok(None);
        };
        if let Some(content) = input.content {
            file.size_bytes = content.len() as i64;
            file.content = Some(content);
        }
        if let Some(hash_set) = input.content_hash {
            file.content_hash = hash_set;
        }
        file.updated_at = Self::now();
        Ok(Some(file.clone()))
    }

    pub async fn delete_memory_file(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        let mut files = self.memory_files.write();
        let before = files.len();
        files.retain(|_, f| !(f.memory_id == memory_id && f.path == path));
        Ok(files.len() < before)
    }

    pub async fn delete_memory_file_recursive(&self, memory_id: Uuid, path: &str) -> Result<u64> {
        let prefix = if path == "/" {
            String::from("/")
        } else {
            format!("{}/", path)
        };
        let path_owned = path.to_string();
        let mut files = self.memory_files.write();
        let before = files.len();
        files.retain(|_, f| {
            if f.memory_id != memory_id {
                return true;
            }
            !(f.path == path_owned || f.path.starts_with(&prefix))
        });
        Ok((before - files.len()) as u64)
    }

    pub async fn grep_memory_files(
        &self,
        memory_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
        max_file_bytes: i64,
    ) -> Result<Vec<MemoryFileInfoRow>> {
        // When path filtering is requested, return metadata candidates without
        // scanning content. The service applies the shared glob matcher first.
        let content_re =
            regex::Regex::new(pattern).map_err(|e| anyhow::anyhow!("invalid pattern: {e}"))?;
        let mut rows: Vec<MemoryFileInfoRow> = self
            .memory_files
            .read()
            .values()
            .filter(|f| {
                if f.memory_id != memory_id || f.is_directory || f.size_bytes > max_file_bytes {
                    return false;
                }
                if path_pattern.is_some() {
                    return true;
                }
                let Some(ref content) = f.content else {
                    return false;
                };
                let Ok(text) = std::str::from_utf8(content) else {
                    return false;
                };
                content_re.is_match(text)
            })
            .map(file_to_info)
            .collect();
        rows.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(rows)
    }

    pub async fn memory_file_exists(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        Ok(self
            .memory_files
            .read()
            .values()
            .any(|f| f.memory_id == memory_id && f.path == path))
    }

    pub async fn memory_directory_has_children(&self, memory_id: Uuid, path: &str) -> Result<bool> {
        let prefix = if path == "/" {
            String::from("/")
        } else {
            format!("{}/", path)
        };
        Ok(self.memory_files.read().values().any(|f| {
            if f.memory_id != memory_id || !f.path.starts_with(&prefix) {
                return false;
            }
            let tail = &f.path[prefix.len()..];
            !tail.is_empty()
        }))
    }
}

fn file_to_info(file: &MemoryFileRow) -> MemoryFileInfoRow {
    MemoryFileInfoRow {
        id: file.id,
        memory_id: file.memory_id,
        path: file.path.clone(),
        is_directory: file.is_directory,
        size_bytes: file.size_bytes,
        content_hash: file.content_hash.clone(),
        created_at: file.created_at,
        updated_at: file.updated_at,
    }
}

fn is_due_for_scheduled_sync(memory: &MemoryRow) -> bool {
    if memory.sync_status != "synced" && memory.sync_status != "failed" {
        return false;
    }
    let Some(interval_secs) = memory
        .source_config
        .get("sync_interval_secs")
        .and_then(|value| value.as_i64())
        .filter(|value| *value > 0)
    else {
        return false;
    };
    let anchor = memory.last_synced_at.unwrap_or(memory.updated_at);
    anchor < InMemoryDatabase::now() - Duration::seconds(interval_secs)
}
