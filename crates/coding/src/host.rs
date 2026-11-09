//! Where the code lives.
//!
//! The navigation tools run in the agent's own process, but the code they read
//! may be somewhere else. [`CodeHost`] is the one place that difference is
//! spelled out:
//!
//! - [`LocalCodeHost`] (feature `local`): the code is on this machine. Files
//!   are read from disk and language servers are child processes. This is what
//!   a terminal agent uses.
//! - [`SandboxCodeHost`]: the code is in the session's sandbox. Source files
//!   are fetched into a private temporary directory for parsing, edits are
//!   written back, and the language server runs inside the sandbox.
//!
//! Paths a host hands out as `String` are absolute paths *where the code
//! lives*. They are meaningful to programs running there (a language server),
//! not to this process.

#[cfg(feature = "local")]
mod local;
mod sandbox;

#[cfg(feature = "local")]
pub use local::LocalCodeHost;
pub use sandbox::SandboxCodeHost;

use crate::source::SourceFilter;
use anyhow::{Result, bail};
use async_trait::async_trait;
use everruns_contracts::runtime::tool_context::ToolContext;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};

/// Directory names never scanned, fetched, or edited by navigation tools.
pub(crate) const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "target",
    "node_modules",
    ".next",
    "dist",
    "build",
    ".direnv",
    ".venv",
    "venv",
    ".tox",
    ".gradle",
];

/// A directory on this machine holding the source files of one request.
///
/// For a local host it is the workspace itself. For a sandbox it is a private
/// copy that disappears when the tree is dropped.
pub struct SourceTree {
    /// Directory to scan. Request paths resolve beneath it.
    pub root: PathBuf,
    /// The root as the model knows it (for example `/workspace`).
    pub display_root: String,
    /// The request's `path`, relative to `root`, when one was given.
    pub scope: Option<String>,
    /// Set when the copy is incomplete, in words fit for a tool result.
    pub note: Option<String>,
    pub(crate) _guard: Option<tempfile::TempDir>,
}

/// One change to the code, applied by [`CodeHost::apply`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileOp {
    Write {
        path: String,
        content: String,
    },
    Create {
        path: String,
        overwrite: bool,
        ignore_if_exists: bool,
    },
    Rename {
        from: String,
        to: String,
    },
    Delete {
        path: String,
        recursive: bool,
    },
}

/// The command line of a long-lived helper process (a language server).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerCommand {
    pub command: String,
    pub args: Vec<String>,
}

/// Moves bytes for a server whose pipes cannot be held open.
///
/// A sandbox runs one command at a time, so nothing reads a remote server's
/// output unless someone asks. While a tool call is using a server it calls
/// [`Pump::tick`] in a loop; between calls the server simply waits.
#[async_trait]
pub trait Pump: Send + Sync {
    /// Send what the client wrote and collect what the server answered.
    /// Returns whether anything moved. An error means the server is gone.
    async fn tick(&self, context: &ToolContext) -> Result<bool>;
}

/// Byte streams to a started server.
pub struct ServerIo {
    pub reader: Box<dyn AsyncRead + Send + Unpin>,
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
    /// Present when the streams only move while pumped.
    pub pump: Option<Arc<dyn Pump>>,
    /// Dropped with the server: kills a local child process.
    pub guard: Option<Box<dyn std::any::Any + Send + Sync>>,
}

/// Access to the code a session works on. See the module docs.
#[async_trait]
pub trait CodeHost: Send + Sync {
    /// Partitions long-lived state (running language servers) between
    /// sessions that share this process.
    fn scope_key(&self, context: &ToolContext) -> String;

    /// The workspace root as the model names it.
    fn display_root(&self) -> String;

    /// A local directory holding the source files under `path` (the whole
    /// workspace when `None`). `filter` names the files the caller can use; a
    /// host that copies files may leave the rest behind.
    async fn materialize(
        &self,
        context: &ToolContext,
        path: Option<&str>,
        filter: &SourceFilter,
    ) -> Result<SourceTree>;

    /// Persist files a tool rewrote inside `tree` (paths relative to its root).
    async fn commit(
        &self,
        context: &ToolContext,
        tree: &SourceTree,
        changed: &[String],
    ) -> Result<()>;

    /// Absolute workspace root where the code lives.
    async fn root(&self, context: &ToolContext) -> Result<String>;

    /// Absolute path of a file the model named, refused when it would leave
    /// the workspace.
    async fn resolve_file(&self, context: &ToolContext, path: &str) -> Result<String>;

    /// The project a file belongs to: its repository root when the workspace
    /// holds several repositories, otherwise the workspace root.
    async fn project_root(&self, context: &ToolContext, file: &str) -> Result<String>;

    /// Read text files by absolute path. A missing or unreadable file is
    /// `None`; the result has one entry per request, in order.
    async fn read_texts(
        &self,
        context: &ToolContext,
        paths: &[String],
    ) -> Result<Vec<Option<String>>>;

