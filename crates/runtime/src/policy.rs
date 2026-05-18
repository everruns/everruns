// Policy decorators for `SessionFileStore`.
//
// These wrappers add cross-cutting concerns (blocklists, human approval)
// without rewriting the storage layer. They compose:
//
//     ApprovalGatingFileStore::new(
//         WriteBlocklistFileStore::default_blocked(RealDiskFileStore::new(root)?),
//         approval_cb,
//     )
//
// See `specs/file-store.md` for the contract and the "smell" note explaining
// why approval lives at the storage layer rather than the tool layer.

use async_trait::async_trait;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, InitialFile, SessionFile};
use everruns_core::traits::SessionFileStore;
use everruns_core::typed_id::SessionId;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::backends::RuntimeFileStore;

const DEFAULT_BLOCKED: &[&str] = &[
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

/// A `SessionFileStore` wrapper that rejects writes inside any directory
/// segment named in the block list. Reads pass through unchanged.
///
/// Matching is path-segment based: `/foo/.git/HEAD` is blocked because
/// `.git` appears anywhere in the path components.
#[derive(Clone)]
pub struct WriteBlocklistFileStore<S> {
    inner: S,
    blocked: Arc<Vec<String>>,
}

impl<S> WriteBlocklistFileStore<S> {
    /// Create a new wrapper with a custom block list.
    pub fn with_blocked(inner: S, blocked: impl IntoIterator<Item = String>) -> Self {
        Self {
            inner,
            blocked: Arc::new(blocked.into_iter().collect()),
        }
    }

    /// Create a wrapper with the canonical default block list
    /// (`.git`, `node_modules`, `target`, `dist`, `build`, `.next`, `.venv`,
    /// `venv`, `.tox`, `.gradle`).
    pub fn default_blocked(inner: S) -> Self {
        Self::with_blocked(inner, DEFAULT_BLOCKED.iter().map(|s| (*s).to_string()))
    }

    /// Borrow the inner store.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    fn check_path(&self, path: &str) -> Result<()> {
        if path_contains_blocked_segment(path, &self.blocked) {
            return Err(AgentLoopError::tool(format!(
                "write blocked by policy (path contains blocked segment): {path}"
            )));
        }
        Ok(())
    }
}

fn path_contains_blocked_segment(path: &str, blocked: &[String]) -> bool {
    path.split('/')
        .filter(|seg| !seg.is_empty())
        .any(|seg| blocked.iter().any(|b| b == seg))
}

#[async_trait]
impl<S> SessionFileStore for WriteBlocklistFileStore<S>
where
    S: SessionFileStore + Send + Sync,
{
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
        self.check_path(path)?;
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
        self.check_path(path)?;
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
        self.check_path(path)?;
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
        self.check_path(path)?;
        self.inner.create_directory(session_id, path).await
    }
}

#[async_trait]
impl<S> RuntimeFileStore for WriteBlocklistFileStore<S>
where
    S: RuntimeFileStore + Send + Sync,
{
    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        // Seeding bypasses the blocklist: starter files declared by the
        // harness/agent are trusted and not subject to the agent-write
        // policy. They never come from user input at runtime.
        self.inner.seed_initial_file(session_id, file).await
    }
}

/// Describes a write/delete/CAS/create-directory request handed to an
/// `ApprovalCallback`.
#[derive(Debug, Clone)]
pub struct WriteRequest {
    pub session_id: SessionId,
    pub path: String,
    pub kind: WriteKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteKind {
    Write,
    Delete,
    CreateDirectory,
    Cas,
}

/// Async callback signature for `ApprovalGatingFileStore`. Return `true` to
/// allow the operation, `false` to reject it.
pub type ApprovalCallback =
    Arc<dyn Fn(WriteRequest) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// A `SessionFileStore` wrapper that gates writes, deletes, CAS, and
/// `create_directory` through an async approval callback. Reads pass through.
///
/// The callback returns `bool`. `true` means "allowed, perform the
/// operation"; `false` means "rejected, return a `tool` error to the caller".
#[derive(Clone)]
pub struct ApprovalGatingFileStore<S> {
    inner: S,
    approve: ApprovalCallback,
}

impl<S> ApprovalGatingFileStore<S> {
    /// Construct a new wrapper. Use [`auto_approve_callback`] as the
    /// callback for tests or trusted callers.
    pub fn new(inner: S, approve: ApprovalCallback) -> Self {
        Self { inner, approve }
    }

