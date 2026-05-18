// Composable SessionFileSystem decorators for policy enforcement.
//
// EVE-478 plans to ship these in everruns-runtime; until then they live here
// alongside the embedder that needs them. They wrap any `SessionFileSystem`
// (today: the runtime's `RealDiskFileStore`) and layer two concerns:
//
//   * WriteBlocklistFileStore — reject writes/deletes inside vendored or
//     build directories (`.git/`, `node_modules/`, `target/`, …) at any
//     depth. Reads pass through.
//   * ApprovalGatingFileStore — gate writes/deletes through the existing
//     ApprovalGate (TUI prompt or auto-approve). Reads pass through. The
//     gate is async and may sit on a oneshot from the UI thread.
//
// Compose: ApprovalGating::new(WriteBlocklist::new(RealDiskFileStore::new(root)))
// Reads short-circuit through both layers.

use crate::approval::{ApprovalGate, ApprovalRequest};
use async_trait::async_trait;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, InitialFile, SessionFile};
use everruns_core::traits::SessionFileSystem;
use everruns_core::typed_id::SessionId;
use std::path::Component;
use std::sync::Arc;

const DEFAULT_WRITE_BLOCKLIST: &[&str] = &[
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

/// Reject writes into vendored / build directories.
//
// Non-generic over the wrapped store: we store an `Arc<dyn SessionFileSystem>`
// rather than a generic `Arc<S>`. Two reasons:
//   1. Trait objects sidestep coherence questions for decorator stacks.
//   2. The only concrete value of `S` that ever flows through is the runtime's
//      `RealDiskFileStore`; monomorphization wasn't earning anything.
pub struct WriteBlocklistFileStore {
    inner: Arc<dyn SessionFileSystem>,
    blocklist: Vec<String>,
}

impl WriteBlocklistFileStore {
    pub fn new(inner: Arc<dyn SessionFileSystem>) -> Self {
        Self {
            inner,
            blocklist: DEFAULT_WRITE_BLOCKLIST
                .iter()
                .map(|s| s.to_string())
                .collect(),
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

/// Gate destructive ops through the approval channel. Reads pass through.
///
/// For writes we always read the existing content first (via the inner store)
/// so the approval prompt can show a diff. That's one extra read per write,
/// acceptable for a coding agent where writes are rare and small.
pub struct ApprovalGatingFileStore {
    inner: Arc<dyn SessionFileSystem>,
    gate: Arc<ApprovalGate>,
}

impl ApprovalGatingFileStore {
    pub fn new(inner: Arc<dyn SessionFileSystem>, gate: Arc<ApprovalGate>) -> Self {
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
        let approved = self
            .gate
            .approve(ApprovalRequest::FileWrite {
                path: path.to_string(),
                before,
                after: content.to_string(),
            })
            .await;
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
        // Reuse the default trait impl path: read existing, compare, then
        // call self.write_file (which gates). One approval per actual write.
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
        // Approval prompt sees the diff before -> after.
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
        let approved = self
            .gate
            .approve(ApprovalRequest::FileDelete {
                path: path.to_string(),
                recursive,
            })
            .await;
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
        // create_directory is a precursor to writes; we don't prompt for it
        // separately — the subsequent write_file in that directory triggers
        // the approval flow.
        self.inner.create_directory(session_id, path).await
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        // Initial files are embedder-supplied seed data — not LLM-driven —
        // so no approval prompt.
        self.inner.seed_initial_file(session_id, file).await
    }
}
