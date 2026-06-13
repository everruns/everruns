// Real-disk SessionFileSystem implementation.
//
// Rationale: built-in capabilities (`file_system`, `agent_instructions`,
// `skills`, ...) read and write through `SessionFileSystem`. For non-server
// embedders like the coding-CLI, the workspace is a real directory on disk,
// not the in-memory VFS. `RealDiskSessionFileSystemFactory` lets the platform
// resolve a `RealDiskFileStore` rooted at a workspace path.
//
// See `specs/file-store.md` for the contract, path-namespace rules, and the
// forward-compatibility plan with the mount-overlay resolver (Option B).

use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, InitialFile, SessionFile};
use everruns_core::traits::{
    SessionFileSystem, SessionFileSystemFactory, SessionFileSystemFactoryContext,
};
use everruns_core::typed_id::SessionId;
use ignore::WalkBuilder;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::RwLock;
use uuid::Uuid;

/// A `SessionFileSystem` rooted at a real host directory.
///
/// Paths are interpreted per the session filesystem namespace rules (leading `/`,
/// optional `/workspace` prefix, `..` rejected anywhere). `session_id` is
/// accepted on every method but ignored — the store is single-workspace per
/// process. See `specs/file-store.md` for the multi-tenant upgrade path.
///
/// `is_readonly` flags from `seed_initial_file` are tracked in an in-memory
/// set (the disk backend has no place to persist them), so writes and
/// deletes through this store still honor the trait contract within a
/// single process. The flag is *not* mapped onto filesystem permissions —
/// other host processes can still modify the file directly.
#[derive(Debug, Clone)]
pub struct RealDiskFileStore {
    root: PathBuf,
    readonly: Arc<RwLock<HashSet<String>>>,
}

/// Factory for real-disk session files rooted at a fixed host directory.
#[derive(Debug, Clone)]
pub struct RealDiskSessionFileSystemFactory {
    root: PathBuf,
}

impl RealDiskSessionFileSystemFactory {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

#[async_trait]
impl SessionFileSystemFactory for RealDiskSessionFileSystemFactory {
    fn name(&self) -> &'static str {
        "RealDiskSessionFileSystemFactory"
    }

    async fn create_session_file_system(
        &self,
        _context: SessionFileSystemFactoryContext,
    ) -> Result<Arc<dyn SessionFileSystem>> {
        Ok(Arc::new(RealDiskFileStore::new(self.root.clone())?))
    }
}

