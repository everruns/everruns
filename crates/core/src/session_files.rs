//! Neutral session filesystem contract and execution-scoping adapter.

use crate::error::Result;
use crate::session_file::{
    FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, InitialFile, SessionFile,
};
use crate::typed_id::{SessionId, WorkspaceId};
use async_trait::async_trait;
use std::sync::Arc;

/// Trait for session filesystem operations
///
/// This trait abstracts the session filesystem contract for tools and hosts.
/// Implementations can:
/// - Store files in a database (production)
/// - Use an in-memory filesystem for testing
/// - Project files onto real disk or object storage
#[async_trait]
pub trait SessionFileSystem: Send + Sync {
    /// Human-facing root path for this filesystem.
    ///
    /// `/workspace` is the stable agent namespace and the default. Direct
    /// host-backed stores may override this for host-side integrations, while
    /// [`MountFs`](crate::mount_fs::MountFs) restores the agent-facing root.
    fn display_root(&self) -> String {
        crate::session_path::WORKSPACE_PREFIX.to_string()
    }

    /// Convert a canonical session path into a human-facing path.
    ///
    /// The default renders the `/workspace` alias. Direct host-backed stores may
    /// override it, while [`MountFs`](crate::mount_fs::MountFs) presents primary
    /// workspace paths through the stable agent-facing namespace.
    fn display_path(&self, path: &str) -> String {
        crate::session_path::to_display_path(path)
    }

    /// Resolve an input path (any accepted spelling, relative or absolute) to an
    /// absolute path within this filesystem's namespace. Relative inputs resolve
    /// against the filesystem's current directory.
    ///
    /// This is how a shell seeds its working directory: [`MountFs`] returns a
    /// path in its stable agent-facing namespace, so the shell and file tools
    /// share the same identity. The default is the flat VFS session form.
    /// Security decorators may authorize the returned path, so providers that
    /// accept additional aliases must use the same contained mapping here and
    /// in their I/O methods. An alias must never resolve to one workspace path
    /// here and a different storage object during the subsequent operation.
    ///
    /// [`MountFs`]: crate::mount_fs::MountFs
    fn resolve_path(&self, input: &str) -> String {
        crate::session_path::to_session_path(input)
    }

    /// Whether this store is already a mount-based resolver
    /// ([`MountFs`](crate::mount_fs::MountFs)).
    ///
    /// Used to avoid re-wrapping nested mount tables when building tool context.
    fn is_mount_resolver(&self) -> bool;

    /// Read a file by path
    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>>;

    /// Write/create a file
    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile>;

    /// Write a file only if its current content snapshot still matches.
    ///
    /// Implementations backed by transactional storage should override this
    /// with an atomic compare-and-set update.
    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        let Some(existing) = self.read_file(session_id, path).await? else {
            return Ok(None);
        };

        if existing.is_directory {
            return Ok(None);
        }

        let current_content = existing.content.unwrap_or_default();
        if current_content != expected_content || existing.encoding != expected_encoding {
            return Ok(None);
        }

        self.write_file(session_id, path, content, encoding)
            .await
            .map(Some)
    }

    /// Delete a file or directory
    async fn delete_file(&self, session_id: SessionId, path: &str, recursive: bool)
    -> Result<bool>;

    /// List files in a directory
    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>>;

    /// Get file metadata
    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>>;

    /// Search file contents with Rust regex syntax, optionally filtering canonical paths by glob.
    ///
    /// Implementations compile the content pattern once before scanning and
    /// return an error for invalid regex. Basename-only globs match at any
    /// depth. Non-glob path filters retain legacy substring matching; see
    /// `knowledge/runtime-resources/file-store.md`.
    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>>;

    /// Search with match pagination and bounded before/after context.
    ///
    /// Backends should override this to collect context during their content
    /// scan. The default preserves compatibility for third-party stores that
    /// only implement the original zero-context method.
    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        if options.before_context != 0 || options.after_context != 0 {
            return Err(crate::error::AgentLoopError::tool(
                "this file store does not support grep context",
            ));
        }
        let all = self
            .grep_files(session_id, pattern, options.path_pattern.as_deref())
            .await?;
        Ok(crate::session_file::bound_grep_matches(all, options))
    }

    /// Create a directory
    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo>;

    /// Seed a starter file into a session workspace.
    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        if file.is_readonly {
            return Err(crate::error::AgentLoopError::store(
                "read-only initial files require a SessionFileSystem-specific seed implementation",
            ));
        }
        self.write_file(session_id, &file.path, &file.content, &file.encoding)
            .await?;
        Ok(())
    }
}

