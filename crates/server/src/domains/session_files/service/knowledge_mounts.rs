//! Compose static workspace scaffolds with live, session-authorized collections.
use super::*;
use crate::domains::knowledge::{KnowledgeService, runtime::is_mount};

pub(super) enum KnowledgeRoute<T> {
    Legacy,
    Resolved(T),
}

/// Collection IDs reserve their subtree even when the collection is not bound.
pub(crate) fn collection_path(path: &str) -> bool {
    let path = everruns_core::session_path::to_session_path(path);
    ["/knowledge/", "/knowledge-text/"]
        .iter()
        .filter_map(|root| path.strip_prefix(*root))
        .any(|rest| {
            rest.split('/')
                .next()
                .is_some_and(|name| name.starts_with("kn_"))
        })
}

impl WorkspaceFileService {
    /// A workspace address cannot serve as proof of a session's attachments.
    pub(crate) fn without_session_knowledge(mut self) -> Self {
        self.session_knowledge = false;
        self
    }

    pub(in crate::domains::session_files) async fn legacy_workspace_key(
        &self,
        session: Uuid,
    ) -> Result<Uuid> {
        if !self.session_knowledge {
            return Ok(session);
        }
        Ok(self
            .db
            .get_session_unscoped(SessionId::from_uuid(session))
            .await?
            .map_or(session, |row| row.workspace_id))
    }

    async fn legacy_knowledge_keys(&self, session: Uuid) -> Result<Vec<Uuid>> {
        let workspace = self.legacy_workspace_key(session).await?;
        let mut keys = vec![workspace];
        if workspace != session {
            keys.push(session);
        }
        Ok(keys)
    }

    pub(super) async fn knowledge_read(
        &self,
        session: Uuid,
        path: &str,
    ) -> Result<KnowledgeRoute<Option<SessionFile>>> {
        if !is_mount(path) {
            return Ok(KnowledgeRoute::Legacy);
        }
        if self.session_knowledge
            && !matches!(path, "/knowledge" | "/knowledge-text")
            && let Some(mut file) = KnowledgeService::files(self.db.clone())
                .mount_read(session, path)
                .await?
        {
            // Preserve the scaffold's OKF frontmatter before the live catalog.
            if path == "/knowledge/index.md"
                && let Some(legacy) = self.legacy_knowledge_file(session, path).await?
                && legacy.encoding == "text"
            {
                let content = format!(
                    "{}\n\n{}",
                    legacy.content.unwrap_or_default(),
                    file.content.unwrap_or_default()
                );
                file.size_bytes = content.len() as i64;
                file.content = Some(content);
            }
            return Ok(KnowledgeRoute::Resolved(Some(file)));
        }
        // THREAT[TM-FS-023]: a miss or deny never grants fallback collection bytes.
        if collection_path(path) {
            return Ok(KnowledgeRoute::Resolved(None));
        }
        Ok(match self.legacy_knowledge_file(session, path).await? {
            Some(file) => KnowledgeRoute::Resolved(Some(file)),
            None => KnowledgeRoute::Legacy,
        })
    }

    async fn legacy_knowledge_file(
        &self,
        session: Uuid,
        path: &str,
    ) -> Result<Option<SessionFile>> {
        for key in self.legacy_knowledge_keys(session).await? {
            if let Some(registry) = &self.virtual_registry
                && let Some(file) = registry.read_file(&key, path)
            {
                let (content, encoding) = SessionFile::encode_content(&file.content);
                let now = chrono::Utc::now();
                return Ok(Some(SessionFile {
                    id: Uuid::nil(),
                    session_id: key,
                    path: path.into(),
                    name: FileInfo::name_from_path(path),
                    content: (!file.is_directory).then_some(content),
                    encoding,
                    is_directory: file.is_directory,
                    is_readonly: true,
                    size_bytes: file.content.len() as i64,
                    created_at: now,
                    updated_at: now,
                }));
            }
            if let Some(row) = self.db.get_session_file(key, path).await? {
                let mut file = Self::row_to_session_file(row);
                file.is_readonly = true;
                return Ok(Some(file));
            }
        }
        Ok(None)
    }

    pub(super) async fn knowledge_stat(
        &self,
        session: Uuid,
        path: &str,
    ) -> Result<KnowledgeRoute<Option<FileStat>>> {
        if !is_mount(path) {
            return Ok(KnowledgeRoute::Legacy);
        }
        if self.session_knowledge
            && let Some(stat) = KnowledgeService::files(self.db.clone())
                .mount_stat(session, path)
                .await?
        {
            return Ok(KnowledgeRoute::Resolved(Some(stat)));
        }
        if collection_path(path) {
            return Ok(KnowledgeRoute::Resolved(None));
        }
        Ok(match self.legacy_knowledge_file(session, path).await? {
            Some(file) => KnowledgeRoute::Resolved(Some(FileStat {
                path: file.path,
                name: file.name,
                is_directory: file.is_directory,
                is_readonly: true,
                size_bytes: file.size_bytes,
                created_at: file.created_at,
                updated_at: file.updated_at,
            })),
            None => KnowledgeRoute::Legacy,
        })
    }

    pub(super) async fn knowledge_list(
        &self,
        session: Uuid,
        path: &str,
    ) -> Result<KnowledgeRoute<Vec<FileInfo>>> {
        if !is_mount(path) {
            return Ok(KnowledgeRoute::Legacy);
        }
        let collections = if self.session_knowledge {
            KnowledgeService::files(self.db.clone())
                .mount_list(session, path)
                .await?
        } else {
            Vec::new()
        };
        if collection_path(path) {
            return Ok(KnowledgeRoute::Resolved(collections));
        }
        let mut entries = std::collections::BTreeMap::new();
        // Workspace files win over any session-local static fallback.
        for key in self.legacy_knowledge_keys(session).await?.into_iter().rev() {
            for row in self.db.list_session_files(key, path).await? {
                let mut file = Self::row_to_file_info_from_info(row);
                if !collection_path(&file.path) {
                    file.is_readonly = true;
                    entries.insert(file.name.clone(), file);
                }
            }
            if let Some(registry) = &self.virtual_registry {
                let now = chrono::Utc::now();
                for file in registry.list_directory(&key, path) {
                    if collection_path(&file.path) {
                        continue;
                    }
                    let name = FileInfo::name_from_path(&file.path);
                    entries.insert(
                        name.clone(),
                        FileInfo {
                            id: Uuid::nil(),
                            session_id: key,
                            path: file.path,
                            name,
                            is_directory: file.is_directory,
                            is_readonly: true,
                            size_bytes: file.size_bytes,
                            created_at: now,
                            updated_at: now,
                        },
                    );
                }
            }
        }
        if entries.is_empty()
            && collections.is_empty()
            && !matches!(path, "/knowledge" | "/knowledge-text")
        {
            match self.legacy_knowledge_file(session, path).await? {
                None => return Ok(KnowledgeRoute::Legacy),
                Some(file) if !file.is_directory => {
                    return Err(anyhow!("Path is not a directory: {}", path));
                }
                Some(_) => {}
            }
        }
        for file in collections {
            entries.insert(file.name.clone(), file);
        }
        let mut entries: Vec<_> = entries.into_values().collect();
        entries.sort_by(|a, b| {
            b.is_directory
                .cmp(&a.is_directory)
                .then_with(|| a.path.cmp(&b.path))
        });
        Ok(KnowledgeRoute::Resolved(entries))
    }
}