    /// Borrow the inner store.
    pub fn inner(&self) -> &S {
        &self.inner
    }

    async fn check(&self, request: WriteRequest) -> Result<()> {
        let approved = (self.approve)(request.clone()).await;
        if !approved {
            return Err(AgentLoopError::tool(format!(
                "write denied by approval policy: {} on {}",
                describe_kind(request.kind),
                request.path
            )));
        }
        Ok(())
    }
}

fn describe_kind(kind: WriteKind) -> &'static str {
    match kind {
        WriteKind::Write => "write",
        WriteKind::Delete => "delete",
        WriteKind::CreateDirectory => "create_directory",
        WriteKind::Cas => "compare_and_set",
    }
}

/// A trivial callback that approves every write. Useful for tests, for the
/// default `ApprovalGatingFileStore` constructor in embedders that gate only
/// some operations, and as documentation of the callback shape.
pub fn auto_approve_callback() -> ApprovalCallback {
    Arc::new(|_req| Box::pin(async { true }))
}

#[async_trait]
impl<S> SessionFileStore for ApprovalGatingFileStore<S>
where
    S: SessionFileStore + Send + Sync,
{
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
        self.check(WriteRequest {
            session_id,
            path: path.to_string(),
            kind: WriteKind::Write,
        })
        .await?;
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
        self.check(WriteRequest {
            session_id,
            path: path.to_string(),
            kind: WriteKind::Cas,
        })
        .await?;
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
        self.check(WriteRequest {
            session_id,
            path: path.to_string(),
            kind: WriteKind::Delete,
        })
        .await?;
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
        self.check(WriteRequest {
            session_id,
            path: path.to_string(),
            kind: WriteKind::CreateDirectory,
        })
        .await?;
        self.inner.create_directory(session_id, path).await
    }
}

