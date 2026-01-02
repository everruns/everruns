// Session Files service for virtual filesystem operations

use crate::storage::{
    models::{CreateSessionFileRow, SessionFileInfoRow, SessionFileRow, UpdateSessionFile},
    Database,
};
use anyhow::{anyhow, Result};
use everruns_core::{FileInfo, FileStat, GrepMatch, GrepResult, SessionFile};
use regex::Regex;
use std::sync::Arc;
use uuid::Uuid;

/// Input for creating a file
pub struct CreateFileInput {
    pub path: String,
    pub content: Option<String>,
    pub encoding: Option<String>,
    pub is_readonly: Option<bool>,
}

/// Input for creating a directory
pub struct CreateDirectoryInput {
    pub path: String,
}

/// Input for updating a file
pub struct UpdateFileInput {
    pub content: Option<String>,
    pub encoding: Option<String>,
    pub is_readonly: Option<bool>,
}

/// Input for moving a file
pub struct MoveFileInput {
    pub src_path: String,
    pub dst_path: String,
}

/// Input for copying a file
pub struct CopyFileInput {
    pub src_path: String,
    pub dst_path: String,
}

/// Input for grep search
pub struct GrepInput {
    pub pattern: String,
    pub path_pattern: Option<String>,
}

pub struct SessionFileService {
    db: Arc<Database>,
}

