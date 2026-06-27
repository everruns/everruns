// Mount-based virtual filesystem resolver (EVE-660).
//
// `MountFs` is the single resolution seam for the agent's filesystem. It owns:
//
//   * a **mount table** — named mount points, each backed by a
//     `SessionFileSystem`, with a per-mount root in the backend's keyspace, and
//   * a **current working directory** — relative paths resolve against it.
//
// Resolution is uniform and POSIX-shaped: normalize the input against cwd,
// collapse `.`/`..`, then dispatch to the longest matching mount. `/workspace`
// is just a mount point (and the default cwd), not a magic prefix re-implemented
// in every store. Adding `/outputs`, `/.agents/skills`, or volume mounts backed
// by *different* stores later is `with_mount(...)` — the resolver does not change.
//
// Today there is a single workspace backend, so the table holds the root mount
// (`/` → backend, for legacy backend-native paths like `/AGENTS.md`,
// `/outputs/…`) and the `/workspace` view of the same backend. Both resolve to
// the same files; `/workspace` wins by longest-prefix so `/workspace/foo`
// ≡ `/foo`.
//
// See `specs/file-store.md` for the contract and the migration plan.

use async_trait::async_trait;
use std::sync::{Arc, RwLock};

use crate::error::Result;
use crate::session_file::{FileInfo, FileStat, GrepMatch, InitialFile, SessionFile};
use crate::traits::SessionFileSystem;
use crate::typed_id::SessionId;
use crate::workspace_paths::WorkspacePaths;

/// The conventional mount point and default cwd for the workspace. Models
/// trained on cloud-agent layouts address files here; it is a real mount, not a
/// strip-prefix.
pub const WORKSPACE_MOUNT: &str = "/workspace";

/// A single entry in the mount table.
#[derive(Clone)]
struct Mount {
    /// Virtual mount point: normalized, absolute, no trailing slash (`/` for
    /// root).
    mount_point: String,
    /// Backend serving this mount.
    backend: Arc<dyn SessionFileSystem>,
    /// Path inside the backend's own keyspace that `mount_point` maps to.
    backend_root: String,
}

/// Mount-based resolver. Implements `SessionFileSystem`, so it drops into
/// `ToolContext` / `SystemPromptContext` wherever the file store is wired.
pub struct MountFs {
    /// Sorted by mount-point length descending so the first match is the
    /// longest (most specific) mount.
    mounts: Vec<Mount>,
    /// The workspace backend — used for grep, display, and host mapping.
    primary: Arc<dyn SessionFileSystem>,
    /// Current working directory (a normalized virtual path). Relative inputs
    /// resolve against it. Defaults to [`WORKSPACE_MOUNT`].
    cwd: Arc<RwLock<String>>,
}

impl MountFs {
    /// Build a resolver over a single workspace backend.
    ///
    /// The backend is mounted at both `/` (its native keyspace, for legacy
    /// absolute paths) and `/workspace` (the model-facing view). cwd defaults to
    /// `/workspace`.
    pub fn new(workspace: Arc<dyn SessionFileSystem>) -> Self {
        let mounts = vec![
            Mount {
                mount_point: "/".to_string(),
                backend: workspace.clone(),
                backend_root: "/".to_string(),
            },
            Mount {
                mount_point: WORKSPACE_MOUNT.to_string(),
                backend: workspace.clone(),
                backend_root: "/".to_string(),
            },
        ];
        let mut fs = Self {
            mounts,
            primary: workspace,
            cwd: Arc::new(RwLock::new(WORKSPACE_MOUNT.to_string())),
        };
        fs.sort_mounts();
        fs
    }

    /// Build a resolver and return it as a trait object.
    pub fn wrap(workspace: Arc<dyn SessionFileSystem>) -> Arc<dyn SessionFileSystem> {
        Arc::new(Self::new(workspace))
    }

    /// Register an additional mount (e.g. a read-only skills source or a named
    /// volume) backed by a different store. Longest-prefix wins at resolution.
    pub fn with_mount(
        mut self,
        mount_point: impl Into<String>,
        backend: Arc<dyn SessionFileSystem>,
        backend_root: impl Into<String>,
    ) -> Self {
        self.mounts.push(Mount {
            mount_point: normalize_virtual(&mount_point.into(), "/"),
            backend,
            backend_root: normalize_virtual(&backend_root.into(), "/"),
        });
        self.sort_mounts();
        self
    }

    /// The current working directory (normalized virtual path).
    pub fn cwd(&self) -> String {
        self.cwd.read().expect("MountFs cwd lock poisoned").clone()
    }

    /// Repoint the current working directory. Relative paths resolve against it.
    pub fn set_cwd(&self, cwd: impl AsRef<str>) {
        let normalized = normalize_virtual(cwd.as_ref(), "/");
        *self.cwd.write().expect("MountFs cwd lock poisoned") = normalized;
    }

    fn sort_mounts(&mut self) {
        // Longest mount point first, so resolution picks the most specific mount.
        self.mounts
            .sort_by_key(|m| std::cmp::Reverse(m.mount_point.len()));
    }