#[async_trait]
impl<S> RuntimeFileStore for ApprovalGatingFileStore<S>
where
    S: RuntimeFileStore + Send + Sync,
{
    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        // Same rationale as `WriteBlocklistFileStore::seed_initial_file`:
        // starter files are declared by harness/agent config, not produced
        // by agent decisions at runtime. Bypass the approval gate.
        self.inner.seed_initial_file(session_id, file).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::real_disk::RealDiskFileStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    fn make_disk() -> (RealDiskFileStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = RealDiskFileStore::new(dir.path()).unwrap();
        (store, dir)
    }

    fn sid() -> SessionId {
        SessionId::new()
    }

    #[tokio::test]
    async fn blocklist_rejects_writes_into_git_directory() {
        let (disk, _dir) = make_disk();
        let store = WriteBlocklistFileStore::default_blocked(disk);
        let session = sid();
        let err = store
            .write_file(session, "/.git/HEAD", "ref: ...", "text")
            .await
            .expect_err("must reject .git write");
        let msg = format!("{err}");
        assert!(msg.contains("blocked"), "got: {msg}");
    }

    #[tokio::test]
    async fn blocklist_rejects_nested_blocked_segment() {
        let (disk, _dir) = make_disk();
        let store = WriteBlocklistFileStore::default_blocked(disk);
        let session = sid();
        let err = store
            .write_file(session, "/subdir/node_modules/x.js", "1", "text")
            .await
            .expect_err("must reject nested node_modules");
        assert!(format!("{err}").contains("blocked"));
    }

    #[tokio::test]
    async fn blocklist_allows_writes_outside_blocked_segments() {
        let (disk, _dir) = make_disk();
        let store = WriteBlocklistFileStore::default_blocked(disk);
        let session = sid();
        store
            .write_file(session, "/src/main.rs", "fn main() {}", "text")
            .await
            .expect("non-blocked write succeeds");
        let read = store
            .read_file(session, "/src/main.rs")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.content.as_deref(), Some("fn main() {}"));
    }

    #[tokio::test]
    async fn blocklist_allows_reads_into_blocked_segments() {
        // Reads must always pass through — listing `.git/config` is OK even
        // if writes are not.
        let (disk, dir) = make_disk();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        let store = WriteBlocklistFileStore::default_blocked(disk);
        let session = sid();
        let read = store
            .read_file(session, "/.git/HEAD")
            .await
            .unwrap()
            .unwrap();
        assert!(read.content.as_deref().unwrap().contains("refs/heads/main"));
    }

    #[tokio::test]
    async fn approval_gate_blocks_when_callback_returns_false() {
        let (disk, _dir) = make_disk();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed_path = Arc::new(std::sync::Mutex::new(String::new()));
        let calls_cb = calls.clone();
        let path_cb = observed_path.clone();
        let cb: ApprovalCallback = Arc::new(move |req| {
            calls_cb.fetch_add(1, Ordering::SeqCst);
            *path_cb.lock().unwrap() = req.path.clone();
            Box::pin(async { false })
        });
        let store = ApprovalGatingFileStore::new(disk, cb);
        let session = sid();
        let err = store
            .write_file(session, "/src/main.rs", "fn main() {}", "text")
            .await
            .expect_err("must be denied");
        assert!(format!("{err}").contains("denied"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(*observed_path.lock().unwrap(), "/src/main.rs");
    }

    #[tokio::test]
    async fn approval_gate_allows_when_callback_returns_true() {
        let (disk, _dir) = make_disk();
        let store = ApprovalGatingFileStore::new(disk, auto_approve_callback());
        let session = sid();
        store
            .write_file(session, "/src/main.rs", "fn main() {}", "text")
            .await
            .expect("approved");
        let read = store
            .read_file(session, "/src/main.rs")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.content.as_deref(), Some("fn main() {}"));
    }

    #[tokio::test]
    async fn approval_gate_does_not_invoke_callback_on_read() {
        let (disk, _dir) = make_disk();
        let session = sid();
        // Seed a file directly through the disk store before wrapping.
        disk.write_file(session, "/x.txt", "hello", "text")
            .await
            .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_cb = calls.clone();
        let cb: ApprovalCallback = Arc::new(move |_req| {
            calls_cb.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { true })
        });
        let store = ApprovalGatingFileStore::new(disk, cb);
        let _ = store.read_file(session, "/x.txt").await.unwrap();
        let _ = store.list_directory(session, "/").await.unwrap();
        let _ = store.stat_file(session, "/x.txt").await.unwrap();
        let _ = store.grep_files(session, "x", None).await.unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "reads must not consult the approval callback"
        );
    }

    #[tokio::test]
    async fn approval_into_blocklist_into_disk_composes() {
        // Mirrors the ercode wiring: gate -> blocklist -> disk.
        let (disk, _dir) = make_disk();
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_cb = calls.clone();
        let cb: ApprovalCallback = Arc::new(move |_req| {
            calls_cb.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { true })
        });
        let stack =
            ApprovalGatingFileStore::new(WriteBlocklistFileStore::default_blocked(disk), cb);
        let session = sid();

        // Blocked path: rejected before approval callback runs because the
        // outer gate sees the request first, then the blocklist gate. Order
        // matters — in this stack the approval gate is outermost.
        let err = stack
            .write_file(session, "/.git/HEAD", "x", "text")
            .await
            .expect_err("blocklist must reject");
        assert!(format!("{err}").contains("blocked"));
        // Approval was consulted once (the outer layer), but the blocklist
        // rejected before disk I/O.
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Allowed path: flows all the way through.
        stack
            .write_file(session, "/src/lib.rs", "pub fn x() {}", "text")
            .await
            .expect("ok");
        let read = stack
            .read_file(session, "/src/lib.rs")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.content.as_deref(), Some("pub fn x() {}"));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
