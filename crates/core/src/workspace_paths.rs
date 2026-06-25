// Unified workspace path model (EVE-660).
//
// Everruns historically carried two incompatible path namespaces for the same
// workspace: a VFS/tool namespace that taught models `/workspace/...`, and a
// host namespace used by bash, spawn cwd, and embedder capabilities. Every
// host-path consumer independently re-implemented `/workspace` stripping,
// containment checks, and worktree-root updates. `WorkspacePaths` is the single
// addressing seam that all of them share.
//
// Design intent and the migration plan live in `specs/file-store.md`.
//
// Key invariants:
//   * The canonical, in-memory representation is workspace-*relative* (`RelPath`):
//     normalized, no `..`, no leading slash, no host prefix.
//   * The on-the-wire form used by `SessionFileSystem` / the DB store stays the
//     legacy leading-slash session path (`/src/lib.rs`); `RelPath::to_session_path`
//     produces it so existing storage backends need no change.
//   * `/workspace` is a *display alias* over the workspace root, not a second
//     namespace. `to_display` is the only place a prefix is added.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::error::{AgentLoopError, Result};

/// The default display prefix shown to models and UIs for the workspace root.
///
/// `/workspace` is a common cloud-agent convention (DevContainers, Codespaces).
/// It is purely a *view* over the workspace root — see [`WorkspacePaths`].
pub const DEFAULT_DISPLAY_PREFIX: &str = "/workspace";

/// A canonical workspace-relative path.
///
/// Always normalized: forward-slash separated, no leading slash, no `.`/`..`
/// segments, no host prefix. The workspace root is the empty path.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub struct RelPath(String);

impl RelPath {
    /// The workspace root.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// True when this path addresses the workspace root.
    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    /// The relative form: `src/lib.rs` (empty string for the root).
    pub fn as_relative(&self) -> &str {
        &self.0
    }

    /// The legacy leading-slash session/VFS path: `/src/lib.rs` (`/` for root).
    ///
    /// This is the form `SessionFileSystem` methods and the DB-backed store key
    /// on, so callers bridging to those layers serialize through here.
    pub fn to_session_path(&self) -> String {
        if self.0.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", self.0)
        }
    }
}

/// The single workspace path parser/formatter shared by every backend.
///
/// A `WorkspacePaths` optionally binds a host filesystem root (set for
/// real-disk stores and host shells; absent for the pure in-memory / DB VFS)
/// and carries a display prefix used to render model-facing paths. Clones share
/// the same host-root handle, so a worktree switch via [`set_host_root`] is seen
/// by every consumer that cloned from the same instance — the file store, the
/// shell, and host-path capabilities all agree on the current root.
///
/// [`set_host_root`]: WorkspacePaths::set_host_root
#[derive(Clone, Debug)]
pub struct WorkspacePaths {
    /// Host filesystem root, shared so worktree switches propagate to clones.
    /// `None` for the in-memory / DB VFS which has no host mapping.
    host_root: Option<Arc<RwLock<PathBuf>>>,
    /// Literal display prefix. `None` means "derive from the host root" (the
    /// real-disk default, which displays host-absolute paths). `Some("")`
    /// displays bare relative paths.
    display_prefix: Option<Arc<str>>,
}

impl Default for WorkspacePaths {
    fn default() -> Self {
        Self::vfs()
    }
}

impl WorkspacePaths {
    /// A VFS path model with no host root and the default `/workspace` display
    /// prefix. This is what the in-memory and DB-backed stores use.
    pub fn vfs() -> Self {
        Self {
            host_root: None,
            display_prefix: Some(Arc::from(DEFAULT_DISPLAY_PREFIX)),
        }
    }

    /// A host-rooted path model bound to `root`.
    ///
    /// The root is canonicalized once (resolving symlinks). Returns an error if
    /// it does not exist or is not a directory. The display prefix defaults to
    /// "derive from host root", preserving real-disk's host-absolute display.
    pub fn host(root: impl Into<PathBuf>) -> Result<Self> {
        let canonical = canonicalize_root(root.into())?;
        Ok(Self {
            host_root: Some(Arc::new(RwLock::new(canonical))),
            display_prefix: None,
        })
    }