impl SessionFileService {
    pub fn new(db: Arc<Database>) -> Self {
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

    /// Validate that a path is valid
    fn validate_path(path: &str) -> Result<()> {
        if path.is_empty() {
            return Err(anyhow!("Path cannot be empty"));
        }

        if !path.starts_with('/') {
            return Err(anyhow!("Path must start with /"));
        }

        // Check for invalid characters
        if path.contains('\0') {
            return Err(anyhow!("Path cannot contain null characters"));
        }

        // Check for .. path traversal
        if path.split('/').any(|segment| segment == "..") {
            return Err(anyhow!("Path cannot contain '..' segments"));
        }

        Ok(())
    }

    /// Create a new file
    pub async fn create_file(&self, session_id: Uuid, req: CreateFileInput) -> Result<SessionFile> {
        let path = Self::normalize_path(&req.path);
        Self::validate_path(&path)?;

        // Decode content if provided
        let content = if let Some(ref content_str) = req.content {
            let encoding = req.encoding.as_deref().unwrap_or("text");
            Some(SessionFile::decode_content(content_str, encoding)?)
        } else {
            None
        };

        // Ensure parent directory exists (create recursively if needed)
        if let Some(parent) = FileInfo::parent_path(&path) {
            self.ensure_directory_exists(session_id, &parent).await?;
        }

        // Check if file already exists
        if self.db.session_file_exists(session_id, &path).await? {
            return Err(anyhow!("File already exists at path: {}", path));
        }

        let input = CreateSessionFileRow {
            session_id,
            path: path.clone(),
            content,
            is_directory: false,
            is_readonly: req.is_readonly.unwrap_or(false),
        };

        let row = self.db.create_session_file(input).await?;
        Ok(Self::row_to_session_file(row))
    }

    /// Create a directory (and parent directories if needed)
    pub async fn create_directory(
        &self,
        session_id: Uuid,
        req: CreateDirectoryInput,
    ) -> Result<FileInfo> {
        let path = Self::normalize_path(&req.path);
        Self::validate_path(&path)?;

        // Check if already exists
        if let Some(existing) = self.db.get_session_file(session_id, &path).await? {
            if existing.is_directory {
                return Ok(Self::row_to_file_info(existing));
            } else {
                return Err(anyhow!("A file exists at path: {}", path));
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

        let row = self.db.create_session_file(input).await?;
        Ok(Self::row_to_file_info(row))
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
                return Err(anyhow!("A file exists at path: {}", path));
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

    /// Read a file
    pub async fn read_file(&self, session_id: Uuid, path: &str) -> Result<Option<SessionFile>> {
        let path = Self::normalize_path(path);
        let row = self.db.get_session_file(session_id, &path).await?;
        Ok(row.map(Self::row_to_session_file))
    }

    /// Get file stat (metadata)
    pub async fn stat(&self, session_id: Uuid, path: &str) -> Result<Option<FileStat>> {
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

        let row = self.db.get_session_file(session_id, &path).await?;
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

    /// List directory contents
    pub async fn list_directory(&self, session_id: Uuid, path: &str) -> Result<Vec<FileInfo>> {
        let path = Self::normalize_path(path);

        // Verify directory exists (root always exists)
        if path != "/" {
            let dir = self.db.get_session_file(session_id, &path).await?;
            match dir {
                Some(d) if !d.is_directory => {
                    return Err(anyhow!("Path is not a directory: {}", path))
                }
                None => return Err(anyhow!("Directory not found: {}", path)),
                _ => {}
            }
        }

        let rows = self.db.list_session_files(session_id, &path).await?;
        Ok(rows
            .into_iter()
            .map(Self::row_to_file_info_from_info)
            .collect())
    }

    /// List all files recursively
    pub async fn list_all(&self, session_id: Uuid) -> Result<Vec<FileInfo>> {
        let rows = self.db.list_all_session_files(session_id).await?;
        Ok(rows
            .into_iter()
            .map(Self::row_to_file_info_from_info)
            .collect())
    }

    /// Update a file
    pub async fn update_file(
        &self,
        session_id: Uuid,
        path: &str,
        req: UpdateFileInput,
    ) -> Result<Option<SessionFile>> {
        let path = Self::normalize_path(path);

        // Check if file exists and is not readonly
        if let Some(existing) = self.db.get_session_file(session_id, &path).await? {
            if existing.is_directory {
                return Err(anyhow!("Cannot update directory: {}", path));
            }
            if existing.is_readonly && req.content.is_some() {
                return Err(anyhow!("Cannot modify readonly file: {}", path));
            }
        }

        // Decode content if provided
        let content = if let Some(ref content_str) = req.content {
            let encoding = req.encoding.as_deref().unwrap_or("text");
            Some(SessionFile::decode_content(content_str, encoding)?)
        } else {
            None
        };

        let input = UpdateSessionFile {
            content,
            is_readonly: req.is_readonly,
        };

        let row = self
            .db
            .update_session_file(session_id, &path, input)
            .await?;
        Ok(row.map(Self::row_to_session_file))
    }

    /// Delete a file or directory
    pub async fn delete(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool> {
        let path = Self::normalize_path(path);

        if path == "/" {
            if recursive {
                // Delete all files in session
                self.db
                    .delete_session_file_recursive(session_id, "/")
                    .await?;
                return Ok(true);
            } else {
                return Err(anyhow!(
                    "Cannot delete root directory without recursive flag"
                ));
            }
        }

        // Check if it's a directory with children
        let file = self.db.get_session_file(session_id, &path).await?;
        if let Some(ref f) = file {
            if f.is_directory && !recursive {
                let has_children = self
                    .db
                    .session_directory_has_children(session_id, &path)
                    .await?;
                if has_children {
                    return Err(anyhow!(
                        "Directory is not empty. Use recursive=true to delete"
                    ));
                }
            }
        }

        if recursive {
            let deleted = self
                .db
                .delete_session_file_recursive(session_id, &path)
                .await?;
            Ok(deleted > 0)
        } else {
            self.db.delete_session_file(session_id, &path).await
        }
    }

    /// Move/rename a file or directory
    pub async fn move_file(
        &self,
        session_id: Uuid,
        req: MoveFileInput,
    ) -> Result<Option<SessionFile>> {
        let src_path = Self::normalize_path(&req.src_path);
        let dst_path = Self::normalize_path(&req.dst_path);

        Self::validate_path(&dst_path)?;

        // Check source exists
        let source = self.db.get_session_file(session_id, &src_path).await?;
        if source.is_none() {
            return Err(anyhow!("Source not found: {}", src_path));
        }

        // Check destination doesn't exist
        if self.db.session_file_exists(session_id, &dst_path).await? {
            return Err(anyhow!("Destination already exists: {}", dst_path));
        }

        // Ensure destination parent exists
        if let Some(parent) = FileInfo::parent_path(&dst_path) {
            self.ensure_directory_exists(session_id, &parent).await?;
        }

        let row = self
            .db
            .move_session_file(session_id, &src_path, &dst_path)
            .await?;
        Ok(row.map(Self::row_to_session_file))
    }

    /// Copy a file
    pub async fn copy_file(
        &self,
        session_id: Uuid,
        req: CopyFileInput,
    ) -> Result<Option<SessionFile>> {
        let src_path = Self::normalize_path(&req.src_path);
        let dst_path = Self::normalize_path(&req.dst_path);

        Self::validate_path(&dst_path)?;

        // Check source exists and is not a directory
        let source = self.db.get_session_file(session_id, &src_path).await?;
        match source {
            None => return Err(anyhow!("Source not found: {}", src_path)),
            Some(ref s) if s.is_directory => {
                return Err(anyhow!("Cannot copy directories: {}", src_path))
            }
            _ => {}
        }

        // Check destination doesn't exist
        if self.db.session_file_exists(session_id, &dst_path).await? {
            return Err(anyhow!("Destination already exists: {}", dst_path));
        }

        // Ensure destination parent exists
        if let Some(parent) = FileInfo::parent_path(&dst_path) {
            self.ensure_directory_exists(session_id, &parent).await?;
        }

        let row = self
            .db
            .copy_session_file(session_id, &src_path, &dst_path)
            .await?;
        Ok(row.map(Self::row_to_session_file))
    }

    /// Search files using grep-like pattern matching
    pub async fn grep(&self, session_id: Uuid, req: GrepInput) -> Result<Vec<GrepResult>> {
        // Validate regex pattern
        let regex = Regex::new(&req.pattern)?;

        // Get matching files from database
        let files = self
            .db
            .grep_session_files(session_id, &req.pattern, req.path_pattern.as_deref())
            .await?;

        let mut results = Vec::new();

        // For each matching file, find the actual line matches
        for file_info in files {
            // Read full file content
            let file = self
                .db
                .get_session_file(session_id, &file_info.path)
                .await?;
            if let Some(f) = file {
                if let Some(content) = f.content {
                    // Try to decode as text
                    if let Ok(text) = String::from_utf8(content) {
                        let matches: Vec<GrepMatch> = text
                            .lines()
                            .enumerate()
                            .filter(|(_, line)| regex.is_match(line))
                            .map(|(i, line)| GrepMatch {
                                path: file_info.path.clone(),
                                line_number: i + 1,
                                line: line.to_string(),
                            })
                            .collect();

                        if !matches.is_empty() {
                            results.push(GrepResult {
                                path: file_info.path.clone(),
                                matches,
                            });
                        }
                    }
                }
            }
        }

        Ok(results)
    }

    fn row_to_session_file(row: SessionFileRow) -> SessionFile {
        let (content, encoding) = if let Some(bytes) = row.content {
            let (c, e) = SessionFile::encode_content(&bytes);
            (Some(c), e)
        } else {
            (None, "text".to_string())
        };

        SessionFile {
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
        }
    }

    fn row_to_file_info(row: SessionFileRow) -> FileInfo {
        FileInfo {
            id: row.id,
            session_id: row.session_id,
            path: row.path.clone(),
            name: FileInfo::name_from_path(&row.path),
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    fn row_to_file_info_from_info(row: SessionFileInfoRow) -> FileInfo {
        FileInfo {
            id: row.id,
            session_id: row.session_id,
            path: row.path.clone(),
            name: FileInfo::name_from_path(&row.path),
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