impl RealDiskFileStore {
    /// Create a new real-disk store rooted at `root`.
    ///
    /// The root is canonicalized once at construction time. Any operation
    /// whose canonical-form path would escape the root is rejected.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        if !root.exists() {
            return Err(AgentLoopError::config(format!(
                "RealDiskFileStore root does not exist: {}",
                root.display()
            )));
        }
        let canonical = std::fs::canonicalize(&root).map_err(|e| {
            AgentLoopError::config(format!(
                "failed to canonicalize RealDiskFileStore root {}: {e}",
                root.display()
            ))
        })?;
        if !canonical.is_dir() {
            return Err(AgentLoopError::config(format!(
                "RealDiskFileStore root is not a directory: {}",
                canonical.display()
            )));
        }
        Ok(Self {
            root: canonical,
            readonly: Arc::new(RwLock::new(HashSet::new())),
        })
    }

    async fn is_readonly(&self, canonical_path: &str) -> bool {
        self.readonly.read().await.contains(canonical_path)
    }

    async fn mark_readonly(&self, canonical_path: String, readonly: bool) {
        let mut guard = self.readonly.write().await;
        if readonly {
            guard.insert(canonical_path);
        } else {
            guard.remove(&canonical_path);
        }
    }

    /// Borrow the canonicalized workspace root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn normalize_path(&self, path: &str) -> String {
        let input = Path::new(path);
        if input.is_absolute()
            && let Ok(relative) = input.strip_prefix(&self.root)
            && let Ok(path) = capability_path_from_relative(relative)
        {
            return path;
        }
        normalize_path(path)
    }

    /// Resolve a capability-facing path to an absolute host path.
    ///
    /// Returns an error if the input contains a `..` segment or if joining
    /// produces a path outside the root. Symlink containment is checked by
    /// `reject_symlink_path` at each filesystem access so missing write
    /// targets can still be created safely.
    fn resolve(&self, path: &str) -> Result<PathBuf> {
        let normalized = self.normalize_path(path);
        if normalized == "/" {
            return Ok(self.root.clone());
        }
        // Drop the leading `/` so `Path::join` treats it as relative.
        let relative = normalized.trim_start_matches('/');
        let candidate = Path::new(relative);

        for component in candidate.components() {
            match component {
                Component::Normal(_) => {}
                Component::CurDir => {}
                Component::ParentDir => {
                    return Err(AgentLoopError::tool(format!(
                        "path traversal rejected: {path}"
                    )));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(AgentLoopError::tool(format!(
                        "absolute path component rejected: {path}"
                    )));
                }
            }
        }

        let absolute = self.root.join(candidate);
        if !absolute.starts_with(&self.root) {
            return Err(AgentLoopError::tool(format!(
                "path escapes workspace root: {path}"
            )));
        }
        Ok(absolute)
    }

    /// Reject symlinks anywhere in the resolved path before performing real
    /// disk I/O. File operations are LLM-controlled in embedded runtimes, so
    /// following workspace symlinks would bypass the workspace boundary and
    /// any lexical write policies layered above this store. Missing components
    /// are allowed so callers can create new files/directories after all
    /// existing ancestors have been checked.
    async fn reject_symlink_path(&self, absolute: &Path) -> Result<()> {
        let relative = absolute.strip_prefix(&self.root).map_err(|_| {
            AgentLoopError::tool(format!(
                "path is outside workspace root: {}",
                absolute.display()
            ))
        })?;

        let mut current = self.root.clone();
        for component in relative.components() {
            match component {
                Component::Normal(segment) => current.push(segment),
                _ => {
                    return Err(AgentLoopError::tool(format!(
                        "unexpected path component in {}",
                        absolute.display()
                    )));
                }
            }

            match tokio::fs::symlink_metadata(&current).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(AgentLoopError::tool(format!(
                        "symlink paths are not allowed in real-disk workspace access: {}",
                        current.display()
                    )));
                }
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => {
                    return Err(AgentLoopError::tool(format!(
                        "lstat failed for {}: {e}",
                        current.display()
                    )));
                }
            }
        }
        Ok(())
    }

    fn relative_capability_path(&self, absolute: &Path) -> Result<String> {
        let rel = absolute.strip_prefix(&self.root).map_err(|_| {
            AgentLoopError::tool(format!(
                "path is outside workspace root: {}",
                absolute.display()
            ))
        })?;
        capability_path_from_relative(rel)
    }
}

fn capability_path_from_relative(relative: &Path) -> Result<String> {
    if relative.as_os_str().is_empty() {
        return Ok("/".to_string());
    }
    let mut segments = Vec::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(s) => {
                let segment = s.to_str().ok_or_else(|| {
                    AgentLoopError::tool(format!(
                        "non-UTF-8 path component: {}",
                        relative.display()
                    ))
                })?;
                segments.push(segment);
            }
            _ => {
                return Err(AgentLoopError::tool(format!(
                    "unexpected path component in {}",
                    relative.display()
                )));
            }
        }
    }
    if segments.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", segments.join("/")))
    }
}

fn display_join(root: &Path, path: &str) -> String {
    if path == "/" {
        return root.display().to_string();
    }
    root.join(path.trim_start_matches('/'))
        .display()
        .to_string()
}

#[async_trait]
impl SessionFileSystem for RealDiskFileStore {
    fn display_root(&self) -> String {
        self.root.display().to_string()
    }