/// A [`SessionFileSystem`] decorator that pins every operation to a fixed
/// workspace key, ignoring the per-call `session_id`.
///
/// Used to re-key file I/O for a session attached to a shared workspace (where
/// `workspace.id != session.id`): wrap the session's file store once with the
/// session's `workspace_id`, and all downstream capability/tool access then
/// addresses the attached workspace rather than the session's own keyspace. For
/// the default 1:1 session the key equals the session id, so the wrapper is a
/// transparent pass-through. See `knowledge/runtime-resources/workspace.md`.
pub struct WorkspaceScopedFileSystem {
    inner: Arc<dyn SessionFileSystem>,
    key: SessionId,
}

impl WorkspaceScopedFileSystem {
    /// Wrap `inner`, pinning all operations to `workspace_id`'s key.
    pub fn wrap(
        inner: Arc<dyn SessionFileSystem>,
        workspace_id: WorkspaceId,
    ) -> Arc<dyn SessionFileSystem> {
        Arc::new(Self {
            inner,
            key: SessionId::from_uuid(workspace_id.uuid()),
        })
    }
}

#[async_trait]
impl SessionFileSystem for WorkspaceScopedFileSystem {
    async fn read_file(&self, _session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        self.inner.read_file(self.key, path).await
    }
    async fn write_file(
        &self,
        _session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        self.inner
            .write_file(self.key, path, content, encoding)
            .await
    }
    async fn write_file_if_content_matches(
        &self,
        _session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        self.inner
            .write_file_if_content_matches(
                self.key,
                path,
                expected_content,
                expected_encoding,
                content,
                encoding,
            )
            .await
    }
    async fn delete_file(
        &self,
        _session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        self.inner.delete_file(self.key, path, recursive).await
    }
    async fn list_directory(&self, _session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        self.inner.list_directory(self.key, path).await
    }
    async fn stat_file(&self, _session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        self.inner.stat_file(self.key, path).await
    }
    async fn grep_files(
        &self,
        _session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        self.inner.grep_files(self.key, pattern, path_pattern).await
    }
    async fn grep_files_with_options(
        &self,
        _session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        self.inner
            .grep_files_with_options(self.key, pattern, options)
            .await
    }
    async fn create_directory(&self, _session_id: SessionId, path: &str) -> Result<FileInfo> {
        self.inner.create_directory(self.key, path).await
    }
    async fn seed_initial_file(&self, _session_id: SessionId, file: &InitialFile) -> Result<()> {
        self.inner.seed_initial_file(self.key, file).await
    }

    fn display_root(&self) -> String {
        self.inner.display_root()
    }

    fn display_path(&self, path: &str) -> String {
        self.inner.display_path(path)
    }

    fn resolve_path(&self, input: &str) -> String {
        self.inner.resolve_path(input)
    }

    fn is_mount_resolver(&self) -> bool {
        self.inner.is_mount_resolver()
    }
}

#[async_trait]
impl<T: SessionFileSystem + ?Sized> SessionFileSystem for std::sync::Arc<T> {
    fn display_root(&self) -> String {
        (**self).display_root()
    }

    fn display_path(&self, path: &str) -> String {
        (**self).display_path(path)
    }

    fn resolve_path(&self, input: &str) -> String {
        (**self).resolve_path(input)
    }

    fn is_mount_resolver(&self) -> bool {
        (**self).is_mount_resolver()
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        (**self).read_file(session_id, path).await
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        (**self)
            .write_file(session_id, path, content, encoding)
            .await
    }

    async fn write_file_if_content_matches(
        &self,
        session_id: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        (**self)
            .write_file_if_content_matches(
                session_id,
                path,
                expected_content,
                expected_encoding,
                content,
                encoding,
            )
            .await
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        (**self).delete_file(session_id, path, recursive).await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        (**self).list_directory(session_id, path).await
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        (**self).stat_file(session_id, path).await
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        (**self).grep_files(session_id, pattern, path_pattern).await
    }

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        (**self)
            .grep_files_with_options(session_id, pattern, options)
            .await
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        (**self).create_directory(session_id, path).await
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        (**self).seed_initial_file(session_id, file).await
    }
}