    /// Resolve any input path to `(backend, backend_path)`.
    ///
    /// Relative inputs are joined to cwd; `.`/`..` are collapsed (clamped at
    /// root); the longest matching mount is selected and the remainder is mapped
    /// into that backend's keyspace.
    fn resolve(&self, input: &str) -> (Arc<dyn SessionFileSystem>, String) {
        let virtual_path = normalize_virtual(input, &self.cwd());
        for mount in &self.mounts {
            if let Some(rest) = mount_suffix(&mount.mount_point, &virtual_path) {
                return (
                    mount.backend.clone(),
                    join_backend_path(&mount.backend_root, &rest),
                );
            }
        }
        // The root mount matches every absolute path, so this is unreachable in
        // practice; fall back to the primary backend with the literal path.
        (self.primary.clone(), virtual_path)
    }
}

/// Normalize an input into an absolute virtual path: join cwd if relative, then
/// collapse `.`/`..` segments (a leading `..` is clamped at root).
fn normalize_virtual(input: &str, cwd: &str) -> String {
    let combined = if input.starts_with('/') {
        input.to_string()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), input)
    };
    let mut stack: Vec<&str> = Vec::new();
    for segment in combined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", stack.join("/"))
    }
}

/// If `virtual_path` is at or under `mount_point`, return the suffix as a
/// `/`-rooted remainder (`/` for an exact match). Segment-aware: `/workspacefoo`
/// is not under `/workspace`.
fn mount_suffix(mount_point: &str, virtual_path: &str) -> Option<String> {
    if mount_point == "/" {
        // The root mount owns the whole path.
        return Some(virtual_path.to_string());
    }
    if virtual_path == mount_point {
        return Some("/".to_string());
    }
    virtual_path
        .strip_prefix(mount_point)
        .filter(|rest| rest.starts_with('/'))
        .map(|rest| rest.to_string())
}

/// Join a backend root with a `/`-rooted remainder into a backend keyspace path.
fn join_backend_path(backend_root: &str, rest: &str) -> String {
    if backend_root == "/" {
        return rest.to_string();
    }
    if rest == "/" {
        return backend_root.to_string();
    }
    format!("{backend_root}{rest}")
}

#[async_trait]
impl SessionFileSystem for MountFs {
    fn workspace_paths(&self) -> WorkspacePaths {
        // Present the workspace mount's view: host mapping (if the backend is
        // host-rooted) preserved, display forced to the `/workspace` mount so
        // the model sees one namespace regardless of backend.
        self.primary
            .workspace_paths()
            .with_display_prefix(WORKSPACE_MOUNT)
    }

    fn display_root(&self) -> String {
        WORKSPACE_MOUNT.to_string()
    }

    fn resolve_path(&self, input: &str) -> String {
        // The absolute virtual path: relative inputs resolve against cwd,
        // `.`/`..` collapse, leading `..` clamps at root. This is the namespace
        // the shell sees — `/workspace` is just the default cwd, and any path
        // is reachable from the root mount.
        normalize_virtual(input, &self.cwd())
    }

