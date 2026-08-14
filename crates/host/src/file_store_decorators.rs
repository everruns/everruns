// Composable host `SessionFileSystem` decorators for policy enforcement.
//
// EVE-478: promoted from `examples/coding-cli` so any non-server embedder can
// compose them on top of `RealDiskFileStore`. Three concerns are layered here:
//
//   * `PolicyFileStore` — apply portable workspace read/write policy to any
//     provider selected by the runtime.
//   * `WriteBlocklistFileStore` — reject writes/deletes inside vendored or
//     build directories (`.git/`, `node_modules/`, `target/`, …) at any depth.
//     Reads pass through.
//   * `ApprovalGatingFileStore` — gate writes/deletes through an
//     embedder-supplied async `FileApprovalGate`. Reads pass through. The
//     embedder owns the UI / oneshot wiring; this crate only cares about the
//     yes/no answer.

use async_trait::async_trait;
use everruns_core::WorkspacePolicy;
use everruns_core::session_file::{
    FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, InitialFile, SessionFile,
    build_grep_search_result,
};
use everruns_core::session_files::SessionFileSystem;
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::SessionId;
use std::collections::{HashSet, VecDeque};
use std::path::Component;
use std::sync::Arc;

const MAX_POLICY_WALK_ENTRIES: usize = 100_000;