    /// Override the model-facing display prefix.
    ///
    /// Pass `/workspace` to show the cloud-agent alias even for a host-rooted
    /// workspace, or `""` to show bare relative paths.
    pub fn with_display_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.display_prefix = Some(Arc::from(prefix.into().as_str()));
        self
    }

    /// The current host root, if this workspace is host-bound.
    pub fn host_root(&self) -> Option<PathBuf> {
        self.host_root.as_ref().map(|handle| {
            handle
                .read()
                .expect("workspace host root lock poisoned")
                .clone()
        })
    }

    /// Repoint the host root — used when an embedder switches worktrees.
    ///
    /// Canonicalizes and validates the new root, then updates the shared handle
    /// so every clone (file store, shell, capabilities) sees the switch.
    /// Returns an error for a VFS workspace that has no host root.
    pub fn set_host_root(&self, root: impl Into<PathBuf>) -> Result<()> {
        let handle = self.host_root.as_ref().ok_or_else(|| {
            AgentLoopError::config("cannot set host root on a VFS workspace (no host filesystem)")
        })?;
        let canonical = canonicalize_root(root.into())?;
        *handle.write().expect("workspace host root lock poisoned") = canonical;
        Ok(())
    }

    /// Parse any accepted model/tool path input into a canonical [`RelPath`].
    ///
    /// Accepts (all addressing the same canonical path):
    ///   * relative: `src/foo`
    ///   * absolute session: `/src/foo`
    ///   * display-alias: `/workspace/src/foo`, `/workspace`
    ///   * host-absolute under the root (when host-bound): `/host/root/src/foo`
    ///
    /// Rejects `..` traversal anywhere and paths that escape the root. Bare
    /// `workspace/...` (no leading slash) is intentionally *not* treated as the
    /// alias, so a real top-level `workspace/` directory stays reachable.
    pub fn parse_input(&self, input: &str) -> Result<RelPath> {
        let trimmed = input.trim();

        // Host-absolute paths under the root are aliases for the same canonical
        // path (e.g. an editor or model that echoes the real checkout path).
        if let Some(root) = self.host_root() {
            let candidate = Path::new(trimmed);
            if candidate.is_absolute()
                && let Ok(relative) = candidate.strip_prefix(&root)
            {
                return rel_from_path(relative);
            }
        }

        let stripped = strip_alias_prefix(trimmed, self.display_prefix.as_deref());
        rel_from_str(stripped)
    }

    /// Canonical path → host filesystem path (for bash cwd, `std::fs`, spawn).
    ///
    /// Errors on a VFS workspace, or if the result would escape the root.
    pub fn to_host(&self, path: &RelPath) -> Result<PathBuf> {
        let root = self.require_host_root()?;
        if path.is_root() {
            return Ok(root);
        }
        let candidate = root.join(path.as_relative());
        if !candidate.starts_with(&root) {
            return Err(AgentLoopError::tool(format!(
                "path escapes workspace root: {}",
                path.as_relative()
            )));
        }
        Ok(candidate)
    }

    /// Host path under the root → canonical, if contained.
    pub fn from_host(&self, host: &Path) -> Result<RelPath> {
        let root = self.require_host_root()?;
        let relative = host.strip_prefix(&root).map_err(|_| {
            AgentLoopError::tool(format!(
                "path is outside workspace root: {}",
                host.display()
            ))
        })?;
        rel_from_path(relative)
    }

    /// Parse an absolute path produced by a shell (bashkit resolves every path
    /// against cwd before handing it to the filesystem), returning `None` when
    /// it falls *outside* the workspace mount rather than leniently
    /// reinterpreting it as workspace-relative.
    ///
    /// This preserves the "the shell can only touch the workspace" containment
    /// while recognizing every accepted spelling of an in-mount path: the
    /// `/workspace` alias, the configured display prefix, and — crucially for
    /// real-disk stores — host-absolute paths under the root. The latter is
    /// what lets a model use the same path with `read_file` and with `cat`.
    pub fn parse_mounted(&self, input: &str) -> Option<RelPath> {
        let trimmed = input.trim();

        // The `/workspace` alias / configured display prefix is always in-mount.
        let stripped = strip_alias_prefix(trimmed, self.display_prefix.as_deref());
        if !std::ptr::eq(stripped, trimmed) {
            return rel_from_str(stripped).ok();
        }

        match self.host_root() {
            Some(root) => {
                let candidate = Path::new(trimmed);
                if candidate.is_absolute() {
                    // Only host-absolute paths under the root are in the mount.
                    candidate
                        .strip_prefix(&root)
                        .ok()
                        .and_then(|rel| rel_from_path(rel).ok())
                } else {
                    rel_from_str(trimmed).ok()
                }
            }
            None => {
                // VFS: an absolute path that didn't match the alias is outside
                // the mount; relative paths map into it.
                if trimmed.starts_with('/') {
                    None
                } else {
                    rel_from_str(trimmed).ok()
                }
            }
        }
    }

    /// Canonical path → model-facing display string.
    pub fn to_display(&self, path: &RelPath) -> String {
        let root = self.display_root();
        if path.is_root() {
            return root;
        }
        // Empty prefix => bare relative display.
        if root.is_empty() {
            return path.as_relative().to_string();
        }
        if root.ends_with('/') {
            format!("{root}{}", path.as_relative())
        } else {
            format!("{root}/{}", path.as_relative())
        }
    }

    /// The human-facing root label (display prefix, host root, or `/workspace`).
    pub fn display_root(&self) -> String {
        match &self.display_prefix {
            Some(prefix) => prefix.to_string(),
            None => match self.host_root() {
                Some(root) => root.display().to_string(),
                None => DEFAULT_DISPLAY_PREFIX.to_string(),
            },
        }
    }

    /// A validated working directory for spawning a host process.
    ///
    /// Fails clearly when the workspace directory is missing (e.g. a removed
    /// worktree) instead of surfacing an opaque `No such file or directory`
    /// from the spawn itself.
    pub fn spawn_cwd(&self) -> Result<PathBuf> {
        let root = self.require_host_root()?;
        if !root.is_dir() {
            return Err(AgentLoopError::tool(format!(
                "workspace directory does not exist: {}",
                root.display()
            )));
        }
        Ok(root)
    }

    fn require_host_root(&self) -> Result<PathBuf> {
        self.host_root().ok_or_else(|| {
            AgentLoopError::tool("workspace has no host filesystem root".to_string())
        })
    }
}