    fn display_path(&self, path: &str) -> String {
        display_join(&self.root, &self.normalize_path(path))
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        // Clear any prior readonly mark so seeding always wins over a
        // previous starter-file declaration with the same path.
        let absolute = self.resolve(&file.path)?;
        self.reject_symlink_path(&absolute).await?;
        let canonical = self.relative_capability_path(&absolute)?;
        self.mark_readonly(canonical.clone(), false).await;

        self.write_file(session_id, &file.path, &file.content, &file.encoding)
            .await?;
        if file.is_readonly {
            self.mark_readonly(canonical, true).await;
        }
        Ok(())
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let absolute = self.resolve(path)?;
        self.reject_symlink_path(&absolute).await?;
        let metadata = match tokio::fs::metadata(&absolute).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(AgentLoopError::tool(format!(
                    "stat failed for {}: {e}",
                    absolute.display()
                )));
            }
        };

        let canonical_path = self.relative_capability_path(&absolute)?;
        let name = FileInfo::name_from_path(&canonical_path);
        let id = path_id(&canonical_path);

        let (created_at, updated_at) = file_times(&metadata);
        let is_readonly = self.is_readonly(&canonical_path).await;

        if metadata.is_dir() {
            return Ok(Some(SessionFile {
                id,
                session_id: session_id.uuid(),
                path: canonical_path,
                name,
                content: None,
                encoding: "text".to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at,
                updated_at,
            }));
        }

        let bytes = tokio::fs::read(&absolute).await.map_err(|e| {
            AgentLoopError::tool(format!("read failed for {}: {e}", absolute.display()))
        })?;
        let size_bytes = saturating_i64(bytes.len() as u64);
        let (content, encoding) = SessionFile::encode_content(&bytes);

        Ok(Some(SessionFile {
            id,
            session_id: session_id.uuid(),
            path: canonical_path,
            name,
            content: Some(content),
            encoding,
            is_directory: false,
            is_readonly,
            size_bytes,
            created_at,
            updated_at,
        }))
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let absolute = self.resolve(path)?;
        self.reject_symlink_path(&absolute).await?;
        let canonical_path = self.relative_capability_path(&absolute)?;
        if self.is_readonly(&canonical_path).await {
            return Err(AgentLoopError::tool(format!(
                "file is read-only: {canonical_path}"
            )));
        }
        if let Some(parent) = absolute.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AgentLoopError::tool(format!("failed to create parent {}: {e}", parent.display()))
            })?;
        }

        if let Ok(meta) = tokio::fs::metadata(&absolute).await
            && meta.is_dir()
        {
            return Err(AgentLoopError::tool(format!(
                "write target is a directory: {}",
                absolute.display()
            )));
        }

        let bytes = SessionFile::decode_content(content, encoding)
            .map_err(|e| AgentLoopError::tool(format!("base64 decode failed for {path}: {e}")))?;
        tokio::fs::write(&absolute, &bytes).await.map_err(|e| {
            AgentLoopError::tool(format!("write failed for {}: {e}", absolute.display()))
        })?;

        let metadata = tokio::fs::metadata(&absolute).await.map_err(|e| {
            AgentLoopError::tool(format!(
                "post-write stat failed for {}: {e}",
                absolute.display()
            ))
        })?;
        let (created_at, updated_at) = file_times(&metadata);
        let name = FileInfo::name_from_path(&canonical_path);
        let id = path_id(&canonical_path);

        Ok(SessionFile {
            id,
            session_id: session_id.uuid(),
            path: canonical_path,
            name,
            content: Some(content.to_string()),
            encoding: encoding.to_string(),
            is_directory: false,
            is_readonly: false,
            size_bytes: saturating_i64(bytes.len() as u64),
            created_at,
            updated_at,
        })
    }

    async fn delete_file(
        &self,
        _session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        let absolute = self.resolve(path)?;
        self.reject_symlink_path(&absolute).await?;
        if absolute == self.root {
            return Err(AgentLoopError::tool(
                "cannot delete workspace root".to_string(),
            ));
        }
        let canonical_path = self.relative_capability_path(&absolute)?;
        if self.is_readonly(&canonical_path).await {
            return Err(AgentLoopError::tool(format!(
                "file is read-only: {canonical_path}"
            )));
        }
        let metadata = match tokio::fs::metadata(&absolute).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => {
                return Err(AgentLoopError::tool(format!(
                    "stat failed for {}: {e}",
                    absolute.display()
                )));
            }
        };

        if metadata.is_dir() {
            if recursive {
                tokio::fs::remove_dir_all(&absolute).await.map_err(|e| {
                    AgentLoopError::tool(format!(
                        "recursive delete failed for {}: {e}",
                        absolute.display()
                    ))
                })?;
            } else {
                let mut read_dir = tokio::fs::read_dir(&absolute).await.map_err(|e| {
                    AgentLoopError::tool(format!("read_dir failed for {}: {e}", absolute.display()))
                })?;
                if read_dir
                    .next_entry()
                    .await
                    .map_err(|e| {
                        AgentLoopError::tool(format!(
                            "read_dir entry failed for {}: {e}",
                            absolute.display()
                        ))
                    })?
                    .is_some()
                {
                    return Ok(false);
                }
                tokio::fs::remove_dir(&absolute).await.map_err(|e| {
                    AgentLoopError::tool(format!("rmdir failed for {}: {e}", absolute.display()))
                })?;
            }
            return Ok(true);
        }

        tokio::fs::remove_file(&absolute).await.map_err(|e| {
            AgentLoopError::tool(format!("delete failed for {}: {e}", absolute.display()))
        })?;
        Ok(true)
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let absolute = self.resolve(path)?;
        self.reject_symlink_path(&absolute).await?;
        let metadata = match tokio::fs::metadata(&absolute).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => {
                return Err(AgentLoopError::tool(format!(
                    "stat failed for {}: {e}",
                    absolute.display()
                )));
            }
        };
        if !metadata.is_dir() {
            return Ok(vec![]);
        }

        let mut read_dir = tokio::fs::read_dir(&absolute).await.map_err(|e| {
            AgentLoopError::tool(format!("read_dir failed for {}: {e}", absolute.display()))
        })?;
        let mut entries = Vec::new();
        while let Some(entry) = read_dir.next_entry().await.map_err(|e| {
            AgentLoopError::tool(format!(
                "read_dir entry failed for {}: {e}",
                absolute.display()
            ))
        })? {
            let entry_path = entry.path();
            let canonical = self.relative_capability_path(&entry_path)?;
            let entry_meta = match tokio::fs::symlink_metadata(&entry_path).await {
                Ok(m) if m.file_type().is_symlink() => continue,
                Ok(m) => m,
                Err(_) => continue,
            };
            let (created_at, updated_at) = file_times(&entry_meta);
            let is_dir = entry_meta.is_dir();
            entries.push(FileInfo {
                id: path_id(&canonical),
                session_id: session_id.uuid(),
                name: FileInfo::name_from_path(&canonical),
                path: canonical,
                is_directory: is_dir,
                is_readonly: false,
                size_bytes: if is_dir {
                    0
                } else {
                    saturating_i64(entry_meta.len())
                },
                created_at,
                updated_at,
            });
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn stat_file(&self, _session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        let absolute = self.resolve(path)?;
        self.reject_symlink_path(&absolute).await?;
        let metadata = match tokio::fs::metadata(&absolute).await {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(AgentLoopError::tool(format!(
                    "stat failed for {}: {e}",
                    absolute.display()
                )));
            }
        };
        let canonical = self.relative_capability_path(&absolute)?;
        let name = FileInfo::name_from_path(&canonical);
        let (created_at, updated_at) = file_times(&metadata);
        let is_readonly = self.is_readonly(&canonical).await;
        Ok(Some(FileStat {
            path: canonical,
            name,
            is_directory: metadata.is_dir(),
            is_readonly,
            size_bytes: if metadata.is_dir() {
                0
            } else {
                saturating_i64(metadata.len())
            },
            created_at,
            updated_at,
        }))
    }

    async fn grep_files(
        &self,
        _session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let root = self.root.clone();
        let pattern = pattern.to_string();
        let path_pattern = path_pattern.map(|path| {
            self.normalize_path(path)
                .trim_start_matches('/')
                .to_string()
        });

        // `ignore::WalkBuilder` is sync; reading file content per match is
        // sync too. Push the whole walk onto `spawn_blocking` so we don't
        // block the executor on large trees.
        tokio::task::spawn_blocking(move || -> Result<Vec<GrepMatch>> {
            let mut out = Vec::new();
            let walker = WalkBuilder::new(&root)
                .hidden(false)
                .git_ignore(true)
                .git_global(false)
                .git_exclude(true)
                .build();
            for entry in walker {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let path = entry.path();
                if !entry.file_type().map(|ft| ft.is_file()).unwrap_or(false) {
                    continue;
                }
                let relative = match path.strip_prefix(&root) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                // Skip non-UTF-8 paths rather than corrupting them with
                // `to_string_lossy()`: `GrepMatch.path` must round-trip back
                // through `resolve` for subsequent `read_file` calls.
                let mut rel_str = String::new();
                let mut ok = true;
                let mut first = true;
                for component in relative.components() {
                    if let Component::Normal(seg) = component {
                        if !first {
                            rel_str.push('/');
                        }
                        first = false;
                        match seg.to_str() {
                            Some(s) => rel_str.push_str(s),
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    } else {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    continue;
                }
                if let Some(filter) = &path_pattern
                    && !rel_str.contains(filter.as_str())
                {
                    continue;
                }
                let bytes = match std::fs::read(path) {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                if !SessionFile::is_text_content(&bytes) {
                    continue;
                }
                let text = match std::str::from_utf8(&bytes) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                let canonical_path = format!("/{rel_str}");
                for (idx, line) in text.lines().enumerate() {
                    if line.contains(&pattern) {
                        out.push(GrepMatch {
                            path: canonical_path.clone(),
                            line_number: idx + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
            Ok(out)
        })
        .await
        .map_err(|e| AgentLoopError::tool(format!("grep walk join failed: {e}")))?
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        let absolute = self.resolve(path)?;
        self.reject_symlink_path(&absolute).await?;
        tokio::fs::create_dir_all(&absolute).await.map_err(|e| {
            AgentLoopError::tool(format!(
                "create_dir_all failed for {}: {e}",
                absolute.display()
            ))
        })?;
        let metadata = tokio::fs::metadata(&absolute).await.map_err(|e| {
            AgentLoopError::tool(format!("stat failed for {}: {e}", absolute.display()))
        })?;
        let canonical = self.relative_capability_path(&absolute)?;
        let (created_at, updated_at) = file_times(&metadata);
        Ok(FileInfo {
            id: path_id(&canonical),
            session_id: session_id.uuid(),
            name: FileInfo::name_from_path(&canonical),
            path: canonical,
            is_directory: true,
            is_readonly: false,
            size_bytes: 0,
            created_at,
            updated_at,
        })
    }
}

fn normalize_path(path: &str) -> String {
    if path.is_empty() || path == "/" {
        return "/".to_string();
    }
    let mut normalized = if let Some(stripped) = path.strip_prefix("/workspace/") {
        format!("/{}", stripped)
    } else if path == "/workspace" {
        "/".to_string()
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

fn path_id(canonical_path: &str) -> Uuid {
    // Stable, deterministic IDs derived from the canonical path. The disk
    // backend has no other persistent identifier; consumers that rely on a
    // SessionFile.id should still see the same UUID on subsequent reads.
    Uuid::new_v5(&Uuid::NAMESPACE_OID, canonical_path.as_bytes())
}

fn file_times(metadata: &std::fs::Metadata) -> (DateTime<Utc>, DateTime<Utc>) {
    let modified = metadata
        .modified()
        .ok()
        .and_then(system_time_to_utc)
        .unwrap_or_else(Utc::now);
    let created = metadata
        .created()
        .ok()
        .and_then(system_time_to_utc)
        .unwrap_or(modified);
    (created, modified)
}

fn system_time_to_utc(time: SystemTime) -> Option<DateTime<Utc>> {
    let duration = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Utc.timestamp_opt(duration.as_secs() as i64, duration.subsec_nanos())
        .single()
}

/// Saturating `u64 -> i64` cast. The `SessionFile` trait fixes size as
/// `i64`; files larger than 9 EiB are not realistically reachable through
/// this code path, but the explicit cap makes the wrap intent obvious.
fn saturating_i64(value: u64) -> i64 {
    if value > i64::MAX as u64 {
        i64::MAX
    } else {
        value as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store() -> (RealDiskFileStore, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let store = RealDiskFileStore::new(dir.path()).expect("store");
        (store, dir)
    }

    fn sid() -> SessionId {
        SessionId::new()
    }

    #[tokio::test]
    async fn round_trip_text_file() {
        let (store, _dir) = make_store();
        let session = sid();
        let written = store
            .write_file(session, "/notes.md", "# hello", "text")
            .await
            .expect("write");
        assert_eq!(written.path, "/notes.md");
        assert_eq!(written.encoding, "text");

        let read = store
            .read_file(session, "/notes.md")
            .await
            .expect("read")
            .expect("present");
        assert_eq!(read.content.as_deref(), Some("# hello"));
        assert_eq!(read.encoding, "text");
        assert_eq!(read.size_bytes, 7);
        assert!(!read.is_directory);
    }

    #[tokio::test]
    async fn round_trip_binary_file() {
        let (store, _dir) = make_store();
        let session = sid();
        let bytes = [0x89u8, b'P', b'N', b'G', 0, 1, 2, 3];
        let (encoded, encoding) = SessionFile::encode_content(&bytes);
        assert_eq!(encoding, "base64");

        store
            .write_file(session, "/img.bin", &encoded, &encoding)
            .await
            .expect("write");

        let read = store
            .read_file(session, "/img.bin")
            .await
            .expect("read")
            .expect("present");
        assert_eq!(read.encoding, "base64");
        let decoded = SessionFile::decode_content(read.content.as_deref().unwrap(), &read.encoding)
            .expect("decode");
        assert_eq!(decoded, bytes);
    }

    #[tokio::test]
    async fn workspace_prefix_normalized() {
        let (store, _dir) = make_store();
        let session = sid();
        store
            .write_file(session, "/workspace/sub/dir/file.txt", "hi", "text")
            .await
            .expect("write");

        let via_canonical = store
            .read_file(session, "/sub/dir/file.txt")
            .await
            .expect("read")
            .expect("present");
        let via_workspace = store
            .read_file(session, "/workspace/sub/dir/file.txt")
            .await
            .expect("read")
            .expect("present");
        assert_eq!(via_canonical.content, via_workspace.content);
        assert_eq!(via_canonical.path, "/sub/dir/file.txt");
    }

    #[tokio::test]
    async fn real_disk_display_paths_use_host_root() {
        let (store, dir) = make_store();
        let root = std::fs::canonicalize(dir.path()).expect("canonical tempdir");

        assert_eq!(store.display_root(), root.display().to_string());
        assert_eq!(
            store.display_path("/sub/dir/file.txt"),
            root.join("sub/dir/file.txt").display().to_string()
        );
    }

    #[tokio::test]
    async fn host_absolute_paths_under_root_are_workspace_aliases() {
        let (store, _dir) = make_store();
        let session = sid();
        let host_path = store.display_path("/sub/dir/file.txt");

        store
            .write_file(session, &host_path, "hi", "text")
            .await
            .expect("write via host path");

        let via_workspace = store
            .read_file(session, "/workspace/sub/dir/file.txt")
            .await
            .expect("read")
            .expect("present");
        assert_eq!(via_workspace.content.as_deref(), Some("hi"));
        assert_eq!(via_workspace.path, "/sub/dir/file.txt");
    }

    #[tokio::test]
    async fn host_absolute_aliases_allow_current_dir_segments() {
        let (store, _dir) = make_store();
        let session = sid();
        let host_path = Path::new(&store.display_root())
            .join("./sub/dir/file.txt")
            .display()
            .to_string();

        store
            .write_file(session, &host_path, "hi", "text")
            .await
            .expect("write via host path");

        let via_workspace = store
            .read_file(session, "/workspace/sub/dir/file.txt")
            .await
            .expect("read")
            .expect("present");
        assert_eq!(via_workspace.content.as_deref(), Some("hi"));
        assert_eq!(via_workspace.path, "/sub/dir/file.txt");
    }

    #[tokio::test]
    async fn grep_path_pattern_accepts_host_absolute_path_alias() {
        let (store, _dir) = make_store();
        let session = sid();
        store
            .write_file(session, "/src/lib.rs", "needle", "text")
            .await
            .expect("write src");
        store
            .write_file(session, "/docs/readme.md", "needle", "text")
            .await
            .expect("write docs");
        let host_filter = store.display_path("/src");

        let matches = store
            .grep_files(session, "needle", Some(&host_filter))
            .await
            .expect("grep");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path, "/src/lib.rs");
    }

    #[tokio::test]
    async fn path_traversal_rejected() {
        let (store, _dir) = make_store();
        let session = sid();
        let err = store
            .read_file(session, "/../outside.txt")
            .await
            .expect_err("must reject traversal");
        let msg = format!("{err}");
        assert!(msg.contains("traversal"), "got: {msg}");

        let err = store
            .write_file(session, "/foo/../../etc/passwd", "x", "text")
            .await
            .expect_err("must reject traversal");
        let msg = format!("{err}");
        assert!(msg.contains("traversal"), "got: {msg}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn read_file_rejects_symlink_to_outside_workspace() {
        let (store, dir) = make_store();
        let outside = TempDir::new().expect("outside tempdir");
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        std::fs::create_dir(dir.path().join("docs")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("docs/secret")).unwrap();

        let err = store
            .read_file(sid(), "/docs/secret/secret.txt")
            .await
            .expect_err("symlink read must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("symlink"), "got: {msg}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn list_directory_rejects_symlink_to_outside_workspace() {
        let (store, dir) = make_store();
        let outside = TempDir::new().expect("outside tempdir");
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("secret_dir")).unwrap();

        let err = store
            .list_directory(sid(), "/secret_dir")
            .await
            .expect_err("symlink list must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("symlink"), "got: {msg}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_file_rejects_symlink_parent() {
        let (store, dir) = make_store();
        let outside = TempDir::new().expect("outside tempdir");
        std::os::unix::fs::symlink(outside.path(), dir.path().join("outlink")).unwrap();

        let err = store
            .write_file(sid(), "/outlink/owned.txt", "owned", "text")
            .await
            .expect_err("symlink write must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("symlink"), "got: {msg}");
        assert!(!outside.path().join("owned.txt").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn list_directory_skips_symlink_children() {
        let (store, dir) = make_store();
        let outside = TempDir::new().expect("outside tempdir");
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("link.txt"),
        )
        .unwrap();
        store
            .write_file(sid(), "/safe.txt", "safe", "text")
            .await
            .unwrap();

        let entries = store.list_directory(sid(), "/").await.unwrap();
        let paths: Vec<&str> = entries.iter().map(|entry| entry.path.as_str()).collect();
        assert!(paths.contains(&"/safe.txt"));
        assert!(!paths.contains(&"/link.txt"));
    }

    #[tokio::test]
    async fn list_directory_returns_children() {
        let (store, _dir) = make_store();
        let session = sid();
        store
            .write_file(session, "/a.txt", "1", "text")
            .await
            .unwrap();
        store
            .write_file(session, "/sub/b.txt", "2", "text")
            .await
            .unwrap();
        store
            .write_file(session, "/sub/c.txt", "3", "text")
            .await
            .unwrap();

        let root = store.list_directory(session, "/").await.unwrap();
        let paths: Vec<&str> = root.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"/a.txt"));
        assert!(paths.contains(&"/sub"));

        let sub = store.list_directory(session, "/sub").await.unwrap();
        let sub_paths: Vec<&str> = sub.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(sub_paths, vec!["/sub/b.txt", "/sub/c.txt"]);
    }

    #[tokio::test]
    async fn grep_finds_matches_and_respects_ignore_files() {
        let (store, dir) = make_store();
        let session = sid();
        // The `ignore` crate honors `.ignore` files unconditionally; it
        // honors `.gitignore` only inside a real git repo, which we don't
        // need for this test. Both files are walked by `WalkBuilder`.
        std::fs::write(dir.path().join(".ignore"), "ignored.txt\n").unwrap();
        store
            .write_file(
                session,
                "/src.rs",
                "fn needle() {}\nfn other() {}\n",
                "text",
            )
            .await
            .unwrap();
        store
            .write_file(session, "/ignored.txt", "needle\n", "text")
            .await
            .unwrap();

        let hits = store.grep_files(session, "needle", None).await.unwrap();
        let hit_paths: Vec<&str> = hits.iter().map(|m| m.path.as_str()).collect();
        assert!(hit_paths.contains(&"/src.rs"));
        assert!(!hit_paths.contains(&"/ignored.txt"));

        let filtered = store
            .grep_files(session, "needle", Some(".rs"))
            .await
            .unwrap();
        assert!(filtered.iter().all(|m| m.path.ends_with(".rs")));
    }

    #[tokio::test]
    async fn cas_rejects_stale_writes() {
        let (store, _dir) = make_store();
        let session = sid();
        store
            .write_file(session, "/foo.txt", "v1", "text")
            .await
            .unwrap();

        // Stale CAS — expects v0 content.
        let stale = store
            .write_file_if_content_matches(session, "/foo.txt", "v0", "text", "v2", "text")
            .await
            .unwrap();
        assert!(stale.is_none(), "stale CAS should not update");

        let read = store.read_file(session, "/foo.txt").await.unwrap().unwrap();
        assert_eq!(read.content.as_deref(), Some("v1"));

        // Matching CAS — updates.
        let updated = store
            .write_file_if_content_matches(session, "/foo.txt", "v1", "text", "v2", "text")
            .await
            .unwrap();
        assert!(updated.is_some(), "matching CAS should update");
        let read = store.read_file(session, "/foo.txt").await.unwrap().unwrap();
        assert_eq!(read.content.as_deref(), Some("v2"));
    }

    #[tokio::test]
    async fn delete_non_recursive_fails_on_nonempty_dir() {
        let (store, _dir) = make_store();
        let session = sid();
        store
            .write_file(session, "/d/x.txt", "x", "text")
            .await
            .unwrap();

        let removed = store.delete_file(session, "/d", false).await.unwrap();
        assert!(!removed, "non-recursive delete must refuse non-empty dir");

        let removed = store.delete_file(session, "/d", true).await.unwrap();
        assert!(removed);
        let after = store.read_file(session, "/d/x.txt").await.unwrap();
        assert!(after.is_none());
    }

    #[tokio::test]
    async fn seed_initial_file_persists() {
        let (store, _dir) = make_store();
        let session = sid();
        store
            .seed_initial_file(
                session,
                &InitialFile {
                    path: "/workspace/AGENTS.md".to_string(),
                    content: "# Project rules".to_string(),
                    encoding: "text".to_string(),
                    is_readonly: false,
                },
            )
            .await
            .unwrap();

        let read = store
            .read_file(session, "/AGENTS.md")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.content.as_deref(), Some("# Project rules"));
    }

    #[tokio::test]
    async fn root_directory_resolves() {
        let (store, _dir) = make_store();
        let session = sid();
        let stat = store.stat_file(session, "/").await.unwrap().unwrap();
        assert!(stat.is_directory);
        assert_eq!(stat.path, "/");
    }

    #[tokio::test]
    async fn rejects_missing_root() {
        let missing = std::env::temp_dir().join("everruns-nonexistent-xyz-12345");
        let _ = std::fs::remove_dir_all(&missing);
        let err = RealDiskFileStore::new(&missing).expect_err("must reject missing root");
        let msg = format!("{err}");
        assert!(msg.contains("does not exist"), "got: {msg}");
    }

    #[tokio::test]
    async fn delete_root_returns_explicit_error() {
        let (store, _dir) = make_store();
        let session = sid();
        let err = store
            .delete_file(session, "/", true)
            .await
            .expect_err("root delete must be an explicit error, not Ok(false)");
        assert!(format!("{err}").contains("workspace root"));
    }

    #[tokio::test]
    async fn seeded_readonly_file_rejects_writes() {
        let (store, _dir) = make_store();
        let session = sid();
        store
            .seed_initial_file(
                session,
                &InitialFile {
                    path: "/locked.txt".to_string(),
                    content: "starter".to_string(),
                    encoding: "text".to_string(),
                    is_readonly: true,
                },
            )
            .await
            .unwrap();

        let read = store
            .read_file(session, "/locked.txt")
            .await
            .unwrap()
            .unwrap();
        assert!(read.is_readonly);

        let err = store
            .write_file(session, "/locked.txt", "changed", "text")
            .await
            .expect_err("readonly write must fail");
        assert!(format!("{err}").contains("read-only"));

        let err = store
            .delete_file(session, "/locked.txt", false)
            .await
            .expect_err("readonly delete must fail");
        assert!(format!("{err}").contains("read-only"));
    }

    #[tokio::test]
    async fn reseeding_clears_readonly() {
        let (store, _dir) = make_store();
        let session = sid();
        store
            .seed_initial_file(
                session,
                &InitialFile {
                    path: "/foo.txt".to_string(),
                    content: "v1".to_string(),
                    encoding: "text".to_string(),
                    is_readonly: true,
                },
            )
            .await
            .unwrap();
        // Re-seed without readonly: subsequent writes must succeed.
        store
            .seed_initial_file(
                session,
                &InitialFile {
                    path: "/foo.txt".to_string(),
                    content: "v2".to_string(),
                    encoding: "text".to_string(),
                    is_readonly: false,
                },
            )
            .await
            .unwrap();
        store
            .write_file(session, "/foo.txt", "v3", "text")
            .await
            .unwrap();
    }
}
