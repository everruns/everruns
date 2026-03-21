// In-memory storage: Session Files

use super::super::models::*;
use super::InMemoryDatabase;
use anyhow::Result;
use anyhow::anyhow;
use everruns_core::SessionId;
use uuid::Uuid;

impl InMemoryDatabase {
    // ============================================
    // Session Files
    // ============================================

    pub async fn create_session_file(&self, input: CreateSessionFileRow) -> Result<SessionFileRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let content_len = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);
        let row = SessionFileRow {
            id,
            session_id: input.session_id,
            path: input.path,
            content: input.content,
            is_directory: input.is_directory,
            is_readonly: input.is_readonly,
            size_bytes: content_len,
            created_at: now,
            updated_at: now,
        };
        self.session_files.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_session_file(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileRow>> {
        Ok(self
            .session_files
            .read()
            .values()
            .find(|f| f.session_id == session_id && f.path == path)
            .cloned())
    }

    pub async fn get_session_file_by_id(&self, id: Uuid) -> Result<Option<SessionFileRow>> {
        Ok(self.session_files.read().get(&id).cloned())
    }

    /// Convert SessionFileRow to SessionFileInfoRow (strips content)
    fn file_to_info(f: &SessionFileRow) -> SessionFileInfoRow {
        SessionFileInfoRow {
            id: f.id,
            session_id: f.session_id,
            path: f.path.clone(),
            is_directory: f.is_directory,
            is_readonly: f.is_readonly,
            size_bytes: f.size_bytes,
            created_at: f.created_at,
            updated_at: f.updated_at,
        }
    }

    pub async fn list_session_files(
        &self,
        session_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<SessionFileInfoRow>> {
        let files = self.session_files.read();
        let prefix = if parent_path == "/" {
            "/".to_string()
        } else {
            format!("{}/", parent_path.trim_end_matches('/'))
        };

        let mut result: Vec<_> = files
            .values()
            .filter(|f| {
                if f.session_id != session_id {
                    return false;
                }
                if parent_path == "/" {
                    // Root level: files directly under /
                    f.path.starts_with('/') && !f.path[1..].contains('/')
                } else {
                    // Under specific directory
                    f.path.starts_with(&prefix) && !f.path[prefix.len()..].contains('/')
                }
            })
            .map(Self::file_to_info)
            .collect();
        result.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(result)
    }

    pub async fn list_all_session_files(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileInfoRow>> {
        let files = self.session_files.read();
        let mut result: Vec<_> = files
            .values()
            .filter(|f| f.session_id == session_id)
            .map(Self::file_to_info)
            .collect();
        result.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(result)
    }

    pub async fn update_session_file(
        &self,
        session_id: Uuid,
        path: &str,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        let mut files = self.session_files.write();
        if let Some(file) = files
            .values_mut()
            .find(|f| f.session_id == session_id && f.path == path)
        {
            if let Some(content) = input.content {
                file.size_bytes = content.len() as i64;
                file.content = Some(content);
            }
            if let Some(is_readonly) = input.is_readonly {
                file.is_readonly = is_readonly;
            }
            file.updated_at = Self::now();
            return Ok(Some(file.clone()));
        }
        Ok(None)
    }

    pub async fn update_session_file_if_content_matches(
        &self,
        session_id: Uuid,
        path: &str,
        expected_content: Vec<u8>,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        let mut files = self.session_files.write();
        if let Some(file) = files
            .values_mut()
            .find(|f| f.session_id == session_id && f.path == path)
        {
            if file.is_directory || file.is_readonly {
                return Ok(None);
            }

            let current_content = file.content.clone().unwrap_or_default();
            if current_content != expected_content {
                return Ok(None);
            }

            if let Some(content) = input.content {
                file.size_bytes = content.len() as i64;
                file.content = Some(content);
            }
            if let Some(is_readonly) = input.is_readonly {
                file.is_readonly = is_readonly;
            }
            file.updated_at = Self::now();
            return Ok(Some(file.clone()));
        }
        Ok(None)
    }

    pub async fn delete_session_file(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        let mut files = self.session_files.write();
        let to_remove: Option<Uuid> = files
            .iter()
            .find(|(_, f)| f.session_id == session_id && f.path == path)
            .map(|(id, _)| *id);

        if let Some(id) = to_remove {
            files.remove(&id);
            return Ok(true);
        }
        Ok(false)
    }

    pub async fn delete_session_file_recursive(&self, session_id: Uuid, path: &str) -> Result<u64> {
        let session_id = SessionId::from_uuid(session_id);
        let mut files = self.session_files.write();
        let prefix = format!("{}/", path.trim_end_matches('/'));

        let to_remove: Vec<Uuid> = files
            .iter()
            .filter(|(_, f)| {
                f.session_id == session_id && (f.path == path || f.path.starts_with(&prefix))
            })
            .map(|(id, _)| *id)
            .collect();

        let count = to_remove.len() as u64;
        for id in to_remove {
            files.remove(&id);
        }
        Ok(count)
    }

    pub async fn move_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        let session_id = SessionId::from_uuid(session_id);
        // Check if destination exists
        {
            let files = self.session_files.read();
            if files
                .values()
                .any(|f| f.session_id == session_id && f.path == dest_path)
            {
                return Err(anyhow!("Destination path already exists"));
            }
        }

        let mut files = self.session_files.write();
        if let Some(file) = files
            .values_mut()
            .find(|f| f.session_id == session_id && f.path == source_path)
        {
            file.path = dest_path.to_string();
            file.updated_at = Self::now();
            return Ok(Some(file.clone()));
        }
        Ok(None)
    }

    pub async fn copy_session_file(
        &self,
        session_id: Uuid,
        source_path: &str,
        dest_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        let session_id = SessionId::from_uuid(session_id);
        // Check if destination exists
        {
            let files = self.session_files.read();
            if files
                .values()
                .any(|f| f.session_id == session_id && f.path == dest_path)
            {
                return Err(anyhow!("Destination path already exists"));
            }
        }

        let source = {
            let files = self.session_files.read();
            files
                .values()
                .find(|f| f.session_id == session_id && f.path == source_path)
                .cloned()
        };

        if let Some(source) = source {
            let now = Self::now();
            let id = Uuid::now_v7();
            let new_file = SessionFileRow {
                id,
                session_id,
                path: dest_path.to_string(),
                content: source.content,
                is_directory: source.is_directory,
                is_readonly: source.is_readonly,
                size_bytes: source.size_bytes,
                created_at: now,
                updated_at: now,
            };
            self.session_files.write().insert(id, new_file.clone());
            return Ok(Some(new_file));
        }
        Ok(None)
    }

    pub async fn grep_session_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_prefix: Option<&str>,
    ) -> Result<Vec<SessionFileInfoRow>> {
        let session_id = SessionId::from_uuid(session_id);
        let regex = regex::Regex::new(pattern)?;
        let files = self.session_files.read();

        let result: Vec<_> = files
            .values()
            .filter(|f| {
                if f.session_id != session_id || f.is_directory {
                    return false;
                }
                if let Some(prefix) = path_prefix
                    && !f.path.starts_with(prefix)
                {
                    return false;
                }
                // Content is Vec<u8>, convert to str for regex matching
                f.content
                    .as_ref()
                    .and_then(|c| std::str::from_utf8(c).ok())
                    .map(|s| regex.is_match(s))
                    .unwrap_or(false)
            })
            .map(Self::file_to_info)
            .collect();

        Ok(result)
    }

    pub async fn session_file_exists(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        Ok(self
            .session_files
            .read()
            .values()
            .any(|f| f.session_id == session_id && f.path == path))
    }

    pub async fn session_directory_has_children(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        let prefix = format!("{}/", path.trim_end_matches('/'));
        Ok(self
            .session_files
            .read()
            .values()
            .any(|f| f.session_id == session_id && f.path.starts_with(&prefix)))
    }

    pub async fn has_readonly_session_files(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let session_id = SessionId::from_uuid(session_id);
        let files = self.session_files.read();

        if path == "/" {
            return Ok(files
                .values()
                .any(|f| f.session_id == session_id && f.is_readonly));
        }

        let prefix = format!("{}/", path.trim_end_matches('/'));
        Ok(files.values().any(|f| {
            f.session_id == session_id
                && (f.path == path || f.path.starts_with(&prefix))
                && f.is_readonly
        }))
    }

    /// Load all non-directory files with content for a session (single pass).
    pub async fn load_all_session_files_with_content(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileRow>> {
        let session_id = SessionId::from_uuid(session_id);
        let mut files: Vec<_> = self
            .session_files
            .read()
            .values()
            .filter(|f| f.session_id == session_id && !f.is_directory)
            .cloned()
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }
}
