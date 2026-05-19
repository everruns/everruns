// Composable `SessionFileSystem` decorators for policy enforcement.
//
// EVE-478: promoted from `examples/coding-cli` so any non-server embedder can
// compose them on top of `RealDiskFileStore`. Two concerns are layered here:
//
//   * `WriteBlocklistFileStore` — reject writes/deletes inside vendored or
//     build directories (`.git/`, `node_modules/`, `target/`, …) at any depth.
//     Reads pass through.
//   * `ApprovalGatingFileStore` — gate writes/deletes through an
//     embedder-supplied async `FileApprovalGate`. Reads pass through. The
//     embedder owns the UI / oneshot wiring; this crate only cares about the
//     yes/no answer.
//
// Compose as `ApprovalGatingFileStore::new(WriteBlocklistFileStore::new(real_disk), gate)`.
// Reads short-circuit through both layers; only the destructive paths take
// the policy decisions.

use async_trait::async_trait;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, InitialFile, SessionFile};
use everruns_core::traits::SessionFileSystem;
use everruns_core::typed_id::SessionId;
use std::path::Component;
use std::sync::Arc;

/// Default vendored / build directory names that `WriteBlocklistFileStore`
/// rejects writes into. Embedders can override via `with_blocklist`.
pub const DEFAULT_WRITE_BLOCKLIST: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".venv",
    "venv",
    ".tox",
    ".gradle",
];

/// Reject writes into vendored / build directories at any depth.
///
/// Reads, listings, stats, and greps pass through; only mutating operations
/// (`write_file`, `delete_file`, `create_directory`, `seed_initial_file`,
/// `write_file_if_content_matches`) check the blocklist.
//
// Non-generic over the wrapped store: we hold `Arc<dyn SessionFileSystem>`
// rather than a generic `Arc<S>` so decorator stacks compose without
// coherence gymnastics. The runtime only ever wraps one concrete store
// (`RealDiskFileStore` today), so monomorphization wasn't earning anything.
pub struct WriteBlocklistFileStore {
    inner: Arc<dyn SessionFileSystem>,
    blocklist: Vec<String>,
}

