// Database-backed SessionFileStore implementation
//
// This module implements the core SessionFileStore trait for persisting
// session files to the database.

use async_trait::async_trait;
use everruns_core::{
    traits::SessionFileStore, AgentLoopError, FileInfo, FileStat, GrepMatch, Result, SessionFile,
};
use regex::Regex;
use uuid::Uuid;

use super::models::{CreateSessionFileRow, UpdateSessionFile};
use super::repositories::Database;

// ============================================================================
// DbSessionFileStore - Stores session files in database
// ============================================================================

/// Database-backed session file store
///
/// Stores session files in the session_files table.
/// Used by tools that need to read/write the session's virtual filesystem.
#[derive(Clone)]
pub struct DbSessionFileStore {
    db: Database,
}

impl DbSessionFileStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Normalize a path: ensure it starts with /, no trailing slash, no double slashes
    fn normalize_path(path: &str) -> String {
        let mut normalized = path.trim().to_string();

        // Ensure starts with /
        if !normalized.starts_with('/') {
            normalized = format!("/{}", normalized);
        }

        // Remove trailing slash (except for root)
        if normalized.len() > 1 && normalized.ends_with('/') {
            normalized.pop();
        }

        // Remove double slashes
        while normalized.contains("//") {
            normalized = normalized.replace("//", "/");
        }

        normalized
    }

    /// Ensure a directory exists, creating it and parents if needed
    async fn ensure_directory_exists(&self, session_id: Uuid, path: &str) -> Result<()> {
        if path == "/" {
            return Ok(()); // Root always exists
        }

        // Check if directory exists
        if let Some(existing) = self.db.get_session_file(session_id, path).await? {
            if existing.is_directory {
                return Ok(());
            } else {
                return Err(AgentLoopError::store(format!(
                    "A file exists at path: {}",
                    path
                )));
            }
        }

        // Create parent first
        if let Some(parent) = FileInfo::parent_path(path) {
            Box::pin(self.ensure_directory_exists(session_id, &parent)).await?;
        }

        // Create this directory
        let input = CreateSessionFileRow {
            session_id,
            path: path.to_string(),
            content: None,
            is_directory: true,
            is_readonly: false,
        };

        self.db.create_session_file(input).await?;
        Ok(())
    }
}

#[async_trait]
impl SessionFileStore for DbSessionFileStore {
    async fn read_file(&self, session_id: Uuid, path: &str) -> Result<Option<SessionFile>> {
        let path = Self::normalize_path(path);
        let row = self
            .db
            .get_session_file(session_id, &path)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(row.map(|r| {
            let (content, encoding) = if let Some(bytes) = r.content {
                let (c, e) = SessionFile::encode_content(&bytes);
                (Some(c), e)
            } else {
                (None, "text".to_string())
            };

            SessionFile {
                id: r.id,
                session_id: r.session_id,
                path: r.path.clone(),
                name: FileInfo::name_from_path(&r.path),
                content,
                encoding,
                is_directory: r.is_directory,
                is_readonly: r.is_readonly,
                size_bytes: r.size_bytes,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }
        }))
    }

