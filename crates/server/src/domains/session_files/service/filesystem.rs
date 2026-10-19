//! Session filesystem port over workspace and Memory services.
use super::*;
#[async_trait]
impl SessionFileSystem for WorkspaceFileService {
    async fn create_file_if_absent(
        &self,
        session: SessionId,
        path: &str,
        content: &str,
    ) -> everruns_contracts::error::Result<bool> {
        Self::validate_path(path).map_err(file_system_error)?;
        let Some((mount, inner)) = self.route_memory(session.uuid(), path).await else {
            return Err(everruns_contracts::error::AgentLoopError::config(
                "artifact destination must be Memory",
            ));
        };
        self.memory_mounts
            .create_file_if_absent(&mount, &inner, content)
            .await
            .map_err(file_system_error)
    }

    async fn dreaming_scope(
        &self,
        session_id: SessionId,
        mount: &str,
    ) -> everruns_contracts::error::Result<Option<everruns_core::dreaming::DreamMemoryScope>> {
        let Some((memory, inner)) = self.route_memory(session_id.uuid(), mount).await else {
            return Ok(None);
        };
        if inner != "/"
            || !self
                .memory_mounts
                .writable_for_dreaming(&memory)
                .await
                .map_err(file_system_error)?
        {
            return Ok(None);
        }
        // Source privacy follows session ownership, even when the owner's
        // private Memory is archived, read-only or unavailable.
        if mount != "/memory/user" {
            let source = self
                .db
                .get_session_unscoped(session_id)
                .await
                .map_err(file_system_error)?;
            if source.is_none_or(|session| session.resolved_owner_user_id.is_some()) {
                return Ok(None);
            }
        }
        Ok(Some(everruns_core::dreaming::DreamMemoryScope {
            key: memory.memory_id.to_string(),
            private: mount == "/memory/user",
        }))
    }

    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn seed_initial_file(
        &self,
        session_id: SessionId,
        file: &everruns_core::InitialFile,
    ) -> everruns_contracts::error::Result<()> {
        let session_id_uuid = session_id.uuid();
        let path = Self::normalize_path(&file.path);
        Self::validate_path(&path).map_err(file_system_error)?;
        self.ensure_path_not_virtual(session_id_uuid, &path)
            .map_err(file_system_error)?;
        self.ensure_parent_path_writable(session_id_uuid, &path)
            .await
            .map_err(file_system_error)?;

        if let Some(parent) = FileInfo::parent_path(&path) {
            self.ensure_directory_exists(session_id_uuid, &parent)
                .await
                .map_err(file_system_error)?;
        }

        let content = SessionFile::decode_content(&file.content, &file.encoding)
            .map_err(file_system_display_error)?;

        if self
            .db
            .session_file_exists(session_id_uuid, &path)
            .await
            .map_err(file_system_error)?
        {
            self.db
                .update_session_file(
                    session_id_uuid,
                    &path,
                    UpdateSessionFile {
                        content: Some(content),
                        is_readonly: Some(file.is_readonly),
                    },
                )
                .await
                .map_err(file_system_error)?
                .ok_or_else(|| AgentLoopError::store("File not found after update"))?;
        } else {
            self.db
                .create_session_file(CreateSessionFileRow {
                    session_id,
                    path,
                    content: Some(content),
                    is_directory: false,
                    is_readonly: file.is_readonly,
                })
                .await
                .map_err(file_system_error)?;
        }

        Ok(())
    }

    async fn read_file(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> everruns_contracts::error::Result<Option<SessionFile>> {
        WorkspaceFileService::read_file(self, session_id.uuid(), path)
            .await
            .map_err(file_system_error)
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> everruns_contracts::error::Result<SessionFile> {
        let session_id_uuid = session_id.uuid();
        if WorkspaceFileService::read_file(self, session_id_uuid, path)
            .await
            .map_err(file_system_error)?
            .is_some()
        {
            return self
                .update_file(
                    session_id_uuid,
                    path,
                    UpdateFileInput {
                        content: Some(content.to_string()),
                        encoding: Some(encoding.to_string()),
                        is_readonly: None,
                    },
                )
                .await
                .map_err(file_system_error)?
                .ok_or_else(|| AgentLoopError::store("File not found after update"));
        }

        match self
            .create_file(
                session_id_uuid,
                CreateFileInput {
                    path: path.to_string(),
                    content: Some(content.to_string()),
                    encoding: Some(encoding.to_string()),
                    is_readonly: Some(false),
                },
            )
            .await
        {
            Ok(file) => Ok(file),
            Err(error) if is_already_exists_error(&error) => self
                .update_file(
                    session_id_uuid,
                    path,
                    UpdateFileInput {
                        content: Some(content.to_string()),
                        encoding: Some(encoding.to_string()),
                        is_readonly: None,
                    },
                )
                .await
                .map_err(file_system_error)?
                .ok_or_else(|| AgentLoopError::store("File not found after race recovery")),
            Err(error) => Err(file_system_error(error)),
        }
    }

    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> everruns_contracts::error::Result<Option<SessionFile>> {
        self.update_file_if_content_matches(
            session_id.uuid(),
            path,
            expected_content,
            expected_encoding,
            content,
            encoding,
        )
        .await
        .map_err(file_system_error)
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> everruns_contracts::error::Result<bool> {
        self.delete(session_id.uuid(), path, recursive)
            .await
            .map_err(file_system_error)
    }

    async fn list_directory(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> everruns_contracts::error::Result<Vec<FileInfo>> {
        WorkspaceFileService::list_directory(self, session_id.uuid(), path)
            .await
            .map_err(file_system_error)
    }

    async fn stat_file(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> everruns_contracts::error::Result<Option<FileStat>> {
        self.stat(session_id.uuid(), path)
            .await
            .map_err(file_system_error)
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> everruns_contracts::error::Result<Vec<GrepMatch>> {
        let results = self
            .grep(
                session_id.uuid(),
                GrepInput {
                    pattern: pattern.to_string(),
                    path_pattern: path_pattern.map(ToString::to_string),
                    excluded_path_prefix: None,
                },
            )
            .await
            .map_err(file_system_error)?;
        Ok(results
            .into_iter()
            .flat_map(|result| result.matches)
            .collect())
    }

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> everruns_contracts::error::Result<GrepSearchResult> {
        self.restore_virtual_mounts(session_id.uuid())
            .await
            .map_err(file_system_error)?;
        grep_session_files_with_options(
            &self.db,
            self.virtual_registry.as_deref(),
            session_id.uuid(),
            pattern,
            options,
            None,
        )
        .await
        .map_err(file_system_error)
    }

    async fn create_directory(
        &self,
        session_id: SessionId,
        path: &str,
    ) -> everruns_contracts::error::Result<FileInfo> {
        WorkspaceFileService::create_directory(
            self,
            session_id.uuid(),
            CreateDirectoryInput {
                path: path.to_string(),
            },
        )
        .await
        .map_err(file_system_error)
    }
}