impl WriteBlocklistFileStore {
    /// Wrap `inner` with the [`DEFAULT_WRITE_BLOCKLIST`].
    pub fn new(inner: Arc<dyn SessionFileSystem>) -> Self {
        Self {
            inner,
            blocklist: DEFAULT_WRITE_BLOCKLIST
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    /// Wrap `inner` with a custom blocklist (replaces the default entirely).
    pub fn with_blocklist(
        inner: Arc<dyn SessionFileSystem>,
        blocklist: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            inner,
            blocklist: blocklist.into_iter().map(Into::into).collect(),
        }
    }

    fn check(&self, path: &str) -> Result<()> {
        let p = std::path::Path::new(path);
        for comp in p.components() {
            if let Component::Normal(name) = comp {
                let s = name.to_string_lossy();
                if self.blocklist.iter().any(|b| b == s.as_ref()) {
                    return Err(AgentLoopError::tool(format!(
                        "writes into `{s}/` are blocked; the workspace write blocklist rejected `{path}`"
                    )));
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl SessionFileSystem for WriteBlocklistFileStore {
    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        self.inner.read_file(session_id, path).await
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        self.check(path)?;
        self.inner
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
        self.check(path)?;
        self.inner
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
        self.check(path)?;
        self.inner.delete_file(session_id, path, recursive).await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        self.inner.list_directory(session_id, path).await
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        self.inner.stat_file(session_id, path).await
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        self.inner
            .grep_files(session_id, pattern, path_pattern)
            .await
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        self.check(path)?;
        self.inner.create_directory(session_id, path).await
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        self.check(&file.path)?;
        self.inner.seed_initial_file(session_id, file).await
    }
}

/// Embedder-supplied approval callback used by [`ApprovalGatingFileStore`].
///
/// Implementations decide how to ask the user (TUI prompt, web confirmation,
/// auto-approve, OS notification, etc.). The store passes raw paths and the
/// before/after content for writes so the implementation can render diffs.
#[async_trait]
pub trait FileApprovalGate: Send + Sync {
    /// Decide whether the proposed write may proceed.
    ///
    /// `before` is the inner store's current content, if any — `None` if the
    /// file does not yet exist. `after` is the proposed new content.
    async fn approve_write(&self, path: &str, before: Option<String>, after: &str) -> bool;

    /// Decide whether the proposed delete may proceed.
    async fn approve_delete(&self, path: &str, recursive: bool) -> bool;
}

/// Gate destructive operations through an embedder-supplied
/// [`FileApprovalGate`]. Reads pass through.
///
/// For writes we always fetch the inner store's current content first so the
/// approval prompt can show a diff. That's one extra read per write —
/// acceptable for a coding agent where writes are rare and small. The
/// `create_directory` and `seed_initial_file` paths are not gated: the
/// subsequent `write_file` inside the created directory triggers the prompt,
/// and seed files are embedder-supplied (not LLM-driven).
pub struct ApprovalGatingFileStore {
    inner: Arc<dyn SessionFileSystem>,
    gate: Arc<dyn FileApprovalGate>,
}

impl ApprovalGatingFileStore {
    pub fn new(inner: Arc<dyn SessionFileSystem>, gate: Arc<dyn FileApprovalGate>) -> Self {
        Self { inner, gate }
    }
}

#[async_trait]
impl SessionFileSystem for ApprovalGatingFileStore {
    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        self.inner.read_file(session_id, path).await
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let before = self
            .inner
            .read_file(session_id, path)
            .await
            .ok()
            .flatten()
            .and_then(|f| f.content);
        let approved = self.gate.approve_write(path, before, content).await;
        if !approved {
            return Err(AgentLoopError::tool(format!(
                "user denied write to `{path}`"
            )));
        }
        self.inner
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
        // Read existing, compare, then delegate to `write_file` (which gates).
        // One approval per actual write.
        let Some(existing) = self.inner.read_file(session_id, path).await? else {
            return Ok(None);
        };
        if existing.is_directory {
            return Ok(None);
        }
        let current = existing.content.unwrap_or_default();
        if current != expected_content || existing.encoding != expected_encoding {
            return Ok(None);
        }
        self.write_file(session_id, path, content, encoding)
            .await
            .map(Some)
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        let approved = self.gate.approve_delete(path, recursive).await;
        if !approved {
            return Err(AgentLoopError::tool(format!(
                "user denied delete of `{path}`"
            )));
        }
        self.inner.delete_file(session_id, path, recursive).await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        self.inner.list_directory(session_id, path).await
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        self.inner.stat_file(session_id, path).await
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        self.inner
            .grep_files(session_id, pattern, path_pattern)
            .await
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        self.inner.create_directory(session_id, path).await
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        self.inner.seed_initial_file(session_id, file).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::in_memory::InMemorySessionFileStore;
    use std::sync::Mutex;

    fn sid() -> SessionId {
        "session_00000000000000000000000000000001".parse().unwrap()
    }

    fn inner() -> Arc<dyn SessionFileSystem> {
        Arc::new(InMemorySessionFileStore::new())
    }

    #[tokio::test]
    async fn write_blocklist_rejects_blocked_paths() {
        let store = WriteBlocklistFileStore::new(inner());
        let err = store
            .write_file(sid(), "/.git/config", "bad", "text")
            .await
            .expect_err("write into .git must be rejected");
        assert!(format!("{err}").contains(".git"));
    }

    #[tokio::test]
    async fn write_blocklist_allows_unblocked_paths() {
        let store = WriteBlocklistFileStore::new(inner());
        store
            .write_file(sid(), "/src/main.rs", "fn main() {}", "text")
            .await
            .expect("write outside blocklist must succeed");
    }

    #[tokio::test]
    async fn write_blocklist_reads_pass_through_blocked() {
        // Seed via the inner store directly, then verify read works through
        // the decorator even though the path is in the blocklist.
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(sid(), "/.git/config", "settings", "text")
            .await
            .unwrap();
        let store = WriteBlocklistFileStore::new(inner_store);
        let file = store
            .read_file(sid(), "/.git/config")
            .await
            .unwrap()
            .expect("read through blocklist must succeed");
        assert_eq!(file.content.as_deref(), Some("settings"));
    }

    #[tokio::test]
    async fn write_blocklist_custom_overrides_default() {
        let store = WriteBlocklistFileStore::with_blocklist(inner(), ["forbidden"]);
        // Default-blocked path is now allowed.
        store
            .write_file(sid(), "/.git/config", "ok", "text")
            .await
            .expect("custom blocklist replaces default");
        // Custom-blocked path is rejected.
        let err = store
            .write_file(sid(), "/forbidden/x", "no", "text")
            .await
            .expect_err("custom blocklist entry must be enforced");
        assert!(format!("{err}").contains("forbidden"));
    }

    struct RecordingGate {
        approve: bool,
        writes: Mutex<Vec<(String, Option<String>, String)>>,
        deletes: Mutex<Vec<(String, bool)>>,
    }

    impl RecordingGate {
        fn new(approve: bool) -> Self {
            Self {
                approve,
                writes: Mutex::new(Vec::new()),
                deletes: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl FileApprovalGate for RecordingGate {
        async fn approve_write(&self, path: &str, before: Option<String>, after: &str) -> bool {
            self.writes
                .lock()
                .unwrap()
                .push((path.to_string(), before, after.to_string()));
            self.approve
        }

        async fn approve_delete(&self, path: &str, recursive: bool) -> bool {
            self.deletes
                .lock()
                .unwrap()
                .push((path.to_string(), recursive));
            self.approve
        }
    }

    #[tokio::test]
    async fn approval_gating_denies_write_when_user_rejects() {
        let gate = Arc::new(RecordingGate::new(false));
        let store = ApprovalGatingFileStore::new(inner(), gate.clone());
        let err = store
            .write_file(sid(), "/notes.txt", "new", "text")
            .await
            .expect_err("rejected write must surface as tool error");
        assert!(format!("{err}").contains("denied"));
        assert_eq!(gate.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn approval_gating_approves_write_and_passes_before_after() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(sid(), "/notes.txt", "original", "text")
            .await
            .unwrap();
        let gate = Arc::new(RecordingGate::new(true));
        let store = ApprovalGatingFileStore::new(inner_store, gate.clone());
        let file = store
            .write_file(sid(), "/notes.txt", "updated", "text")
            .await
            .expect("approved write must succeed");
        assert_eq!(file.content.as_deref(), Some("updated"));
        let writes = gate.writes.lock().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, "/notes.txt");
        assert_eq!(writes[0].1.as_deref(), Some("original"));
        assert_eq!(writes[0].2, "updated");
    }

    #[tokio::test]
    async fn approval_gating_denies_delete_when_user_rejects() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(sid(), "/scratch.txt", "x", "text")
            .await
            .unwrap();
        let gate = Arc::new(RecordingGate::new(false));
        let store = ApprovalGatingFileStore::new(inner_store, gate);
        let err = store
            .delete_file(sid(), "/scratch.txt", false)
            .await
            .expect_err("rejected delete must surface as tool error");
        assert!(format!("{err}").contains("denied"));
    }

    #[tokio::test]
    async fn approval_gating_reads_pass_through_without_prompt() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(sid(), "/notes.txt", "hi", "text")
            .await
            .unwrap();
        let gate = Arc::new(RecordingGate::new(false));
        let store = ApprovalGatingFileStore::new(inner_store, gate.clone());
        let file = store.read_file(sid(), "/notes.txt").await.unwrap();
        assert_eq!(file.unwrap().content.as_deref(), Some("hi"));
        assert!(gate.writes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn write_if_content_matches_takes_one_approval_per_write() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(sid(), "/notes.txt", "original", "text")
            .await
            .unwrap();
        let gate = Arc::new(RecordingGate::new(true));
        let store = ApprovalGatingFileStore::new(inner_store, gate.clone());

        let result = store
            .write_file_if_content_matches(
                sid(),
                "/notes.txt",
                "original",
                "text",
                "updated",
                "text",
            )
            .await
            .unwrap();
        assert!(result.is_some());
        assert_eq!(gate.writes.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn write_if_content_matches_with_stale_expected_returns_none_without_prompt() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(sid(), "/notes.txt", "actual", "text")
            .await
            .unwrap();
        let gate = Arc::new(RecordingGate::new(true));
        let store = ApprovalGatingFileStore::new(inner_store, gate.clone());

        let result = store
            .write_file_if_content_matches(
                sid(),
                "/notes.txt",
                "stale-expected",
                "text",
                "new",
                "text",
            )
            .await
            .unwrap();
        assert!(result.is_none());
        assert!(gate.writes.lock().unwrap().is_empty());
    }
}
