// Session Files service for virtual filesystem operations
//
// Design Decision: Capability mounts are applied at session creation time.
// This ensures mount points are available immediately when the session starts,
// rather than waiting until execution time. The apply_capability_mounts()
// method recursively creates files and directories from mount point definitions.

use crate::storage::{
    StorageBackend,
    models::{CreateSessionFileRow, SessionFileInfoRow, SessionFileRow, UpdateSessionFile},
};
use anyhow::{Result, anyhow};
use everruns_core::{
    FileInfo, FileStat, GrepMatch, GrepResult, MountAccess, MountEntry, MountPoint, MountSource,
    SessionFile, SessionId,
};
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
    db: Arc<StorageBackend>,
    virtual_registry: Option<Arc<crate::services::virtual_mount_registry::VirtualMountRegistry>>,
}

impl SessionFileService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            virtual_registry: None,
        }
    }

    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::services::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.virtual_registry = Some(registry);
        self
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

        // Fast-path check (non-racy for in-memory; Postgres constraint catches races)
        if self.db.session_file_exists(session_id, &path).await? {
            return Err(anyhow!("File already exists at path: {}", path));
        }

        let input = CreateSessionFileRow {
            session_id: SessionId::from_uuid(session_id),
            path: path.clone(),
            content,
            is_directory: false,
            is_readonly: req.is_readonly.unwrap_or(false),
        };

        let row = self.db.create_session_file(input).await.map_err(|e| {
            let msg = e.to_string();
            if msg.contains("duplicate key")
                || msg.contains("unique constraint")
                || msg.contains("UNIQUE constraint")
            {
                anyhow!("File already exists at path: {}", path)
            } else {
                e
            }
        })?;
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
            session_id: SessionId::from_uuid(session_id),
            path: path.clone(),
            content: None,
            is_directory: true,
            is_readonly: false,
        };

        let row = match self.db.create_session_file(input).await {
            Ok(row) => row,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("duplicate key")
                    || msg.contains("unique constraint")
                    || msg.contains("UNIQUE constraint")
                {
                    // Race: another request created it concurrently; return existing
                    if let Some(existing) = self.db.get_session_file(session_id, &path).await?
                        && existing.is_directory
                    {
                        return Ok(Self::row_to_file_info(existing));
                    }
                    return Err(anyhow!("A file exists at path: {}", path));
                }
                return Err(e);
            }
        };
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
            session_id: SessionId::from_uuid(session_id),
            path: path.to_string(),
            content: None,
            is_directory: true,
            is_readonly: false,
        };

        match self.db.create_session_file(input).await {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("duplicate key")
                    || msg.contains("unique constraint")
                    || msg.contains("UNIQUE constraint")
                {
                    // Race: another request created it concurrently; check it's a directory
                    if let Some(existing) = self.db.get_session_file(session_id, path).await?
                        && existing.is_directory
                    {
                        return Ok(());
                    }
                    Err(anyhow!("A file exists at path: {}", path))
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Read a file
    pub async fn read_file(&self, session_id: Uuid, path: &str) -> Result<Option<SessionFile>> {
        let path = Self::normalize_path(path);

        // Check virtual mounts first
        if let Some(registry) = &self.virtual_registry
            && let Some(vf) = registry.read_file(&session_id, &path)
        {
            let now = chrono::Utc::now();
            let (content, encoding) = if vf.is_directory {
                (None, "text".to_string())
            } else {
                let (c, e) = SessionFile::encode_content(&vf.content);
                (Some(c), e)
            };
            return Ok(Some(SessionFile {
                id: uuid::Uuid::nil(),
                session_id,
                path: vf.path.clone(),
                name: FileInfo::name_from_path(&vf.path),
                content,
                encoding,
                is_directory: vf.is_directory,
                is_readonly: true,
                size_bytes: vf.content.len() as i64,
                created_at: now,
                updated_at: now,
            }));
        }

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

        // Check virtual mounts first
        if let Some(registry) = &self.virtual_registry
            && let Some(vf) = registry.read_file(&session_id, &path)
        {
            return Ok(Some(FileStat {
                path: vf.path.clone(),
                name: FileInfo::name_from_path(&vf.path),
                is_directory: vf.is_directory,
                is_readonly: true,
                size_bytes: vf.content.len() as i64,
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

        // Check if directory exists in virtual mounts
        let virtual_dir_exists = self.virtual_registry.as_ref().is_some_and(|r| {
            r.read_file(&session_id, &path)
                .is_some_and(|f| f.is_directory)
        });

        // Verify directory exists (root always exists)
        if path != "/" && !virtual_dir_exists {
            let dir = self.db.get_session_file(session_id, &path).await?;
            match dir {
                Some(d) if !d.is_directory => {
                    return Err(anyhow!("Path is not a directory: {}", path));
                }
                None => return Err(anyhow!("Directory not found: {}", path)),
                _ => {}
            }
        }

        let rows = self.db.list_session_files(session_id, &path).await?;
        let mut entries: Vec<FileInfo> = rows
            .into_iter()
            .map(Self::row_to_file_info_from_info)
            .collect();

        // Merge virtual entries (virtual wins on name conflict)
        if let Some(registry) = &self.virtual_registry {
            let virtual_entries = registry.list_directory(&session_id, &path);
            let now = chrono::Utc::now();
            for vf in virtual_entries {
                let name = FileInfo::name_from_path(&vf.path);
                // Remove any DB entry with the same name so virtual wins
                entries.retain(|e| e.name != name);
                entries.push(FileInfo {
                    id: uuid::Uuid::nil(),
                    session_id,
                    path: vf.path,
                    name,
                    is_directory: vf.is_directory,
                    is_readonly: true,
                    size_bytes: vf.size_bytes,
                    created_at: now,
                    updated_at: now,
                });
            }
        }

        // Sort to match DB ordering: directories first, then by path
        entries.sort_by(|a, b| {
            b.is_directory
                .cmp(&a.is_directory)
                .then_with(|| a.path.cmp(&b.path))
        });

        Ok(entries)
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

        // Virtual files cannot be modified
        if let Some(registry) = &self.virtual_registry
            && registry.is_virtual_path(&session_id, &path)
        {
            return Err(anyhow!("Cannot modify readonly file: {}", path));
        }

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

    /// Update a file only if its content still matches the provided snapshot.
    pub async fn update_file_if_content_matches(
        &self,
        session_id: Uuid,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        let path = Self::normalize_path(path);

        let expected_bytes = SessionFile::decode_content(expected_content, expected_encoding)?;
        let content = SessionFile::decode_content(content, encoding)?;

        let row = self
            .db
            .update_session_file_if_content_matches(
                session_id,
                &path,
                expected_bytes,
                UpdateSessionFile {
                    content: Some(content),
                    is_readonly: None,
                },
            )
            .await?;
        Ok(row.map(Self::row_to_session_file))
    }

    /// Delete a file or directory
    pub async fn delete(&self, session_id: Uuid, path: &str, recursive: bool) -> Result<bool> {
        let path = Self::normalize_path(path);

        if path == "/" {
            if recursive {
                // Check for readonly files before deleting all
                let has_readonly = self.db.has_readonly_session_files(session_id, "/").await?;
                if has_readonly {
                    return Err(anyhow!("Cannot delete: directory contains readonly files"));
                }
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

        // Check if file/directory exists
        let file = self.db.get_session_file(session_id, &path).await?;

        // Check readonly on the target itself
        if let Some(ref f) = file
            && f.is_readonly
        {
            return Err(anyhow!("Cannot delete readonly file: {}", path));
        }

        if let Some(ref f) = file
            && f.is_directory
            && !recursive
        {
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

        if recursive {
            // Check for readonly files in subtree before deleting
            let has_readonly = self
                .db
                .has_readonly_session_files(session_id, &path)
                .await?;
            if has_readonly {
                return Err(anyhow!("Cannot delete: directory contains readonly files"));
            }
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
                return Err(anyhow!("Cannot copy directories: {}", src_path));
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

    /// Search files using grep-like pattern matching (delegates to shared helper).
    pub async fn grep(&self, session_id: Uuid, req: GrepInput) -> Result<Vec<GrepResult>> {
        grep_session_files(
            &self.db,
            session_id,
            &req.pattern,
            req.path_pattern.as_deref(),
        )
        .await
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
            session_id: row.session_id.uuid(),
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
            session_id: row.session_id.uuid(),
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
            session_id: row.session_id.uuid(),
            path: row.path.clone(),
            name: FileInfo::name_from_path(&row.path),
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    // =========================================================================
    // Capability Mount Methods
    // =========================================================================

    /// Apply capability mounts to a session filesystem.
    ///
    /// This method creates files and directories in the session filesystem
    /// based on the mount points defined by capabilities. Each mount point
    /// specifies a path, access mode (readonly/readwrite), and content source.
    ///
    /// Files created from readonly mounts are marked as readonly and cannot
    /// be modified or deleted by agents.
    pub async fn apply_capability_mounts(
        &self,
        session_id: Uuid,
        mounts: &[MountPoint],
    ) -> Result<MountApplicationResult> {
        let mut result = MountApplicationResult::default();

        for mount in mounts {
            match self.apply_single_mount(session_id, mount).await {
                Ok(stats) => {
                    result.files_created += stats.files_created;
                    result.directories_created += stats.directories_created;
                    result.mount_points_applied += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        session_id = %session_id,
                        mount_path = %mount.path,
                        capability_id = %mount.capability_id,
                        error = %e,
                        "Failed to apply capability mount"
                    );
                    result.errors.push(MountError {
                        path: mount.path.clone(),
                        capability_id: mount.capability_id.clone(),
                        error: e.to_string(),
                    });
                }
            }
        }

        Ok(result)
    }

    /// Apply a single mount point to the session filesystem.
    async fn apply_single_mount(&self, session_id: Uuid, mount: &MountPoint) -> Result<MountStats> {
        let path = Self::normalize_path(&mount.path);
        Self::validate_path(&path)?;

        let is_readonly = mount.is_readonly();
        let mut stats = MountStats::default();

        // Virtual mounts are registered in-memory, not written to DB
        if let MountSource::Virtual { tree } = &mount.source {
            if mount.access == MountAccess::ReadWrite {
                return Err(anyhow!(
                    "Virtual mounts are always read-only; mount at '{}' has ReadWrite access",
                    mount.path
                ));
            }
            if let Some(registry) = &self.virtual_registry {
                registry.register(session_id, path, tree.clone(), mount.capability_id.clone());
                stats.files_created += tree.len();
            } else {
                return Err(anyhow!(
                    "virtual mount at '{}' for capability '{}' cannot be applied: no virtual registry configured",
                    mount.path,
                    mount.capability_id
                ));
            }
            return Ok(stats);
        }

        self.apply_mount_source(session_id, &path, &mount.source, is_readonly, &mut stats)
            .await?;

        Ok(stats)
    }

    /// Recursively apply a mount source to the filesystem.
    fn apply_mount_source<'a>(
        &'a self,
        session_id: Uuid,
        path: &'a str,
        source: &'a MountSource,
        is_readonly: bool,
        stats: &'a mut MountStats,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match source {
                MountSource::InlineFile { content, encoding } => {
                    self.create_mounted_file(session_id, path, content, encoding, is_readonly)
                        .await?;
                    stats.files_created += 1;
                }
                MountSource::InlineDirectory { entries } => {
                    // Create the directory first
                    self.create_mounted_directory(session_id, path, is_readonly)
                        .await?;
                    stats.directories_created += 1;

                    // Recursively create children
                    for (name, entry) in entries {
                        let child_path = format!("{}/{}", path, name);
                        self.apply_mount_entry(session_id, &child_path, entry, is_readonly, stats)
                            .await?;
                    }
                }
                MountSource::Virtual { .. } => {
                    // Virtual mounts are only supported as top-level mount roots.
                    return Err(anyhow!(
                        "Virtual mounts are only supported at the mount root"
                    ));
                }
            }
            Ok(())
        })
    }

    /// Apply a single mount entry (recursive helper).
    fn apply_mount_entry<'a>(
        &'a self,
        session_id: Uuid,
        path: &'a str,
        entry: &'a MountEntry,
        is_readonly: bool,
        stats: &'a mut MountStats,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.apply_mount_source(session_id, path, &entry.source, is_readonly, stats)
                .await
        })
    }

    /// Create a file from a mount source.
    async fn create_mounted_file(
        &self,
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
        is_readonly: bool,
    ) -> Result<()> {
        // Check if file already exists
        if self.db.session_file_exists(session_id, path).await? {
            return Err(anyhow!("Mount conflict: file already exists at {}", path));
        }

        // Ensure parent directory exists
        if let Some(parent) = FileInfo::parent_path(path) {
            self.ensure_directory_exists(session_id, &parent).await?;
        }

        // Decode content
        let content_bytes = SessionFile::decode_content(content, encoding)?;

        let input = CreateSessionFileRow {
            session_id: SessionId::from_uuid(session_id),
            path: path.to_string(),
            content: Some(content_bytes),
            is_directory: false,
            is_readonly,
        };

        self.db.create_session_file(input).await?;
        Ok(())
    }

    /// Create a directory for a mount (if it doesn't exist).
    ///
    /// When is_readonly is true, the directory is marked readonly, preventing
    /// creation of new files within it (only mount-time creation is allowed).
    async fn create_mounted_directory(
        &self,
        session_id: Uuid,
        path: &str,
        is_readonly: bool,
    ) -> Result<()> {
        // Check if path exists
        if let Some(existing) = self.db.get_session_file(session_id, path).await? {
            if existing.is_directory {
                return Ok(()); // Directory already exists, that's fine
            } else {
                return Err(anyhow!(
                    "Mount conflict: file exists at directory path {}",
                    path
                ));
            }
        }

        // Ensure parent directory exists
        if let Some(parent) = FileInfo::parent_path(path) {
            self.ensure_directory_exists(session_id, &parent).await?;
        }

        let input = CreateSessionFileRow {
            session_id: SessionId::from_uuid(session_id),
            path: path.to_string(),
            content: None,
            is_directory: true,
            is_readonly,
        };

        self.db.create_session_file(input).await?;
        Ok(())
    }
}

/// Max regex pattern length to prevent resource-intensive compilation (TM-DOS-008).
const MAX_GREP_PATTERN_LEN: usize = 1000;

/// Search session files using grep-like regex pattern matching.
///
/// Shared logic used by both `SessionFileService::grep` and
/// `DirectWorkerAdapters::grep_files`. Enforces TM-DOS-008 pattern length
/// limit to prevent expensive regex compilation.
pub async fn grep_session_files(
    db: &StorageBackend,
    session_id: Uuid,
    pattern: &str,
    path_pattern: Option<&str>,
) -> Result<Vec<GrepResult>> {
    // TM-DOS-008: Limit pattern length to prevent expensive regex compilation.
    // Note: Rust's `regex` crate already prevents catastrophic backtracking
    // (uses Thompson NFA), but compilation cost is O(n) in pattern length.
    anyhow::ensure!(
        pattern.len() <= MAX_GREP_PATTERN_LEN,
        "Regex pattern too long (max {} characters)",
        MAX_GREP_PATTERN_LEN
    );

    let regex = Regex::new(pattern)?;

    // Get matching files from database
    let files = db
        .grep_session_files(session_id, pattern, path_pattern)
        .await?;

    let mut results = Vec::new();

    // For each matching file, find the actual line matches
    for file_info in files {
        // Read full file content
        let file = db.get_session_file(session_id, &file_info.path).await?;
        if let Some(f) = file
            && let Some(content) = f.content
            && let Ok(text) = String::from_utf8(content)
        {
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

    Ok(results)
}

/// Result of applying capability mounts to a session.
#[derive(Debug, Clone, Default)]
pub struct MountApplicationResult {
    /// Number of files created
    pub files_created: usize,
    /// Number of directories created
    pub directories_created: usize,
    /// Number of mount points successfully applied
    pub mount_points_applied: usize,
    /// Errors encountered during mount application
    pub errors: Vec<MountError>,
}

impl MountApplicationResult {
    /// Check if all mounts were applied successfully
    pub fn is_success(&self) -> bool {
        self.errors.is_empty()
    }
}

/// Error during mount application
#[derive(Debug, Clone)]
pub struct MountError {
    /// Path that failed
    pub path: String,
    /// Capability that provided the mount
    pub capability_id: String,
    /// Error message
    pub error: String,
}

/// Statistics for a single mount application
#[derive(Debug, Clone, Default)]
struct MountStats {
    files_created: usize,
    directories_created: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::models::CreateSessionFileRow;
    use std::sync::Arc;

    /// Seed a text file into the in-memory store.
    async fn seed_file(db: &StorageBackend, session_id: Uuid, path: &str, content: &str) {
        db.create_session_file(CreateSessionFileRow {
            session_id: SessionId::from_uuid(session_id),
            path: path.to_string(),
            content: Some(content.as_bytes().to_vec()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn grep_session_files_returns_matching_lines() {
        let db = StorageBackend::in_memory();
        let sid = Uuid::new_v4();
        seed_file(&db, sid, "/src/main.rs", "fn main() {\n    hello();\n}\n").await;

        let results = grep_session_files(&db, sid, "hello", None).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches.len(), 1);
        assert_eq!(results[0].matches[0].line_number, 2);
        assert!(results[0].matches[0].line.contains("hello"));
    }

    #[tokio::test]
    async fn grep_session_files_returns_empty_for_no_match() {
        let db = StorageBackend::in_memory();
        let sid = Uuid::new_v4();
        seed_file(
            &db,
            sid,
            "/src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }",
        )
        .await;

        let results = grep_session_files(&db, sid, "nonexistent_pattern", None)
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn grep_session_files_supports_regex() {
        let db = StorageBackend::in_memory();
        let sid = Uuid::new_v4();
        seed_file(&db, sid, "/data.txt", "line 100\nline abc\nline 200\n").await;

        let results = grep_session_files(&db, sid, r"\d{3}", None).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].matches.len(), 2);
    }

    #[tokio::test]
    async fn grep_session_files_rejects_invalid_regex() {
        let db = StorageBackend::in_memory();
        let sid = Uuid::new_v4();

        let result = grep_session_files(&db, sid, "[invalid", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_duplicate_file_returns_already_exists_error() {
        let db = StorageBackend::in_memory();
        let sid = Uuid::new_v4();
        let svc = SessionFileService::new(Arc::new(db.clone()));

        let input = CreateFileInput {
            path: "/test.txt".to_string(),
            content: Some("hello".to_string()),
            encoding: None,
            is_readonly: None,
        };
        svc.create_file(sid, input).await.unwrap();

        let input2 = CreateFileInput {
            path: "/test.txt".to_string(),
            content: Some("world".to_string()),
            encoding: None,
            is_readonly: None,
        };
        let err = svc.create_file(sid, input2).await.unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "Expected 'already exists' in: {err}"
        );
    }

    #[tokio::test]
    async fn grep_session_files_rejects_overlong_pattern() {
        let db = StorageBackend::in_memory();
        let sid = Uuid::new_v4();

        let long_pattern = "a".repeat(MAX_GREP_PATTERN_LEN + 1);
        let result = grep_session_files(&db, sid, &long_pattern, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("too long"), "Expected 'too long' in: {err}");
    }

    #[tokio::test]
    async fn update_file_if_content_matches_updates_when_snapshot_matches() {
        let db = StorageBackend::in_memory();
        let sid = Uuid::new_v4();
        let svc = SessionFileService::new(Arc::new(db.clone()));
        seed_file(&db, sid, "/notes.txt", "hello").await;

        let updated = svc
            .update_file_if_content_matches(sid, "/notes.txt", "hello", "text", "goodbye", "text")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.content.as_deref(), Some("goodbye"));
    }

    #[tokio::test]
    async fn update_file_if_content_matches_rejects_stale_snapshot() {
        let db = StorageBackend::in_memory();
        let sid = Uuid::new_v4();
        let svc = SessionFileService::new(Arc::new(db.clone()));
        seed_file(&db, sid, "/notes.txt", "hello").await;

        let updated = svc
            .update_file_if_content_matches(sid, "/notes.txt", "stale", "text", "goodbye", "text")
            .await
            .unwrap();

        assert!(updated.is_none());
    }

    // ===== Virtual mount tests =====

    use crate::services::virtual_mount_registry::VirtualMountRegistry;
    use everruns_core::capability_types::VirtualFileTree;

    fn make_virtual_svc() -> (SessionFileService, Arc<VirtualMountRegistry>, Uuid) {
        let db = StorageBackend::in_memory();
        let registry = Arc::new(VirtualMountRegistry::new());
        let svc = SessionFileService::new(Arc::new(db)).with_virtual_registry(registry.clone());
        let sid = Uuid::new_v4();
        (svc, registry, sid)
    }

    fn sample_tree() -> Arc<VirtualFileTree> {
        let mut tree = VirtualFileTree::new();
        tree.insert_text("/docs/readme.md", "# Hello");
        tree.insert_text("/docs/guide.md", "Guide content");
        Arc::new(tree)
    }

    #[tokio::test]
    async fn virtual_read_file_returns_content() {
        let (svc, registry, sid) = make_virtual_svc();
        registry.register(sid, "/docs".into(), sample_tree(), "test_cap".into());

        let file = svc
            .read_file(sid, "/docs/readme.md")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(file.content.as_deref(), Some("# Hello"));
        assert!(file.is_readonly);
        assert!(!file.is_directory);
    }

    #[tokio::test]
    async fn virtual_read_directory_returns_none_content() {
        let (svc, registry, sid) = make_virtual_svc();
        registry.register(sid, "/docs".into(), sample_tree(), "test_cap".into());

        let dir = svc.read_file(sid, "/docs").await.unwrap().unwrap();
        assert!(dir.is_directory);
        assert!(dir.content.is_none());
    }

    #[tokio::test]
    async fn virtual_stat_returns_metadata() {
        let (svc, registry, sid) = make_virtual_svc();
        registry.register(sid, "/docs".into(), sample_tree(), "test_cap".into());

        let stat = svc.stat(sid, "/docs/readme.md").await.unwrap().unwrap();
        assert!(!stat.is_directory);
        assert!(stat.is_readonly);
        assert!(stat.size_bytes > 0);
    }

    #[tokio::test]
    async fn virtual_list_directory_returns_entries() {
        let (svc, registry, sid) = make_virtual_svc();
        registry.register(sid, "/docs".into(), sample_tree(), "test_cap".into());

        let entries = svc.list_directory(sid, "/docs").await.unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"readme.md"));
        assert!(names.contains(&"guide.md"));
    }

    #[tokio::test]
    async fn virtual_list_directory_sorted_dirs_first() {
        let (svc, registry, sid) = make_virtual_svc();
        let mut tree = VirtualFileTree::new();
        tree.insert_text("/mnt/file.txt", "content");
        tree.insert_directory("/mnt/subdir");
        registry.register(sid, "/mnt".into(), Arc::new(tree), "test_cap".into());

        let entries = svc.list_directory(sid, "/mnt").await.unwrap();
        assert!(entries[0].is_directory, "directories should come first");
    }

    #[tokio::test]
    async fn virtual_wins_on_name_conflict() {
        let (svc, registry, sid) = make_virtual_svc();

        // Create a DB file at /docs/readme.md
        svc.create_directory(
            sid,
            CreateDirectoryInput {
                path: "/docs".to_string(),
            },
        )
        .await
        .unwrap();
        let db_input = CreateFileInput {
            path: "/docs/readme.md".to_string(),
            content: Some("DB content".to_string()),
            encoding: None,
            is_readonly: Some(false),
        };
        svc.create_file(sid, db_input).await.unwrap();

        // Register virtual mount with same path
        registry.register(sid, "/docs".into(), sample_tree(), "test_cap".into());

        // Virtual should win in listing
        let entries = svc.list_directory(sid, "/docs").await.unwrap();
        let readme = entries.iter().find(|e| e.name == "readme.md").unwrap();
        assert!(readme.is_readonly, "virtual entry should win (readonly)");
    }

    #[tokio::test]
    async fn virtual_update_file_rejected() {
        let (svc, registry, sid) = make_virtual_svc();
        registry.register(sid, "/docs".into(), sample_tree(), "test_cap".into());

        let err = svc
            .update_file(
                sid,
                "/docs/readme.md",
                UpdateFileInput {
                    content: Some("modified".to_string()),
                    encoding: None,
                    is_readonly: None,
                },
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("readonly"),
            "Expected readonly error, got: {err}"
        );
    }
}
