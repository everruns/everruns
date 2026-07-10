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
use std::sync::Arc;

use crate::error::{AgentLoopError, Result};
use crate::session_file::{FileInfo, FileStat, GrepMatch, InitialFile, SessionFile};
use crate::traits::SessionFileSystem;
use crate::typed_id::SessionId;

/// The conventional mount point and default cwd for the workspace. Models
/// trained on cloud-agent layouts address files here; it is a real mount, not a
/// strip-prefix. Same string as [`crate::session_path::WORKSPACE_PREFIX`] (the
/// display alias) — kept as one source of truth.
pub const WORKSPACE_MOUNT: &str = crate::session_path::WORKSPACE_PREFIX;

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
    /// Whether returned backend paths should remain in the backend's canonical
    /// keyspace. The root and `/workspace` mounts preserve the legacy primary
    /// workspace contract (`/src/lib.rs`); named mounts report their mounted
    /// virtual path (`/workspace/roots/backend/src/lib.rs`).
    primary_workspace: bool,
}

#[derive(Clone)]
struct ResolvedMount {
    mount_point: String,
    backend: Arc<dyn SessionFileSystem>,
    backend_root: String,
    backend_path: String,
    primary_workspace: bool,
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
    /// resolve against it; defaults to [`WORKSPACE_MOUNT`]. Fixed at
    /// construction — persistent `cd` across tool calls is not a feature yet.
    cwd: String,
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
                primary_workspace: true,
            },
            Mount {
                mount_point: WORKSPACE_MOUNT.to_string(),
                backend: workspace.clone(),
                backend_root: "/".to_string(),
                primary_workspace: true,
            },
        ];
        let mut fs = Self {
            mounts,
            primary: workspace,
            cwd: WORKSPACE_MOUNT.to_string(),
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
            primary_workspace: false,
        });
        self.sort_mounts();
        self
    }

    /// The current working directory (normalized virtual path).
    pub fn cwd(&self) -> String {
        self.cwd.clone()
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
    fn resolve(&self, input: &str) -> Result<ResolvedMount> {
        reject_additional_root_traversal(input, &self.cwd)?;
        let virtual_path = normalize_virtual(input, &self.cwd());
        for mount in &self.mounts {
            if let Some(rest) = mount_suffix(&mount.mount_point, &virtual_path) {
                return Ok(ResolvedMount {
                    mount_point: mount.mount_point.clone(),
                    backend: mount.backend.clone(),
                    backend_root: mount.backend_root.clone(),
                    backend_path: join_backend_path(&mount.backend_root, &rest),
                    primary_workspace: mount.primary_workspace,
                });
            }
        }
        // The root mount matches every absolute path, so this is unreachable in
        // practice; fall back to the primary backend with the literal path.
        Ok(ResolvedMount {
            mount_point: "/".to_string(),
            backend: self.primary.clone(),
            backend_root: "/".to_string(),
            backend_path: virtual_path,
            primary_workspace: true,
        })
    }

    fn grep_mounts(&self) -> Vec<ResolvedMount> {
        let mut out: Vec<ResolvedMount> = Vec::new();
        for mount in self.mounts.iter().rev() {
            if out.iter().any(|existing| {
                Arc::ptr_eq(&existing.backend, &mount.backend)
                    && existing.backend_root == mount.backend_root
            }) {
                continue;
            }
            out.push(ResolvedMount {
                mount_point: mount.mount_point.clone(),
                backend: mount.backend.clone(),
                backend_root: mount.backend_root.clone(),
                backend_path: mount.backend_root.clone(),
                primary_workspace: mount.primary_workspace,
            });
        }
        out
    }
}

impl ResolvedMount {
    fn map_session_file(&self, mut file: SessionFile) -> SessionFile {
        file.path = self.to_virtual_output_path(&file.path);
        file.name = FileInfo::name_from_path(&file.path);
        file
    }

    fn map_file_info(&self, mut info: FileInfo) -> FileInfo {
        info.path = self.to_virtual_output_path(&info.path);
        info.name = FileInfo::name_from_path(&info.path);
        info
    }