    async fn write_file(
        &self,
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let path = Self::normalize_path(path);

        // Decode content
        let bytes = SessionFile::decode_content(content, encoding)
            .map_err(|e| AgentLoopError::store(format!("Invalid content encoding: {}", e)))?;

        // Ensure parent directory exists
        if let Some(parent) = FileInfo::parent_path(&path) {
            self.ensure_directory_exists(session_id, &parent).await?;
        }

        // Check if file exists
        let existing = self
            .db
            .get_session_file(session_id, &path)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let row = if let Some(existing) = existing {
            // Update existing file
            if existing.is_directory {
                return Err(AgentLoopError::store(format!(
                    "Cannot write to directory: {}",
                    path
                )));
            }
            if existing.is_readonly {
                return Err(AgentLoopError::store(format!(
                    "Cannot modify readonly file: {}",
                    path
                )));
            }

            self.db
                .update_session_file(
                    session_id,
                    &path,
                    UpdateSessionFile {
                        content: Some(bytes),
                        is_readonly: None,
                    },
                )
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?
                .ok_or_else(|| AgentLoopError::store("File not found after update"))?
        } else {
            // Create new file
            let input = CreateSessionFileRow {
                session_id,
                path: path.clone(),
                content: Some(bytes),
                is_directory: false,
                is_readonly: false,
            };

            self.db
                .create_session_file(input)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?
        };

        // Convert row to SessionFile
        let (content, encoding) = if let Some(bytes) = row.content {
            let (c, e) = SessionFile::encode_content(&bytes);
            (Some(c), e)
        } else {
            (None, "text".to_string())
        };

        Ok(SessionFile {
            id: row.id,
            session_id: row.session_id,
            path: row.path.clone(),
            name: FileInfo::name_from_path(&row.path),
            content,
            encoding,
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    async fn delete_file(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool> {
        let path = Self::normalize_path(path);

        if path == "/" {
            if recursive {
                // Delete all files in session
                self.db
                    .delete_session_file_recursive(session_id, "/")
                    .await
                    .map_err(|e| AgentLoopError::store(e.to_string()))?;
                return Ok(true);
            } else {
                return Err(AgentLoopError::store(
                    "Cannot delete root directory without recursive flag",
                ));
            }
        }

        // Check if it's a directory with children
        let file = self
            .db
            .get_session_file(session_id, &path)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        if let Some(ref f) = file {
            if f.is_directory && !recursive {
                let has_children = self
                    .db
                    .session_directory_has_children(session_id, &path)
                    .await
                    .map_err(|e| AgentLoopError::store(e.to_string()))?;
                if has_children {
                    return Err(AgentLoopError::store(
                        "Directory is not empty. Use recursive=true to delete",
                    ));
                }
            }
        }

        if recursive {
            let deleted = self
                .db
                .delete_session_file_recursive(session_id, &path)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?;
            Ok(deleted > 0)
        } else {
            self.db
                .delete_session_file(session_id, &path)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))
        }
    }

    async fn list_directory(&self, session_id: Uuid, path: &str) -> Result<Vec<FileInfo>> {
        let path = Self::normalize_path(path);

        // Verify directory exists (root always exists)
        if path != "/" {
            let dir = self
                .db
                .get_session_file(session_id, &path)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?;
            match dir {
                Some(d) if !d.is_directory => {
                    return Err(AgentLoopError::store(format!(
                        "Path is not a directory: {}",
                        path
                    )))
                }
                None => {
                    return Err(AgentLoopError::store(format!(
                        "Directory not found: {}",
                        path
                    )))
                }
                _ => {}
            }
        }

        let rows = self
            .db
            .list_session_files(session_id, &path)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|r| FileInfo {
                id: r.id,
                session_id: r.session_id,
                path: r.path.clone(),
                name: FileInfo::name_from_path(&r.path),
                is_directory: r.is_directory,
                is_readonly: r.is_readonly,
                size_bytes: r.size_bytes,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect())
    }

    async fn stat_file(&self, session_id: Uuid, path: &str) -> Result<Option<FileStat>> {
        let path = Self::normalize_path(path);

        // Handle root directory specially
        if path == "/" {
            return Ok(Some(FileStat {
                path: "/".to_string(),
                name: "/".to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }));
        }

        let row = self
            .db
            .get_session_file(session_id, &path)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(row.map(|r| FileStat {
            path: r.path.clone(),
            name: FileInfo::name_from_path(&r.path),
            is_directory: r.is_directory,
            is_readonly: r.is_readonly,
            size_bytes: r.size_bytes,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }))
    }

    async fn grep_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        // Validate regex pattern
        let regex = Regex::new(pattern)
            .map_err(|e| AgentLoopError::store(format!("Invalid regex pattern: {}", e)))?;

        // Get matching files from database
        let files = self
            .db
            .grep_session_files(session_id, pattern, path_pattern)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        let mut results = Vec::new();

        // For each matching file, find the actual line matches
        for file_info in files {
            // Read full file content
            let file = self
                .db
                .get_session_file(session_id, &file_info.path)
                .await
                .map_err(|e| AgentLoopError::store(e.to_string()))?;

            if let Some(f) = file {
                if let Some(content) = f.content {
                    // Try to decode as text
                    if let Ok(text) = String::from_utf8(content) {
                        for (i, line) in text.lines().enumerate() {
                            if regex.is_match(line) {
                                results.push(GrepMatch {
                                    path: file_info.path.clone(),
                                    line_number: i + 1,
                                    line: line.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    async fn create_directory(&self, session_id: Uuid, path: &str) -> Result<FileInfo> {
        let path = Self::normalize_path(path);

        // Check if already exists
        if let Some(existing) = self
            .db
            .get_session_file(session_id, &path)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?
        {
            if existing.is_directory {
                return Ok(FileInfo {
                    id: existing.id,
                    session_id: existing.session_id,
                    path: existing.path.clone(),
                    name: FileInfo::name_from_path(&existing.path),
                    is_directory: existing.is_directory,
                    is_readonly: existing.is_readonly,
                    size_bytes: existing.size_bytes,
                    created_at: existing.created_at,
                    updated_at: existing.updated_at,
                });
            } else {
                return Err(AgentLoopError::store(format!(
                    "A file exists at path: {}",
                    path
                )));
            }
        }

        // Create parent directories recursively
        if let Some(parent) = FileInfo::parent_path(&path) {
            self.ensure_directory_exists(session_id, &parent).await?;
        }

        let input = CreateSessionFileRow {
            session_id,
            path: path.clone(),
            content: None,
            is_directory: true,
            is_readonly: false,
        };

        let row = self
            .db
            .create_session_file(input)
            .await
            .map_err(|e| AgentLoopError::store(e.to_string()))?;

        Ok(FileInfo {
            id: row.id,
            session_id: row.session_id,
            path: row.path.clone(),
            name: FileInfo::name_from_path(&row.path),
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

// ============================================================================
// Factory functions
// ============================================================================

/// Create a database-backed session file store
pub fn create_db_session_file_store(db: Database) -> DbSessionFileStore {
    DbSessionFileStore::new(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(DbSessionFileStore::normalize_path("/foo"), "/foo");
        assert_eq!(DbSessionFileStore::normalize_path("foo"), "/foo");
        assert_eq!(DbSessionFileStore::normalize_path("/foo/"), "/foo");
        assert_eq!(DbSessionFileStore::normalize_path("/foo//bar"), "/foo/bar");
        assert_eq!(DbSessionFileStore::normalize_path("/"), "/");
        assert_eq!(DbSessionFileStore::normalize_path(""), "/");
    }
}