/// Strip the `/workspace` alias from a leading-slash session path, lenient about
/// `..` (the underlying store's `resolve` is the traversal defense for host
/// stores; the flat VFS does not need one). This is the single normalizer the
/// in-memory store and `file_system` capability share.
///
/// Examples: `/workspace` → `/`, `/workspace/a.txt` → `/a.txt`,
/// `/workspacefoo` → `/workspacefoo`, `a.txt` → `/a.txt`.
pub fn to_session_path(input: &str) -> String {
    let stripped = strip_workspace_alias(input);
    if stripped.is_empty() || stripped == "/" {
        return "/".to_string();
    }
    let mut normalized = if stripped.starts_with('/') {
        stripped.to_string()
    } else {
        format!("/{stripped}")
    };
    while normalized.len() > 1 && normalized.ends_with('/') {
        normalized.pop();
    }
    normalized
}

/// Strip the canonical `/workspace` alias only (exact match or `/workspace/`
/// prefix). Returns the remainder, which may or may not have a leading slash.
fn strip_workspace_alias(s: &str) -> &str {
    if s == DEFAULT_DISPLAY_PREFIX {
        return "";
    }
    if let Some(rest) = s.strip_prefix(&format!("{DEFAULT_DISPLAY_PREFIX}/")) {
        return rest;
    }
    s
}

/// Strip the canonical `/workspace` alias and, if configured and absolute, the
/// instance's display prefix too. Returns the remainder for segment parsing.
fn strip_alias_prefix<'a>(s: &'a str, display_prefix: Option<&str>) -> &'a str {
    let after_workspace = strip_workspace_alias(s);
    if !std::ptr::eq(after_workspace, s) {
        return after_workspace;
    }
    if let Some(prefix) = display_prefix
        && prefix.starts_with('/')
        && prefix != DEFAULT_DISPLAY_PREFIX
    {
        if s == prefix {
            return "";
        }
        if let Some(rest) = s.strip_prefix(&format!("{prefix}/")) {
            return rest;
        }
    }
    s
}

/// Normalize a slash-separated string into a [`RelPath`], rejecting traversal.
fn rel_from_str(s: &str) -> Result<RelPath> {
    let mut segments = Vec::new();
    for part in s.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                return Err(AgentLoopError::tool(format!(
                    "path traversal rejected: {s}"
                )));
            }
            segment => segments.push(segment),
        }
    }
    Ok(RelPath(segments.join("/")))
}

/// Normalize a host-relative `Path` into a [`RelPath`], rejecting traversal and
/// non-UTF-8 components. `CurDir` (`.`) segments are skipped so host aliases
/// like `<root>/./src/lib.rs` resolve cleanly.
fn rel_from_path(relative: &Path) -> Result<RelPath> {
    let mut segments = Vec::new();
    for component in relative.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(seg) => {
                let segment = seg.to_str().ok_or_else(|| {
                    AgentLoopError::tool(format!(
                        "non-UTF-8 path component: {}",
                        relative.display()
                    ))
                })?;
                segments.push(segment.to_string());
            }
            Component::ParentDir => {
                return Err(AgentLoopError::tool(format!(
                    "path traversal rejected: {}",
                    relative.display()
                )));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(AgentLoopError::tool(format!(
                    "absolute path component rejected: {}",
                    relative.display()
                )));
            }
        }
    }
    Ok(RelPath(segments.join("/")))
}

