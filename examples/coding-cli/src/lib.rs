//! A minimal coding agent built **only** on the public `everruns` library.
//!
//! This example is the executable acceptance test for the OSS library surface
//! (EVE-835): it depends on `everruns` alone — never on `everruns-core` or
//! `everruns-host` — and exercises the exact import path an application uses.
//! It builds an [`Agent`](everruns::Agent) with example-local file tools defined
//! via [`#[everruns::tool]`](everruns::tool), opens a [`Session`], streams the
//! session's events, and runs one or more prompts.
//!
//! The heavy TUI/MCP variant this replaced needed host internals the public
//! facade intentionally does not expose. Keeping the example inside the facade
//! keeps it honest about what a library user can actually build today.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use everruns::{Agent, AgentBuilder};

/// System prompt describing the agent and the tools it can call.
pub const SYSTEM_PROMPT: &str = "\
You are a terminal coding assistant operating inside a workspace directory. \
Use the `read_file`, `write_file`, and `list_dir` tools to inspect and edit \
files relative to the workspace root. Prefer small, verifiable changes and \
explain what you did.";

/// The workspace root the file tools resolve paths against.
///
/// Set once at startup with [`set_workspace`]; defaults to the process working
/// directory when unset (e.g. in tests that call the tool impls directly).
static WORKSPACE: OnceLock<PathBuf> = OnceLock::new();

/// Point the file tools at `root`. The first call wins; later calls are ignored.
pub fn set_workspace(root: PathBuf) {
    let _ = WORKSPACE.set(root);
}

fn workspace() -> PathBuf {
    WORKSPACE
        .get()
        .cloned()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Resolve a caller-supplied relative path against the workspace root, rejecting
/// absolute paths and any `..` traversal so a tool call cannot escape the
/// workspace.
fn safe_path(rel: &str) -> Result<PathBuf, String> {
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(format!("path must be relative to the workspace: {rel:?}"));
    }
    if path
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(format!("path must not contain `..`: {rel:?}"));
    }
    Ok(workspace().join(path))
}

// --- Tool implementations (plain, directly testable) --------------------
//
// The `#[everruns::tool]` attribute replaces the annotated function with a
// zero-argument constructor, so the real logic lives in these `_impl` helpers
// that the tools call and the tests exercise directly.

async fn read_file_impl(path: &str) -> Result<String, String> {
    let resolved = safe_path(path)?;
    tokio::fs::read_to_string(&resolved)
        .await
        .map_err(|e| format!("read {path:?}: {e}"))
}

async fn write_file_impl(path: &str, contents: &str) -> Result<String, String> {
    let resolved = safe_path(path)?;
    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    tokio::fs::write(&resolved, contents.as_bytes())
        .await
        .map_err(|e| format!("write {path:?}: {e}"))?;
    Ok(format!("wrote {} bytes to {path}", contents.len()))
}

async fn list_dir_impl(path: &str) -> Result<Vec<String>, String> {
    let resolved = safe_path(path)?;
    let mut reader = tokio::fs::read_dir(&resolved)
        .await
        .map_err(|e| format!("list {path:?}: {e}"))?;
    let mut entries = Vec::new();
    while let Some(entry) = reader
        .next_entry()
        .await
        .map_err(|e| format!("list {path:?}: {e}"))?
    {
        entries.push(entry.file_name().to_string_lossy().into_owned());
    }
    entries.sort();
    Ok(entries)
}

// --- Tools (public library surface via the attribute macro) -------------

/// Read a UTF-8 text file, relative to the workspace root.
#[everruns::tool]
async fn read_file(path: String) -> Result<String, String> {
    read_file_impl(&path).await
}

/// Create or replace a UTF-8 text file, relative to the workspace root.
#[everruns::tool]
async fn write_file(path: String, contents: String) -> Result<String, String> {
    write_file_impl(&path, &contents).await
}

/// List the entries of a directory relative to the workspace root (defaults to
/// the root itself).
#[everruns::tool]
async fn list_dir(path: Option<String>) -> Result<serde_json::Value, String> {
    let rel = path.unwrap_or_else(|| ".".to_string());
    let entries = list_dir_impl(&rel).await?;
    Ok(serde_json::json!({ "path": rel, "entries": entries }))
}

/// Start a coding [`Agent`] builder wired with the file tools.
pub fn agent_builder() -> AgentBuilder {
    Agent::builder()
        .instructions(SYSTEM_PROMPT)
        .tool(read_file())
        .tool(write_file())
        .tool(list_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn write_then_read_round_trips() {
        let dir = tempdir().unwrap();
        set_workspace(dir.path().to_path_buf());

        let msg = write_file_impl("notes/hello.txt", "hi there")
            .await
            .unwrap();
        assert!(msg.contains("wrote"), "{msg}");
        let back = read_file_impl("notes/hello.txt").await.unwrap();
        assert_eq!(back, "hi there");

        let entries = list_dir_impl(".").await.unwrap();
        assert!(entries.contains(&"notes".to_string()), "{entries:?}");
    }

    #[tokio::test]
    async fn rejects_traversal_and_absolute_paths() {
        assert!(read_file_impl("../secret").await.is_err());
        assert!(read_file_impl("/etc/passwd").await.is_err());
    }
}