    fn map_file_stat(&self, mut stat: FileStat) -> FileStat {
        stat.path = self.to_virtual_output_path(&stat.path);
        stat.name = FileInfo::name_from_path(&stat.path);
        stat
    }

    fn map_grep_match(&self, mut grep_match: GrepMatch) -> GrepMatch {
        grep_match.path = self.to_virtual_output_path(&grep_match.path);
        grep_match
    }

    fn to_virtual_output_path(&self, backend_path: &str) -> String {
        if self.primary_workspace {
            return normalize_virtual(backend_path, "/");
        }
        let normalized = normalize_virtual(backend_path, "/");
        let rest = mount_suffix(&self.backend_root, &normalized).unwrap_or(normalized);
        join_backend_path(&self.mount_point, &rest)
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

fn reject_additional_root_traversal(input: &str, cwd: &str) -> Result<()> {
    let combined = if input.starts_with('/') {
        input.to_string()
    } else {
        format!("{}/{}", cwd.trim_end_matches('/'), input)
    };
    let segments: Vec<&str> = combined
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    for window_start in 0..segments.len().saturating_sub(2) {
        if segments[window_start] == "workspace" && segments[window_start + 1] == "roots" {
            let root_name_idx = window_start + 2;
            if segments[root_name_idx].is_empty() {
                continue;
            }
            if segments
                .iter()
                .skip(root_name_idx + 1)
                .any(|segment| *segment == "..")
            {
                return Err(AgentLoopError::tool(format!(
                    "path traversal rejected: {input}"
                )));
            }
        }
    }
    Ok(())
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
    fn display_root(&self) -> String {
        self.primary.display_root()
    }

    fn resolve_path(&self, input: &str) -> String {
        // The absolute virtual path: relative inputs resolve against cwd,
        // `.`/`..` collapse, leading `..` clamps at root. This is the namespace
        // the shell sees — `/workspace` is just the default cwd, and any path
        // is reachable from the root mount.
        let virtual_path = normalize_virtual(input, &self.cwd());
        match self.resolve(&virtual_path) {
            Ok(resolved) if resolved.primary_workspace => {
                resolved.backend.display_path(&resolved.backend_path)
            }
            _ => virtual_path,
        }
    }

    fn display_path(&self, path: &str) -> String {
        let virtual_path = normalize_virtual(path, "/");
        match self.resolve(&virtual_path) {
            Ok(resolved) if resolved.primary_workspace => {
                resolved.backend.display_path(&resolved.backend_path)
            }
            _ => virtual_path,
        }
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let resolved = self.resolve(path)?;
        Ok(resolved
            .backend
            .read_file(session_id, &resolved.backend_path)
            .await?
            .map(|file| resolved.map_session_file(file)))
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let resolved = self.resolve(path)?;
        Ok(resolved.map_session_file(
            resolved
                .backend
                .write_file(session_id, &resolved.backend_path, content, encoding)
                .await?,
        ))
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
        let resolved = self.resolve(path)?;
        Ok(resolved
            .backend
            .write_file_if_content_matches(
                session_id,
                &resolved.backend_path,
                expected_content,
                expected_encoding,
                content,
                encoding,
            )
            .await?
            .map(|file| resolved.map_session_file(file)))
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        let resolved = self.resolve(path)?;
        resolved
            .backend
            .delete_file(session_id, &resolved.backend_path, recursive)
            .await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let resolved = self.resolve(path)?;
        Ok(resolved
            .backend
            .list_directory(session_id, &resolved.backend_path)
            .await?
            .into_iter()
            .map(|info| resolved.map_file_info(info))
            .collect())
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        let resolved = self.resolve(path)?;
        Ok(resolved
            .backend
            .stat_file(session_id, &resolved.backend_path)
            .await?
            .map(|stat| resolved.map_file_stat(stat)))
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        match path_pattern {
            Some(pp) => {
                let resolved = self.resolve(pp)?;
                Ok(resolved
                    .backend
                    .grep_files(session_id, pattern, Some(&resolved.backend_path))
                    .await?
                    .into_iter()
                    .map(|grep_match| resolved.map_grep_match(grep_match))
                    .collect())
            }
            None => {
                let mut matches = Vec::new();
                for resolved in self.grep_mounts() {
                    matches.extend(
                        resolved
                            .backend
                            .grep_files(session_id, pattern, Some(&resolved.backend_path))
                            .await?
                            .into_iter()
                            .map(|grep_match| resolved.map_grep_match(grep_match)),
                    );
                }
                matches.sort_by(|a, b| {
                    a.path
                        .cmp(&b.path)
                        .then(a.line_number.cmp(&b.line_number))
                        .then(a.line.cmp(&b.line))
                });
                Ok(matches)
            }
        }
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        let resolved = self.resolve(path)?;
        Ok(resolved.map_file_info(
            resolved
                .backend
                .create_directory(session_id, &resolved.backend_path)
                .await?,
        ))
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        let resolved = self.resolve(&file.path)?;
        let seeded = InitialFile {
            path: resolved.backend_path,
            content: file.content.clone(),
            encoding: file.encoding.clone(),
            is_readonly: file.is_readonly,
        };
        resolved
            .backend
            .seed_initial_file(session_id, &seeded)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        async fn stat_file(&self, _: SessionId, path: &str) -> Result<Option<FileStat>> {
            let files = self.files.lock().unwrap();
            Ok(files.get(path).map(|content| FileStat {
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }))
        }
        async fn grep_files(
            &self,
            _: SessionId,
            pattern: &str,
            path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            let files = self.files.lock().unwrap();
            let mut matches = Vec::new();
            for (path, content) in files.iter() {
                if let Some(filter) = path_pattern
                    && !path.contains(filter)
                {
                    continue;
                }
                for (idx, line) in content.lines().enumerate() {
                    if line.contains(pattern) {
                        matches.push(GrepMatch {
                            path: path.clone(),
                            line_number: idx + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
            Ok(matches)
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
    fn display_uses_the_primary_backends_identity() {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(backend);
        assert_eq!(fs.display_root(), "/workspace");
        assert_eq!(fs.display_path("/src/lib.rs"), "/workspace/src/lib.rs");
        assert_eq!(fs.display_path("/"), "/workspace");
        assert_eq!(fs.resolve_path("src/lib.rs"), "/workspace/src/lib.rs");
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

    #[tokio::test]
    async fn additional_mount_outputs_use_virtual_paths() {
        let workspace: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let volume: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(workspace).with_mount("/workspace/roots/backend", volume, "/");

        let written = fs
            .write_file(
                sid(),
                "/workspace/roots/backend/Cargo.toml",
                "name = \"backend\"",
                "text",
            )
            .await
            .unwrap();
        assert_eq!(written.path, "/workspace/roots/backend/Cargo.toml");

        let stat = fs
            .stat_file(sid(), "/workspace/roots/backend/Cargo.toml")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stat.path, "/workspace/roots/backend/Cargo.toml");
        assert_eq!(
            fs.display_path(&stat.path),
            "/workspace/roots/backend/Cargo.toml"
        );
        assert_eq!(
            fs.resolve_path("/workspace/roots/backend/Cargo.toml"),
            "/workspace/roots/backend/Cargo.toml"
        );
    }

    #[tokio::test]
    async fn grep_without_path_searches_all_mounts() {
        let workspace: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let volume: Arc<dyn SessionFileSystem> = Arc::new(FlatStore::default());
        let fs = MountFs::new(workspace).with_mount("/workspace/roots/backend", volume, "/");

        fs.write_file(sid(), "/workspace/README.md", "needle primary", "text")
            .await
            .unwrap();
        fs.write_file(
            sid(),
            "/workspace/roots/backend/Cargo.toml",
            "needle backend",
            "text",
        )
        .await
        .unwrap();

        let matches = fs.grep_files(sid(), "needle", None).await.unwrap();
        let paths: Vec<_> = matches.into_iter().map(|m| m.path).collect();
        assert_eq!(
            paths,
            vec![
                "/README.md".to_string(),
                "/workspace/roots/backend/Cargo.toml".to_string()
            ]
        );
    }
}