fn canonicalize_root(root: PathBuf) -> Result<PathBuf> {
    if !root.exists() {
        return Err(AgentLoopError::config(format!(
            "workspace directory does not exist: {}",
            root.display()
        )));
    }
    let canonical = std::fs::canonicalize(&root).map_err(|e| {
        AgentLoopError::config(format!(
            "failed to canonicalize workspace root {}: {e}",
            root.display()
        ))
    })?;
    if !canonical.is_dir() {
        return Err(AgentLoopError::config(format!(
            "workspace root is not a directory: {}",
            canonical.display()
        )));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn vfs_parses_alias_forms_to_same_canonical() {
        let wp = WorkspacePaths::vfs();
        for input in [
            "src/lib.rs",
            "/src/lib.rs",
            "/workspace/src/lib.rs",
            "  /workspace/src/lib.rs  ",
        ] {
            let rel = wp.parse_input(input).expect("parse");
            assert_eq!(rel.as_relative(), "src/lib.rs", "input: {input:?}");
            assert_eq!(rel.to_session_path(), "/src/lib.rs", "input: {input:?}");
        }
    }

    #[test]
    fn workspace_root_aliases_normalize_to_root() {
        let wp = WorkspacePaths::vfs();
        for input in ["/workspace", "/", "", "."] {
            let rel = wp.parse_input(input).expect("parse");
            assert!(rel.is_root(), "input: {input:?}");
            assert_eq!(rel.to_session_path(), "/");
        }
    }

    #[test]
    fn bare_workspace_dir_is_not_aliased() {
        let wp = WorkspacePaths::vfs();
        // A real top-level `workspace/` directory must stay reachable.
        let rel = wp.parse_input("workspace/notes.md").expect("parse");
        assert_eq!(rel.as_relative(), "workspace/notes.md");
        // `/workspacefoo` is not the `/workspace` segment.
        let rel = wp.parse_input("/workspacefoo").expect("parse");
        assert_eq!(rel.as_relative(), "workspacefoo");
    }

    #[test]
    fn traversal_is_rejected() {
        let wp = WorkspacePaths::vfs();
        for input in ["../outside", "/workspace/../etc/passwd", "a/../../b"] {
            let err = wp.parse_input(input).expect_err("must reject");
            assert!(format!("{err}").contains("traversal"), "input: {input:?}");
        }
    }

    #[test]
    fn host_round_trip() {
        let dir = TempDir::new().unwrap();
        let wp = WorkspacePaths::host(dir.path()).expect("host");
        let root = std::fs::canonicalize(dir.path()).unwrap();

        let rel = wp.parse_input("/workspace/src/lib.rs").expect("parse");
        let host = wp.to_host(&rel).expect("to_host");
        assert_eq!(host, root.join("src/lib.rs"));

        let back = wp.from_host(&host).expect("from_host");
        assert_eq!(back, rel);
        assert_eq!(back.to_session_path(), "/src/lib.rs");
    }

    #[test]
    fn host_absolute_input_is_an_alias() {
        let dir = TempDir::new().unwrap();
        let wp = WorkspacePaths::host(dir.path()).expect("host");
        let root = std::fs::canonicalize(dir.path()).unwrap();

        let via_host = wp
            .parse_input(&root.join("src/lib.rs").display().to_string())
            .expect("parse host abs");
        let via_workspace = wp
            .parse_input("/workspace/src/lib.rs")
            .expect("parse alias");
        assert_eq!(via_host, via_workspace);

        // `.` segments in host aliases resolve cleanly.
        let via_curdir = wp
            .parse_input(&root.join("./src/lib.rs").display().to_string())
            .expect("parse curdir");
        assert_eq!(via_curdir, via_workspace);
    }

    #[test]
    fn host_display_is_host_absolute_by_default() {
        let dir = TempDir::new().unwrap();
        let wp = WorkspacePaths::host(dir.path()).expect("host");
        let root = std::fs::canonicalize(dir.path()).unwrap();

        assert_eq!(wp.display_root(), root.display().to_string());
        let rel = wp.parse_input("/src/lib.rs").unwrap();
        assert_eq!(
            wp.to_display(&rel),
            root.join("src/lib.rs").display().to_string()
        );
    }

    #[test]
    fn configurable_display_prefix() {
        let dir = TempDir::new().unwrap();
        let wp = WorkspacePaths::host(dir.path())
            .unwrap()
            .with_display_prefix("/workspace");
        let rel = wp.parse_input("/src/lib.rs").unwrap();
        assert_eq!(wp.to_display(&rel), "/workspace/src/lib.rs");

        let relative = WorkspacePaths::host(dir.path())
            .unwrap()
            .with_display_prefix("");
        assert_eq!(relative.to_display(&rel), "src/lib.rs");
        assert_eq!(relative.to_display(&RelPath::root()), "");
    }

    #[test]
    fn missing_root_spawn_error_is_clear() {
        let dir = TempDir::new().unwrap();
        let wp = WorkspacePaths::host(dir.path()).expect("host");
        // Healthy root spawns fine.
        assert_eq!(
            wp.spawn_cwd().unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );

        // Removing the directory (e.g. `git worktree remove`) yields a clear
        // error rather than an opaque spawn failure. The canonical root handle
        // still points at the now-deleted path.
        drop(dir);
        let err = wp.spawn_cwd().expect_err("missing root must fail clearly");
        assert!(
            format!("{err}").contains("workspace directory does not exist"),
            "got: {err}"
        );
    }

    #[test]
    fn set_host_root_propagates_to_clones() {
        let first = TempDir::new().unwrap();
        let second = TempDir::new().unwrap();
        let wp = WorkspacePaths::host(first.path()).expect("host");
        let clone = wp.clone();

        wp.set_host_root(second.path()).expect("switch root");

        let expected = std::fs::canonicalize(second.path()).unwrap();
        assert_eq!(clone.host_root().unwrap(), expected);
        let rel = clone.parse_input("/a.txt").unwrap();
        assert_eq!(clone.to_host(&rel).unwrap(), expected.join("a.txt"));
    }

    #[test]
    fn vfs_has_no_host_mapping() {
        let wp = WorkspacePaths::vfs();
        assert!(wp.host_root().is_none());
        let rel = wp.parse_input("/a.txt").unwrap();
        assert!(wp.to_host(&rel).is_err());
        assert!(wp.spawn_cwd().is_err());
        assert!(wp.set_host_root("/tmp").is_err());
    }

    #[test]
    fn parse_mounted_vfs_contains_only_workspace() {
        let wp = WorkspacePaths::vfs();
        assert_eq!(
            wp.parse_mounted("/workspace/src/lib.rs")
                .unwrap()
                .to_session_path(),
            "/src/lib.rs"
        );
        assert_eq!(
            wp.parse_mounted("/workspace").unwrap().to_session_path(),
            "/"
        );
        // Absolute paths outside the alias are outside the mount.
        assert!(wp.parse_mounted("/tmp/file.txt").is_none());
        assert!(wp.parse_mounted("/home/agent/x").is_none());
        assert!(wp.parse_mounted("/workspacefoo").is_none());
    }

    #[test]
    fn parse_mounted_host_accepts_host_absolute_and_alias() {
        let dir = TempDir::new().unwrap();
        let wp = WorkspacePaths::host(dir.path()).unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();

        // Host-absolute under the root is in-mount (the real-disk fix).
        assert_eq!(
            wp.parse_mounted(&root.join("src/lib.rs").display().to_string())
                .unwrap()
                .to_session_path(),
            "/src/lib.rs"
        );
        // The `/workspace` alias still works even on a host store.
        assert_eq!(
            wp.parse_mounted("/workspace/src/lib.rs")
                .unwrap()
                .to_session_path(),
            "/src/lib.rs"
        );
        // Host-absolute outside the root is rejected.
        assert!(wp.parse_mounted("/etc/passwd").is_none());
    }

    #[test]
    fn free_to_session_path_matches_legacy_normalizer() {
        assert_eq!(to_session_path("/workspace"), "/");
        assert_eq!(to_session_path("/workspace/test.txt"), "/test.txt");
        assert_eq!(
            to_session_path("/workspace/foo/bar/test.txt"),
            "/foo/bar/test.txt"
        );
        assert_eq!(to_session_path("/test.txt"), "/test.txt");
        assert_eq!(to_session_path("/workspacefoo"), "/workspacefoo");
        assert_eq!(to_session_path("foo.txt"), "/foo.txt");
        assert_eq!(to_session_path("/sub/dir/"), "/sub/dir");
    }
}