    /// Apply changes in order. Every path must already be inside `root`.
    async fn apply(&self, context: &ToolContext, root: &str, ops: &[FileOp]) -> Result<()>;

    /// Start a server in `root`. `key` names it, so starting the same key
    /// again replaces the earlier process.
    async fn launch(
        &self,
        context: &ToolContext,
        key: &str,
        command: &ServerCommand,
        root: &str,
    ) -> Result<ServerIo>;
}

/// What to tell the model when a language server is not installed.
pub(crate) fn missing_server_message(command: &str) -> String {
    format!(
        "language server `{command}` is not installed or not on PATH where the code lives; \
         install it there, or point the `lsp` capability at another binary with \
         `servers.<key>.command` in its configuration"
    )
}

/// Resolve `.` and `..` without touching the filesystem. Errors when `..`
/// would climb above the root of an absolute path.
pub(crate) fn normalize_lexically(path: &Path) -> Result<PathBuf> {
    use std::path::Component;
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => out.push(component),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    bail!("path escapes the filesystem root: {}", path.display());
                }
            }
            Component::Normal(part) => out.push(part),
        }
    }
    Ok(out)
}

/// Resolve a model-supplied scope inside a local `root` (already canonical).
///
/// Relative paths resolve from `root`. Real absolute paths are accepted only
/// when they resolve inside it. Symlinks that lead out are refused because the
/// result is canonicalized before the containment check.
pub(crate) fn resolve_scope(root: &Path, path: Option<&str>) -> Result<PathBuf> {
    use anyhow::Context;
    use std::path::Component;
    let Some(path) = path else {
        return Ok(root.to_path_buf());
    };
    let trimmed = path.trim();
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        let canonical = candidate
            .canonicalize()
            .with_context(|| format!("path not found: {path}"))?;
        if canonical.starts_with(root) {
            return Ok(canonical);
        }
        bail!("`path` must stay inside the workspace");
    }
    if trimmed.is_empty() || trimmed == "." {
        return Ok(root.to_path_buf());
    }
    if candidate.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!("`path` must stay inside the workspace");
    }
    let canonical = root
        .join(candidate)
        .canonicalize()
        .with_context(|| format!("path not found: {path}"))?;
    if !canonical.starts_with(root) {
        bail!("`path` must stay inside the workspace");
    }
    Ok(canonical)
}

/// Whether any component of `relative` is a directory the tools never touch.
pub(crate) fn protected_component(relative: &Path) -> Option<String> {
    relative.components().find_map(|component| match component {
        std::path::Component::Normal(name) => name
            .to_str()
            .filter(|name| SKIP_DIRS.contains(name))
            .map(str::to_string),
        _ => None,
    })
}

/// Map a model-supplied path to a relative path beneath a workspace whose
/// model-facing name is `display_root`. Accepts `display_root`-anchored
/// absolute paths and plain relative ones; refuses anything that climbs out.
pub(crate) fn workspace_relative(display_root: &str, path: &str) -> Result<String> {
    let trimmed = path.trim();
    let display_root = display_root.trim_end_matches('/');
    let relative = if trimmed == display_root {
        ""
    } else if let Some(rest) = trimmed.strip_prefix(&format!("{display_root}/")) {
        rest
    } else if trimmed.starts_with('/') {
        bail!("`path` must stay inside the workspace ({display_root})");
    } else {
        trimmed
    };
    let mut parts = Vec::new();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => bail!("`path` must stay inside the workspace"),
            other => parts.push(other),
        }
    }
    Ok(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_relative_accepts_anchored_and_relative_paths() {
        assert_eq!(workspace_relative("/workspace", "/workspace").unwrap(), "");
        assert_eq!(
            workspace_relative("/workspace", "/workspace/app/src/lib.rs").unwrap(),
            "app/src/lib.rs"
        );
        assert_eq!(
            workspace_relative("/workspace", "./app//src/").unwrap(),
            "app/src"
        );
    }

    #[test]
    fn workspace_relative_refuses_paths_that_leave() {
        assert!(workspace_relative("/workspace", "/etc/passwd").is_err());
        assert!(workspace_relative("/workspace", "../other").is_err());
        assert!(workspace_relative("/workspace", "app/../../other").is_err());
        assert!(workspace_relative("/workspace", "/workspace-other/x").is_err());
    }

    #[test]
    fn normalize_lexically_folds_dots() {
        assert_eq!(
            normalize_lexically(Path::new("/a/b/../c/./d")).unwrap(),
            PathBuf::from("/a/c/d")
        );
        assert!(normalize_lexically(Path::new("/../x")).is_err());
    }

    #[test]
    fn protected_component_finds_skipped_directories() {
        assert_eq!(
            protected_component(Path::new("app/node_modules/x.js")).as_deref(),
            Some("node_modules")
        );
        assert_eq!(protected_component(Path::new("app/src/x.js")), None);
    }
}