/// Default vendored / build directory names that `WriteBlocklistFileStore`
/// rejects writes into. Embedders can override via `with_blocklist`.
const LEGACY_WRITE_BLOCKLIST: &[&str] = &[
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

/// Legacy default for [`WriteBlocklistFileStore`].
///
/// New applications should configure [`WorkspacePolicy`] instead. This alias
/// remains only for 0.17 source compatibility; policy defaults are owned by the
/// policy value and may evolve without a permanent public list.
#[deprecated(since = "0.17.25", note = "use WorkspacePolicy instead")]
pub const DEFAULT_WRITE_BLOCKLIST: &[&str] = LEGACY_WRITE_BLOCKLIST;

/// Apply a backend-independent [`WorkspacePolicy`] to a session filesystem.
///
/// Every model-driven read, listing, search, write, create, and delete flows
/// through the same policy regardless of the concrete provider. Starter-file
/// seeding bypasses the policy because it is trusted application configuration;
/// later model access to a seeded path is still checked normally. Custom
/// providers must honor the canonical-path contract of
/// [`SessionFileSystem::resolve_path`].
pub struct PolicyFileStore {
    inner: Arc<dyn SessionFileSystem>,
    policy: WorkspacePolicy,
}

impl PolicyFileStore {
    /// Wrap `inner` with `policy`.
    pub fn new(inner: Arc<dyn SessionFileSystem>, policy: WorkspacePolicy) -> Self {
        Self { inner, policy }
    }

    fn checked_path(&self, path: &str) -> Result<String> {
        WorkspacePolicy::validate_path(path)
            .map(|()| self.inner.resolve_path(path))
            .map_err(|error| AgentLoopError::tool(error.to_string()))
    }

    fn check_read(&self, path: &str) -> Result<()> {
        // THREAT[TM-FS-015]: every read spelling is normalized and checked at
        // the shared filesystem seam before a provider sees it.
        self.checked_path(path).and_then(|path| {
            self.policy
                .check_read(&path)
                .map_err(|error| AgentLoopError::tool(error.to_string()))
        })
    }

    fn check_write(&self, path: &str) -> Result<()> {
        // THREAT[TM-FS-015]: deny precedence and protected-path defaults apply
        // uniformly to every provider and every mutating capability.
        self.checked_path(path).and_then(|path| {
            self.policy
                .check_write(&path)
                .map_err(|error| AgentLoopError::tool(error.to_string()))
        })
    }

    async fn check_recursive_delete(&self, session_id: SessionId, path: &str) -> Result<()> {
        if !self.policy.permits_recursive_delete() {
            return Err(AgentLoopError::tool(format!(
                "workspace policy denied recursive delete of `{path}`; recursive deletion requires an explicit opt-in"
            )));
        }

        let Some(target) = self.inner.stat_file(session_id, path).await? else {
            return Ok(());
        };
        if !target.is_directory {
            return Ok(());
        }

        // THREAT[TM-FS-015]: an opt-in to recursive deletion does not override
        // denied descendants. Inspect through the inner provider so protected
        // entries hidden from model-facing listings still block the delete.
        // A same-user filesystem mutation can still race this preflight; host
        // embedders that need a stronger boundary must add OS isolation.
        let mut pending = VecDeque::from([target.path]);
        let mut visited = HashSet::new();
        let mut entries_seen = 0usize;
        while let Some(directory) = pending.pop_front() {
            if !visited.insert(directory.clone()) {
                continue;
            }
            for entry in self.inner.list_directory(session_id, &directory).await? {
                entries_seen += 1;
                if entries_seen > MAX_POLICY_WALK_ENTRIES {
                    return Err(AgentLoopError::tool(format!(
                        "workspace policy stopped recursive delete after {MAX_POLICY_WALK_ENTRIES} entries"
                    )));
                }
                self.check_write(&entry.path)?;
                if entry.is_directory {
                    pending.push_back(entry.path);
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl SessionFileSystem for PolicyFileStore {
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

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        self.check_read(path)?;
        self.inner.read_file(session_id, path).await
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        self.check_write(path)?;
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
        self.check_write(path)?;
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
        self.check_write(path)?;
        if recursive {
            self.check_recursive_delete(session_id, path).await?;
        }
        self.inner.delete_file(session_id, path, recursive).await
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let canonical = self.checked_path(path)?;
        if !self.policy.permits_read_traversal(&canonical) {
            self.policy
                .check_read(&canonical)
                .map_err(|error| AgentLoopError::tool(error.to_string()))?;
        }
        let mut entries = self.inner.list_directory(session_id, path).await?;
        entries.retain(|entry| {
            self.checked_path(&entry.path).is_ok_and(|canonical| {
                self.policy.permits_read(&canonical)
                    || self.policy.permits_read_traversal(&canonical)
            })
        });
        Ok(entries)
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        let canonical = self.checked_path(path)?;
        if !self.policy.permits_read_traversal(&canonical) {
            self.policy
                .check_read(&canonical)
                .map_err(|error| AgentLoopError::tool(error.to_string()))?;
        }
        self.inner.stat_file(session_id, path).await
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let result = self
            .grep_files_with_options(
                session_id,
                pattern,
                &GrepOptions {
                    path_pattern: path_pattern.map(ToString::to_string),
                    ..GrepOptions::default()
                },
            )
            .await?;
        Ok(result.matches)
    }

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        let regex = crate::grep_limits::build_regex(pattern)?;
        crate::grep_limits::validate_path_pattern(options.path_pattern.as_deref())?;
        let path_pattern = options
            .path_pattern
            .as_deref()
            .map(everruns_core::session_path::GrepPathPattern::new)
            .transpose()?;

        // Walk through the policy-filtered listing surface and read only files
        // the policy permits. Calling the provider's grep directly and
        // filtering its output would still make it open denied files.
        let mut pending = VecDeque::from(["/".to_string()]);
        let mut visited = HashSet::new();
        let mut text_files = Vec::new();
        let mut total_scanned = 0usize;
        let mut entries_seen = 0usize;
        while let Some(directory) = pending.pop_front() {
            if !visited.insert(directory.clone()) {
                continue;
            }
            for entry in self.list_directory(session_id, &directory).await? {
                entries_seen += 1;
                if entries_seen > MAX_POLICY_WALK_ENTRIES {
                    return Err(AgentLoopError::tool(format!(
                        "workspace policy stopped grep after {MAX_POLICY_WALK_ENTRIES} entries"
                    )));
                }
                if entry.is_directory {
                    pending.push_back(entry.path);
                    continue;
                }
                if path_pattern
                    .as_ref()
                    .is_some_and(|matcher| !matcher.is_match(&entry.path))
                {
                    continue;
                }
                let Some(file) = self.read_file(session_id, &entry.path).await? else {
                    continue;
                };
                if file.encoding != "text" {
                    continue;
                }
                let Some(content) = file.content else {
                    continue;
                };
                if crate::grep_limits::account_scan(&mut total_scanned, content.len())? {
                    text_files.push((entry.path, content));
                }
            }
        }
        Ok(build_grep_search_result(text_files, &regex, options))
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        self.check_write(path)?;
        self.inner.create_directory(session_id, path).await
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        self.inner.seed_initial_file(session_id, file).await
    }
}

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
    /// Wrap `inner` with the compatibility blocklist.
    pub fn new(inner: Arc<dyn SessionFileSystem>) -> Self {
        Self {
            inner,
            blocklist: LEGACY_WRITE_BLOCKLIST
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
                        "writes into `{s}/` are blocked; write blocklist rejected `{path}`"
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

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        self.inner
            .grep_files_with_options(session_id, pattern, options)
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

    fn is_mount_resolver(&self) -> bool {
        self.inner.is_mount_resolver()
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

    /// Internal helper: gate a write given an already-known `before` content,
    /// then write through the inner store.
    ///
    /// Used by [`Self::write_file`] for the unconditional-write path. The CAS
    /// path ([`Self::write_file_if_content_matches`]) re-checks via the inner
    /// store's CAS write after approval so it cannot use this helper.
    async fn gated_write_with_before(
        &self,
        session_id: SessionId,
        path: &str,
        before: Option<String>,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
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
        // Propagate inner read errors instead of silently treating them as
        // "no prior content" — a permission error or transient I/O fault
        // should surface, not be hidden behind the approval prompt.
        let before = self
            .inner
            .read_file(session_id, path)
            .await?
            .and_then(|f| f.content);
        self.gated_write_with_before(session_id, path, before, content, encoding)
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
        // Read existing, compare, then gate using the already-fetched content
        // for the approval `before`. Avoids a second `read_file` on the
        // successful-write path.
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
        let approved = self.gate.approve_write(path, Some(current), content).await;
        if !approved {
            return Err(AgentLoopError::tool(format!(
                "user denied write to `{path}`"
            )));
        }

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

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        self.inner
            .grep_files_with_options(session_id, pattern, options)
            .await
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        self.inner.create_directory(session_id, path).await
    }

    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        self.inner.seed_initial_file(session_id, file).await
    }

    fn is_mount_resolver(&self) -> bool {
        self.inner.is_mount_resolver()
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
    async fn write_blocklist_contextual_grep_passes_through() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(
                sid(),
                "/output.log",
                "before\nError: failed\nROOT_CAUSE=duplicate_sentinel\nafter\n",
                "text",
            )
            .await
            .unwrap();
        let store = WriteBlocklistFileStore::new(inner_store);

        let result = store
            .grep_files_with_options(
                sid(),
                "Error|failed",
                &GrepOptions {
                    before_context: 1,
                    after_context: 1,
                    ..GrepOptions::default()
                },
            )
            .await
            .expect("contextual grep must pass through the decorator");

        assert_eq!(result.returned_matches, 1);
        assert_eq!(result.blocks.len(), 1);
        assert_eq!(result.blocks[0].start_line, 1);
        assert_eq!(result.blocks[0].end_line, 3);
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

    #[tokio::test]
    async fn workspace_policy_filters_reads_listings_and_grep_summaries() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        for (path, content) in [
            ("/src/lib.rs", "visible sentinel"),
            ("/.env", "hidden sentinel"),
            ("/private/secret.txt", "private sentinel"),
        ] {
            inner_store
                .write_file(sid(), path, content, "text")
                .await
                .unwrap();
        }
        let policy = WorkspacePolicy::builder()
            .allow_read("/")
            .deny_read("private")
            .build()
            .unwrap();
        let store = PolicyFileStore::new(inner_store, policy);

        assert!(store.read_file(sid(), "/src/lib.rs").await.is_ok());
        assert!(store.read_file(sid(), "/.env").await.is_err());
        assert!(store.read_file(sid(), "/private/secret.txt").await.is_err());

        let listed = store.list_directory(sid(), "/").await.unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|entry| entry.path.as_str())
                .collect::<Vec<_>>(),
            vec!["/src"]
        );

        let result = store
            .grep_files_with_options(sid(), "sentinel", &GrepOptions::default())
            .await
            .unwrap();
        assert_eq!(result.returned_matches, 1);
        assert_eq!(result.total_matches, 1);
        assert_eq!(result.matches[0].path, "/src/lib.rs");
    }

    #[tokio::test]
    async fn workspace_policy_enforces_write_scope_deny_precedence_and_recursive_opt_in() {
        let policy = WorkspacePolicy::builder()
            .allow_read("/")
            .allow_write("output")
            .deny_write("output/locked")
            .allow_recursive_delete(false)
            .build()
            .unwrap();
        let store = PolicyFileStore::new(inner(), policy);

        store
            .write_file(sid(), "/output/report.txt", "ok", "text")
            .await
            .unwrap();
        assert!(
            store
                .write_file(sid(), "/output/locked/report.txt", "no", "text")
                .await
                .is_err()
        );
        assert!(
            store
                .write_file(sid(), "/src/lib.rs", "no", "text")
                .await
                .is_err()
        );
        assert!(store.delete_file(sid(), "/output", true).await.is_err());
    }

    #[tokio::test]
    async fn recursive_delete_opt_in_does_not_override_a_denied_descendant() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(sid(), "/output/report.txt", "ok", "text")
            .await
            .unwrap();
        inner_store
            .write_file(sid(), "/output/locked/secret.txt", "no", "text")
            .await
            .unwrap();
        let policy = WorkspacePolicy::builder()
            .allow_write("output")
            .deny_write("output/locked")
            .allow_recursive_delete(true)
            .build()
            .unwrap();
        let store = PolicyFileStore::new(inner_store.clone(), policy);

        let error = store.delete_file(sid(), "/output", true).await.unwrap_err();
        assert!(error.to_string().contains("/workspace/output/locked"));
        assert!(
            inner_store
                .read_file(sid(), "/output/report.txt")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn trusted_seed_bypasses_policy_but_later_access_does_not() {
        let store = PolicyFileStore::new(inner(), WorkspacePolicy::default());
        store
            .seed_initial_file(
                sid(),
                &InitialFile {
                    path: "/.env".to_string(),
                    content: "TOKEN=secret".to_string(),
                    encoding: "text".to_string(),
                    is_readonly: true,
                },
            )
            .await
            .unwrap();
        assert!(store.read_file(sid(), "/.env").await.is_err());
    }

    #[tokio::test]
    async fn workspace_policy_rejects_traversal_before_backend_access() {
        let store = PolicyFileStore::new(inner(), WorkspacePolicy::read_write());
        let error = store
            .write_file(sid(), "/workspace/src/../../outside", "no", "text")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("traversal"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn host_symlink_swap_between_operations_is_rejected() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace.path().join("current")).unwrap();
        std::fs::write(outside.path().join("secret.txt"), "outside").unwrap();
        let inner: Arc<dyn SessionFileSystem> =
            Arc::new(crate::real_disk::RealDiskFileStore::new(workspace.path()).unwrap());
        let store = PolicyFileStore::new(inner, WorkspacePolicy::read_write());

        store
            .write_file(sid(), "/current/safe.txt", "inside", "text")
            .await
            .unwrap();
        std::fs::remove_dir_all(workspace.path().join("current")).unwrap();
        symlink(outside.path(), workspace.path().join("current")).unwrap();

        let error = store
            .read_file(sid(), "/current/secret.txt")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("symlink"));
    }

    #[tokio::test]
    async fn host_absolute_path_outside_root_cannot_expose_host_file() {
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), "outside").unwrap();
        let inner: Arc<dyn SessionFileSystem> =
            Arc::new(crate::real_disk::RealDiskFileStore::new(workspace.path()).unwrap());
        let store = PolicyFileStore::new(inner, WorkspacePolicy::read_write());

        match store
            .read_file(sid(), outside.path().to_str().unwrap())
            .await
        {
            Ok(None) | Err(_) => {}
            Ok(Some(file)) => assert_ne!(
                file.content.as_deref(),
                Some("outside"),
                "an absolute path outside the root must not expose that host file"
            ),
        }
    }

    #[tokio::test]
    async fn host_absolute_alias_cannot_bypass_canonical_deny_scope() {
        let workspace = tempfile::tempdir().unwrap();
        std::fs::create_dir(workspace.path().join("private")).unwrap();
        let secret = workspace.path().join("private/secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let inner: Arc<dyn SessionFileSystem> =
            Arc::new(crate::real_disk::RealDiskFileStore::new(workspace.path()).unwrap());
        let policy = WorkspacePolicy::builder()
            .allow_read("/")
            .deny_read("private")
            .build()
            .unwrap();
        let store = PolicyFileStore::new(inner, policy);

        // `RealDiskFileStore::display_root` exposes the canonical host root, so
        // use the same spelling a model could echo back from tool output.
        let canonical_secret = secret.canonicalize().unwrap();
        let error = store
            .read_file(sid(), canonical_secret.to_str().unwrap())
            .await
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("/workspace/private/secret.txt"),
            "unexpected policy diagnostic: {message}"
        );
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
    async fn approval_gating_contextual_grep_passes_through_without_prompt() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(
                sid(),
                "/output.log",
                "before\nError: failed\nROOT_CAUSE=duplicate_sentinel\nafter\n",
                "text",
            )
            .await
            .unwrap();
        let gate = Arc::new(RecordingGate::new(false));
        let store = ApprovalGatingFileStore::new(inner_store, gate.clone());

        let result = store
            .grep_files_with_options(
                sid(),
                "Error|failed",
                &GrepOptions {
                    before_context: 1,
                    after_context: 1,
                    ..GrepOptions::default()
                },
            )
            .await
            .expect("contextual grep must pass through the decorator");

        assert_eq!(result.returned_matches, 1);
        assert_eq!(result.blocks.len(), 1);
        assert!(gate.writes.lock().unwrap().is_empty());
        assert!(gate.deletes.lock().unwrap().is_empty());
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

    struct MutatingGate {
        inner: Arc<dyn SessionFileSystem>,
    }

    #[async_trait]
    impl FileApprovalGate for MutatingGate {
        async fn approve_write(&self, _path: &str, _before: Option<String>, _after: &str) -> bool {
            self.inner
                .write_file(sid(), "/notes.txt", "intruder", "text")
                .await
                .unwrap();
            true
        }

        async fn approve_delete(&self, _path: &str, _recursive: bool) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn write_if_content_matches_rechecks_after_approval() {
        let inner_store: Arc<dyn SessionFileSystem> = inner();
        inner_store
            .write_file(sid(), "/notes.txt", "original", "text")
            .await
            .unwrap();
        let gate = Arc::new(MutatingGate {
            inner: inner_store.clone(),
        });
        let store = ApprovalGatingFileStore::new(inner_store.clone(), gate);

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

        assert!(result.is_none());
        let final_file = inner_store
            .read_file(sid(), "/notes.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(final_file.content.as_deref(), Some("intruder"));
    }
}