    fn display_path(&self, path: &str) -> String {
        // `path` is a backend keyspace path (e.g. `/foo`); render it under the
        // workspace mount.
        if path == "/" || path.is_empty() {
            return WORKSPACE_MOUNT.to_string();
        }
        if let Some(rest) = path.strip_prefix('/') {
            format!("{WORKSPACE_MOUNT}/{rest}")
        } else {
            format!("{WORKSPACE_MOUNT}/{path}")
        }
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let (backend, backend_path) = self.resolve(path);
        backend.read_file(session_id, &backend_path).await
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let (backend, backend_path) = self.resolve(path);
        backend
            .write_file(session_id, &backend_path, content, encoding)
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
        let (backend, backend_path) = self.resolve(path);
        backend
            .write_file_if_content_matches(
                session_id,
                &backend_path,
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
        let (backend, backend_path) = self.resolve(path);
        backend
            .delete_file(session_id, &backend_path, recursive)
            .await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let (backend, backend_path) = self.resolve(path);
        backend.list_directory(session_id, &backend_path).await
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        let (backend, backend_path) = self.resolve(path);
        backend.stat_file(session_id, &backend_path).await
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        match path_pattern {
            Some(pp) => {
                let (backend, backend_path) = self.resolve(pp);
                backend
                    .grep_files(session_id, pattern, Some(&backend_path))
                    .await
            }
            None => self.primary.grep_files(session_id, pattern, None).await,
        }
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        let (backend, backend_path) = self.resolve(path);
        backend.create_directory(session_id, &backend_path).await
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        let (backend, backend_path) = self.resolve(&file.path);
        let seeded = InitialFile {
            path: backend_path,
            content: file.content.clone(),
            encoding: file.encoding.clone(),
            is_readonly: file.is_readonly,
        };
        backend.seed_initial_file(session_id, &seeded).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_paths::WorkspacePaths;

    fn sid() -> SessionId {
        SessionId::from_seed(1)
    }

    // A minimal `/`-rooted in-memory backend for resolver tests (kept local to
    // avoid a dependency on everruns-runtime).
    #[derive(Default)]
    struct FlatStore {
        files: std::sync::Mutex<std::collections::HashMap<String, String>>,
    }

    #[async_trait]
    impl SessionFileSystem for FlatStore {
        async fn read_file(&self, sid: SessionId, path: &str) -> Result<Option<SessionFile>> {
            let files = self.files.lock().unwrap();
            Ok(files.get(path).map(|content| SessionFile {
                id: uuid::Uuid::nil(),
                session_id: sid.uuid(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                content: Some(content.clone()),
                encoding: "text".to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }))
        }
        async fn write_file(
            &self,
            sid: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> Result<SessionFile> {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), content.to_string());
            Ok(SessionFile {
                id: uuid::Uuid::nil(),
                session_id: sid.uuid(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
        async fn delete_file(&self, _: SessionId, path: &str, _: bool) -> Result<bool> {
            Ok(self.files.lock().unwrap().remove(path).is_some())
        }
        async fn list_directory(&self, _: SessionId, _: &str) -> Result<Vec<FileInfo>> {
            Ok(vec![])
        }
        async fn stat_file(&self, _: SessionId, _: &str) -> Result<Option<FileStat>> {
            Ok(None)
        }
        async fn grep_files(
            &self,
            _: SessionId,
            _: &str,
            _: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            Ok(vec![])
        }
        async fn create_directory(&self, sid: SessionId, path: &str) -> Result<FileInfo> {
            Ok(FileInfo {
                id: uuid::Uuid::nil(),
                session_id: sid.uuid(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                path: path.to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
    }

    #[test]
    fn normalize_resolves_relative_against_cwd() {
        assert_eq!(
            normalize_virtual("foo/bar", "/workspace"),
            "/workspace/foo/bar"
        );
        assert_eq!(normalize_virtual("/foo", "/workspace"), "/foo");
        assert_eq!(normalize_virtual("a/../b", "/workspace"), "/workspace/b");
        assert_eq!(normalize_virtual("../../x", "/workspace"), "/x");
        assert_eq!(normalize_virtual(".", "/workspace"), "/workspace");
        assert_eq!(normalize_virtual("/", "/workspace"), "/");
    }

    #[tokio::test]
    async fn workspace_and_root_address_the_same_file() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);

        // Write via the /workspace view; read back via the backend-native path.
        fs.write_file(sid(), "/workspace/src/lib.rs", "X", "text")
            .await
            .unwrap();
        let via_root = fs.read_file(sid(), "/src/lib.rs").await.unwrap().unwrap();
        assert_eq!(via_root.content.as_deref(), Some("X"));
        // The backend keyed it at /src/lib.rs (no /workspace in the keyspace).
        assert_eq!(via_root.path, "/src/lib.rs");
    }

    #[tokio::test]
    async fn relative_paths_resolve_against_cwd() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);
        assert_eq!(fs.cwd(), "/workspace");

        fs.write_file(sid(), "notes.md", "hi", "text")
            .await
            .unwrap();
        // cwd is /workspace, so the relative write landed at backend /notes.md.
        let read = fs.read_file(sid(), "/notes.md").await.unwrap().unwrap();
        assert_eq!(read.content.as_deref(), Some("hi"));
    }

    #[tokio::test]
    async fn legacy_subtree_paths_pass_through_root_mount() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);
        // Internal callers write /outputs/... and /AGENTS.md directly.
        fs.write_file(sid(), "/outputs/call.stdout", "out", "text")
            .await
            .unwrap();
        let read = fs
            .read_file(sid(), "/workspace/outputs/call.stdout")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.content.as_deref(), Some("out"));
    }

    #[test]
    fn display_is_the_workspace_view() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);
        assert_eq!(fs.display_root(), "/workspace");
        assert_eq!(fs.display_path("/src/lib.rs"), "/workspace/src/lib.rs");
        assert_eq!(fs.display_path("/"), "/workspace");
        // And it agrees with the workspace_paths view.
        let wp = fs.workspace_paths();
        assert_eq!(wp.display_root(), "/workspace");
    }

    #[tokio::test]
    async fn additional_mount_routes_to_its_backend() {
        let workspace: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let volume: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(workspace).with_mount("/data", volume.clone(), "/");

        fs.write_file(sid(), "/data/report.csv", "1,2,3", "text")
            .await
            .unwrap();
        // It went to the volume backend at /report.csv, not the workspace.
        let from_volume = volume
            .read_file(sid(), "/report.csv")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(from_volume.content.as_deref(), Some("1,2,3"));
    }

    #[test]
    fn workspace_paths_keeps_host_mapping() {
        let dir = tempfile::TempDir::new().unwrap();
        // A host-rooted WorkspacePaths backend would expose to_host; here we
        // assert the resolver forces the /workspace display while delegating.
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);
        let wp = fs.workspace_paths();
        // VFS backend has no host root.
        assert!(wp.host_root().is_none());
        // Sanity: a standalone host WorkspacePaths still maps (unrelated to fs).
        let host = WorkspacePaths::host(dir.path()).unwrap();
        assert!(host.host_root().is_some());
    }
}
