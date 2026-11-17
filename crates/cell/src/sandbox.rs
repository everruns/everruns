//! Managed sandboxes for sessions on the data plane (data plane v2, step
//! 11.3).
//!
//! An agent's `session_sandbox` capability runs on an executor as it runs on
//! a v1 worker (the same tools and providers), with two differences:
//!
//! - The provider key is a sealed org secret
//!   ([`sandbox_key_secret`]) the control plane wrote when it pinned the
//!   session's sandbox. An executor opens it per call for the calling
//!   session's org, and never for a credential the session's pinned
//!   capability does not name (members cannot send a definition naming one:
//!   `gateway.rs`).
//! - The session's workspace in the cell stays the source of truth (`fs.md`
//!   "sync mode"): before a sandbox call the sandbox catches up with the
//!   workspace, after a mutating call the workspace takes what the sandbox
//!   changed ([`CellSandboxSync`]). A lost sandbox restores from the
//!   workspace, so there are no provider checkpoints.
//!
//! Decision: sync mode first, mount mode later (step 11 plan, decision 3A).
//! Sync costs one `exec` round trip after a mutating call and nothing before
//! a call when neither side changed; a mount would cost a round trip per
//! file access inside the sandbox.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use everruns_capabilities::session_sandbox::{
    SandboxWorkspaceSync, SandboxWorkspaceSyncExt, SessionSandboxConfig,
    SessionSandboxCredentialSource, SessionSandboxExecRequest, SessionSandboxProvider,
    SessionSandboxState, save_session_sandbox_state,
};
use everruns_contracts::dataplane::sandbox_key_secret;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::session_sandbox::{SessionSandboxContext, SessionSandboxCredential};
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::HostBackends;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::ToolExecutionResult;
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::Digest;

use crate::files::{Change, Content, Entry};
use crate::hosting::files::CellFiles;
use crate::providers::OrgMaterial;

/// Where a sandbox's record of its last sync lives, in its state's
/// metadata: the workspace version and the sandbox time it matched.
const RECORD: &str = "everruns_fs";
/// Directories neither side copies: tool caches and checkouts that belong
/// to the sandbox. Their files never reach the workspace, and the
/// workspace's never go into them.
const SKIPPED: &[&str] = &[".git", "node_modules", "target", ".venv", "__pycache__"];
/// Files larger than this stay in the sandbox.
const FILE_MAX: u64 = 16 * 1024 * 1024;
/// Changed files a scan lists inline; past this, the whole listing is read.
const CHANGED_MAX: usize = 500;
/// The scan's full listing of the workspace at `root`, inside the sandbox
/// (one per workspace, so sandboxes sharing a machine keep their own).
fn listing_path(root: &str) -> String {
    let digest = hex::encode(sha2::Sha256::digest(root.as_bytes()));
    format!("/tmp/.everruns-fs-{}", &digest[..16])
}
/// How long an opened provider key is kept.
const KEEP: Duration = Duration::from_secs(30);
/// Sessions whose last sync this process remembers.
const MARKS_MAX: usize = 50_000;

/// What this process last saw a session's sandbox hold.
#[derive(Clone)]
struct Mark {
    sandbox: String,
    version: u64,
    at: String,
    /// The sandbox's synced files (session paths).
    paths: Arc<BTreeSet<String>>,
}

/// The sandboxes of the sessions whose turns run here.
#[derive(Default)]
pub struct OrgSandboxes {
    material: OnceLock<Weak<dyn OrgMaterial>>,
    keys: Mutex<HashMap<String, (Instant, Option<String>)>>,
    marks: Mutex<HashMap<SessionId, Mark>>,
    locks: Mutex<HashMap<SessionId, Weak<tokio::sync::Mutex<()>>>>,
    /// Time a sync took, by `direction` (`push`, `pull`) and `kind`
    /// (`skip`, `unchanged`, `warm`, `cold`, `fresh`).
    pub sync: crate::prom::Histograms,
    /// Files a sync copied, by `direction` and `op` (`write`, `delete`).
    pub files: crate::prom::Counters,
    /// Sandboxes found orphaned before a create, and removed.
    pub orphans: crate::prom::Counters,
}

impl OrgSandboxes {
    /// Open provider keys through `material` from now on.
    pub fn bind(&self, material: Weak<dyn OrgMaterial>) {
        let _ = self.material.set(material);
    }

    /// `backends` for one activation: the provider key resolver, and the
    /// sync that keeps the session's sandbox and its workspace together.
    pub fn attach(self: &Arc<Self>, mut backends: HostBackends, files: CellFiles) -> HostBackends {
        if backends.connection_resolver.is_none() {
            backends.connection_resolver = Some(Arc::new(SandboxKeys(self.clone())));
        }
        let sync = SandboxWorkspaceSyncExt(Arc::new(CellSandboxSync {
            sandboxes: self.clone(),
            files,
        }));
        let embedder = backends.tool_context_extensions_factory.take();
        backends.tool_context_extensions_factory = Some(Arc::new(move |org, session| {
            let mut extensions = embedder
                .as_ref()
                .map(|factory| factory(org, session))
                .unwrap_or_default();
            extensions.insert(Arc::new(sync.clone()));
            extensions
        }));
        backends
    }

    /// The provider key the session's pinned sandbox names.
    async fn key(
        &self,
        session: SessionId,
        provider: &str,
        credential: &SessionSandboxCredential,
    ) -> anyhow::Result<Option<String>> {
        if matches!(
            credential.source,
            SessionSandboxCredentialSource::None | SessionSandboxCredentialSource::Platform
        ) {
            return Ok(None);
        }
        let material = self
            .material
            .get()
            .and_then(Weak::upgrade)
            .ok_or_else(|| anyhow::anyhow!("this node does not open org secrets"))?;
        let org = material
            .tenant(session)
            .ok_or_else(|| anyhow::anyhow!("session {session} has no org"))?;
        let secret = sandbox_key_secret(
            provider,
            credential.virtual_user_id,
            credential.connection_id,
        );
        let slot = format!("{org}\n{secret}");
        if let Some((at, key)) = self.keys.lock().get(&slot)
            && at.elapsed() < KEEP
        {
            return Ok(key.clone());
        }
        let key = material
            .open_secret(&org, &secret)
            .await?
            .map(|opened| String::from_utf8(opened.plain))
            .transpose()
            .map_err(|_| anyhow::anyhow!("the {provider} sandbox key is not text"))?;
        self.keys.lock().insert(slot, (Instant::now(), key.clone()));
        Ok(key)
    }

    /// One sync of `session` at a time in this process.
    fn lock(&self, session: SessionId) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock();
        if let Some(lock) = locks.get(&session).and_then(Weak::upgrade) {
            return lock;
        }
        locks.retain(|_, lock| lock.strong_count() > 0);
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        locks.insert(session, Arc::downgrade(&lock));
        lock
    }

    fn remember(&self, session: SessionId, mark: Mark) {
        let mut marks = self.marks.lock();
        // A forgotten mark costs one listing on the session's next call.
        if marks.len() >= MARKS_MAX && !marks.contains_key(&session) {
            marks.clear();
        }
        marks.insert(session, mark);
    }

    /// This process's mark of `session`, if it still describes `state`.
    fn warm(&self, session: SessionId, state: &SessionSandboxState) -> Option<Mark> {
        let record = Record::of(state)?;
        self.marks.lock().get(&session).cloned().filter(|mark| {
            mark.sandbox == state.instance.external_id
                && mark.version == record.version
                && mark.at == record.at
        })
    }
}

/// Provider keys for the sandbox tools; nothing else resolves here.
struct SandboxKeys(Arc<OrgSandboxes>);

#[async_trait]
impl everruns_core::UserConnectionResolver for SandboxKeys {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    async fn get_sandbox_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
        credential: &SessionSandboxCredential,
    ) -> Result<Option<String>> {
        self.0
            .key(session_id, provider, credential)
            .await
            .map_err(|error| AgentLoopError::store(error.to_string()))
    }
}

/// A sandbox's last sync, as its state records it.
#[derive(Debug, Clone, PartialEq)]
struct Record {
    version: u64,
    at: String,
}

impl Record {
    fn of(state: &SessionSandboxState) -> Option<Self> {
        let record = &state.instance.metadata[RECORD];
        Some(Self {
            version: record["version"].as_u64()?,
            at: record["at"].as_str()?.to_owned(),
        })
    }

    async fn save(
        self,
        context: &ToolContext,
        state: &mut SessionSandboxState,
    ) -> std::result::Result<(), ToolExecutionResult> {
        if !state.instance.metadata.is_object() {
            state.instance.metadata = json!({});
        }
        state.instance.metadata[RECORD] = json!({ "version": self.version, "at": self.at });
        state.updated_at = chrono::Utc::now().to_rfc3339();
        save_session_sandbox_state(context, state).await
    }
}

/// Whether `path` (a session path) is one neither side copies.
fn skipped(path: &str) -> bool {
    path.split('/').any(|part| SKIPPED.contains(&part))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn tool_error(error: impl std::fmt::Display) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(format!("Workspace sync failed: {error}"))
}

/// The sandbox's root for a session's workspace.
fn root(state: &SessionSandboxState) -> String {
    state
        .instance
        .workspace_path
        .as_deref()
        .unwrap_or("/workspace")
        .trim_end_matches('/')
        .to_owned()
}

/// The digest a scan prints for a set of session paths: SHA-256 of the
/// sorted paths relative to the workspace, one per line.
fn digest<'a>(paths: impl IntoIterator<Item = &'a String>) -> String {
    let mut hasher = sha2::Sha256::new();
    for path in paths {
        hasher.update(path.trim_start_matches('/').as_bytes());
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())
}

/// One file of a scan.
#[derive(Debug, Clone, PartialEq)]
struct Scanned {
    size: u64,
    changed_at: String,
}

/// What one scan of the sandbox found.
#[derive(Debug, Default)]
struct Scan {
    /// The sandbox's time when the scan started.
    now: String,
    /// Whether the workspace directory exists in the sandbox.
    present: bool,
    /// Digest and count of every file's path.
    digest: String,
    count: usize,
    /// Files changed since the scan's `since`, unless there were too many.
    changed: Option<BTreeMap<String, Scanned>>,
}

/// The scan script: prints the sandbox's time, a digest of every file's
/// path, and the files whose status changed after `since`. The full
/// listing stays at [`listing_path`].
fn scan_script(root: &str, since: &str) -> String {
    let prune = SKIPPED
        .iter()
        .map(|name| format!("-name {}", shell_quote(name)))
        .collect::<Vec<_>>()
        .join(" -o ");
    format!(
        r#"now=$(date +%s.%N)
echo "now $now"
cd {root} 2>/dev/null || {{ echo missing; exit 0; }}
find . -mindepth 1 \( {prune} \) -prune -o -type f -printf '%P\t%s\t%C@\n' 2>/dev/null | LC_ALL=C sort > {listing}
echo "paths $(cut -f1 {listing} | sha256sum | cut -d' ' -f1) $(wc -l < {listing})"
awk -F'\t' -v t={since} '$3 > t' {listing} | head -n {limit}
"#,
        root = shell_quote(root),
        listing = shell_quote(&listing_path(root)),
        since = shell_quote(since),
        limit = CHANGED_MAX + 1,
    )
}

/// `path\tsize\tctime` lines.
fn parse_listing(text: &str) -> BTreeMap<String, Scanned> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let (path, size, changed_at) = (parts.next()?, parts.next()?, parts.next()?);
            if parts.next().is_some() || path.is_empty() {
                return None;
            }
            Some((
                format!("/{path}"),
                Scanned {
                    size: size.parse().ok()?,
                    changed_at: changed_at.to_owned(),
                },
            ))
        })
        .collect()
}

fn parse_scan(stdout: &str) -> std::result::Result<Scan, ToolExecutionResult> {
    let mut lines = stdout.lines();
    let now = lines
        .next()
        .and_then(|line| line.strip_prefix("now "))
        .ok_or_else(|| tool_error("the sandbox did not report its time"))?
        .to_owned();
    let mut scan = Scan {
        now,
        ..Scan::default()
    };
    let Some(paths) = lines.next() else {
        return Err(tool_error("the sandbox scan stopped early"));
    };
    if paths == "missing" {
        scan.changed = Some(BTreeMap::new());
        return Ok(scan);
    }
    let mut fields = paths.split(' ');
    let (Some("paths"), Some(digest), Some(count)) = (fields.next(), fields.next(), fields.next())
    else {
        return Err(tool_error("the sandbox scan stopped early"));
    };
    scan.present = true;
    scan.digest = digest.to_owned();
    scan.count = count.trim().parse().map_err(tool_error)?;
    let rest = lines.collect::<Vec<_>>().join("\n");
    let changed = parse_listing(&rest);
    scan.changed = (changed.len() <= CHANGED_MAX).then_some(changed);
    Ok(scan)
}

/// Keeps a session's sandbox and its workspace in the cell together.
struct CellSandboxSync {
    sandboxes: Arc<OrgSandboxes>,
    files: CellFiles,
}

/// A sandbox call, for one sync.
struct Call<'a> {
    context: &'a ToolContext,
    provider: &'a dyn SessionSandboxProvider,
    config: &'a SessionSandboxConfig,
}

impl Call<'_> {
    async fn exec(
        &self,
        state: &SessionSandboxState,
        command: String,
    ) -> std::result::Result<String, ToolExecutionResult> {
        let response = self
            .provider
            .exec(
                self.context,
                self.config,
                &state.instance,
                &SessionSandboxExecRequest {
                    command,
                    cwd: None,
                    timeout_ms: Some(120_000),
                    output_mode: "full".into(),
                },
            )
            .await?;
        if !response.success {
            return Err(tool_error(format!(
                "exit {}: {}",
                response.exit_code,
                response.stderr.trim()
            )));
        }
        Ok(response.stdout)
    }

    async fn scan(
        &self,
        state: &SessionSandboxState,
        since: &str,
    ) -> std::result::Result<Scan, ToolExecutionResult> {
        let stdout = self.exec(state, scan_script(&root(state), since)).await?;
        parse_scan(&stdout)
    }

    /// Every file of the last scan.
    async fn listing(
        &self,
        state: &SessionSandboxState,
    ) -> std::result::Result<BTreeMap<String, Scanned>, ToolExecutionResult> {
        let file = self
            .provider
            .read_file(
                self.context,
                self.config,
                &state.instance,
                &listing_path(&root(state)),
            )
            .await?;
        let bytes = everruns_core::SessionFile::decode_content(&file.content, &file.encoding)
            .map_err(tool_error)?;
        Ok(parse_listing(&String::from_utf8_lossy(&bytes)))
    }

    /// Write the workspace's files at `paths` into the sandbox, remove
    /// `deletes` and create `dirs`; the sandbox's time after.
    async fn write(
        &self,
        state: &SessionSandboxState,
        files: &CellFiles,
        entries: &BTreeMap<String, Entry>,
        writes: &[String],
        deletes: &[String],
        dirs: &[String],
    ) -> std::result::Result<String, ToolExecutionResult> {
        let root = root(state);
        for path in writes {
            let Some(entry) = entries.get(path) else {
                continue;
            };
            let content = files
                .content(entry)
                .await
                .map_err(tool_error)?
                .unwrap_or_default();
            let bytes = everruns_core::SessionFile::decode_content(&content, &entry.encoding)
                .map_err(tool_error)?;
            self.provider
                .write_file(
                    self.context,
                    self.config,
                    &state.instance,
                    &format!("{root}{path}"),
                    &bytes,
                )
                .await?;
        }
        let mut script = String::new();
        for path in deletes {
            script.push_str(&format!(
                "rm -rf -- {}\n",
                shell_quote(&format!("{root}{path}"))
            ));
        }
        for path in dirs {
            script.push_str(&format!(
                "mkdir -p -- {}\n",
                shell_quote(&format!("{root}{path}"))
            ));
        }
        script.push_str("date +%s.%N\n");
        let stdout = self.exec(state, script).await?;
        Ok(stdout.lines().last().unwrap_or_default().trim().to_owned())
    }
}

/// The workspace's files (not directories) the sandbox mirrors.
fn synced_files(entries: &BTreeMap<String, Entry>) -> BTreeSet<String> {
    entries
        .iter()
        .filter(|(path, entry)| !entry.dir && !skipped(path))
        .map(|(path, _)| path.clone())
        .collect()
}

/// Whether the workspace's `entry` holds exactly `content` in `encoding`.
fn same(entry: &Entry, content: &str, encoding: &str) -> bool {
    entry.encoding == encoding
        && match &entry.content {
            Content::Inline { data } => data == content,
            Content::Blob { sha256 } => {
                *sha256 == hex::encode(sha2::Sha256::digest(content.as_bytes()))
            }
            Content::None => content.is_empty(),
        }
}

impl CellSandboxSync {
    async fn push_inner(
        &self,
        call: &Call<'_>,
        state: &mut SessionSandboxState,
        fresh: bool,
    ) -> std::result::Result<&'static str, ToolExecutionResult> {
        let session = call.context.session_id;
        let (version, entries) = self.files.snapshot(session).await.map_err(tool_error)?;
        let wanted = synced_files(&entries);
        let dirs = || {
            entries
                .iter()
                .filter(|(path, entry)| entry.dir && !skipped(path))
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>()
        };
        let warm = self.sandboxes.warm(session, state);
        if fresh && wanted.is_empty() && dirs().is_empty() {
            // An empty sandbox for an empty workspace: nothing to send, and
            // everything the sandbox holds at the next pull is new.
            self.sandboxes.remember(
                session,
                Mark {
                    sandbox: state.instance.external_id.clone(),
                    version,
                    at: "0".into(),
                    paths: Arc::default(),
                },
            );
            Record {
                version,
                at: "0".into(),
            }
            .save(call.context, state)
            .await?;
            return Ok("fresh");
        }
        let (kind, writes, deletes, dirs, mut paths) = if fresh {
            // An empty sandbox: everything goes in.
            let writes = wanted.iter().cloned().collect::<Vec<_>>();
            ("fresh", writes, Vec::new(), dirs(), BTreeSet::new())
        } else if let Some(mark) = warm {
            if mark.version == version {
                return Ok("skip");
            }
            // Only the workspace changed since this process last synced:
            // its newer files go in, its removed ones go out.
            let writes = wanted
                .iter()
                .filter(|path| entries.get(*path).is_some_and(|e| e.version > mark.version))
                .cloned()
                .collect::<Vec<_>>();
            let deletes = mark.paths.difference(&wanted).cloned().collect::<Vec<_>>();
            let dirs = entries
                .iter()
                .filter(|(path, e)| e.dir && e.version > mark.version && !skipped(path))
                .map(|(path, _)| path.clone())
                .collect();
            ("warm", writes, deletes, dirs, (*mark.paths).clone())
        } else {
            // This process does not know what the sandbox holds (another
            // executor ran the session, or this one restarted): list it, and
            // put back whatever differs from the workspace, including what
            // a call that never finished changed.
            let record = Record::of(state);
            let since = record.as_ref().map_or("0", |r| r.at.as_str()).to_owned();
            call.scan(state, &since).await?;
            let listing = call.listing(state).await?;
            let writes = wanted
                .iter()
                .filter(|path| {
                    let Some(entry) = entries.get(*path) else {
                        return false;
                    };
                    match (&record, listing.get(*path)) {
                        (None, _) | (_, None) => true,
                        (Some(record), Some(scanned)) => {
                            entry.version > record.version || newer(&scanned.changed_at, &record.at)
                        }
                    }
                })
                .cloned()
                .collect::<Vec<_>>();
            let deletes = listing
                .keys()
                .filter(|path| !wanted.contains(*path))
                .cloned()
                .collect::<Vec<_>>();
            let paths = listing.into_keys().collect();
            ("cold", writes, deletes, dirs(), paths)
        };
        let at = call
            .write(state, &self.files, &entries, &writes, &deletes, &dirs)
            .await?;
        self.sandboxes.files.add(
            &[("direction", "push"), ("op", "write")],
            writes.len() as u64,
        );
        self.sandboxes.files.add(
            &[("direction", "push"), ("op", "delete")],
            deletes.len() as u64,
        );
        for path in &deletes {
            paths.remove(path);
        }
        paths.extend(writes);
        let record = Record { version, at };
        self.sandboxes.remember(
            session,
            Mark {
                sandbox: state.instance.external_id.clone(),
                version: record.version,
                at: record.at.clone(),
                paths: Arc::new(paths),
            },
        );
        record.save(call.context, state).await?;
        Ok(kind)
    }

    async fn pull_inner(
        &self,
        call: &Call<'_>,
        state: &mut SessionSandboxState,
    ) -> std::result::Result<&'static str, ToolExecutionResult> {
        let session = call.context.session_id;
        let record = Record::of(state);
        let since = record.as_ref().map_or("0", |r| r.at.as_str()).to_owned();
        let synced_version = record.as_ref().map_or(0, |r| r.version);
        let warm = self.sandboxes.warm(session, state);
        let scan = call.scan(state, &since).await?;
        if !scan.present {
            return Err(tool_error("the sandbox has no workspace directory"));
        }
        // Every file the sandbox holds now: what it held, plus what the
        // call created, when the digest says nothing else changed.
        let (listing, changed, kind) = match (&warm, scan.changed) {
            (Some(mark), Some(changed)) => {
                let mut expected = (*mark.paths).clone();
                expected.extend(changed.keys().cloned());
                if expected.len() == scan.count && digest(&expected) == scan.digest {
                    (expected, changed, "warm")
                } else {
                    let listing = call.listing(state).await?;
                    (listing.keys().cloned().collect(), changed, "cold")
                }
            }
            (_, changed) => {
                let listing = call.listing(state).await?;
                let changed = changed.unwrap_or_else(|| {
                    listing
                        .iter()
                        .filter(|(_, scanned)| newer(&scanned.changed_at, &since))
                        .map(|(path, scanned)| (path.clone(), scanned.clone()))
                        .collect()
                });
                (listing.keys().cloned().collect(), changed, "cold")
            }
        };
        if kind == "warm" && changed.is_empty() {
            // The sandbox holds exactly what it held: the record stands,
            // and the next scan from it finds what changes after.
            return Ok("unchanged");
        }
        let (version, entries) = self.files.snapshot(session).await.map_err(tool_error)?;
        let root = root(state);
        let mut changes = Vec::new();
        let mut writes = 0u64;
        for (path, scanned) in &changed {
            let entry = entries.get(path);
            if skipped(path) || entry.is_some_and(|e| e.dir || e.readonly) {
                continue;
            }
            if scanned.size > FILE_MAX {
                tracing::warn!(%session, path, size = scanned.size, "a sandbox file too large for the workspace stays in the sandbox");
                continue;
            }
            let file = call
                .provider
                .read_file(
                    call.context,
                    call.config,
                    &state.instance,
                    &format!("{root}{path}"),
                )
                .await?;
            let bytes = everruns_core::SessionFile::decode_content(&file.content, &file.encoding)
                .map_err(tool_error)?;
            let (content, encoding) = everruns_core::SessionFile::encode_content(&bytes);
            if entry.is_some_and(|e| same(e, &content, &encoding)) {
                continue;
            }
            let size = content.len() as u64;
            changes.push(Change::Put {
                path: path.clone(),
                encoding,
                content: self.files.stage(&content).await.map_err(tool_error)?,
                size,
                readonly: false,
                if_absent: false,
                expected: None,
            });
            writes += 1;
        }
        // The workspace's files the sandbox no longer holds, unless the
        // workspace changed them during the call (a person's upload wins).
        let mut deletes = 0u64;
        for (path, entry) in &entries {
            if entry.dir
                || entry.readonly
                || skipped(path)
                || entry.version > synced_version
                || listing.contains(path)
            {
                continue;
            }
            changes.push(Change::Delete {
                path: path.clone(),
                recursive: false,
            });
            deletes += 1;
        }
        let committed = if changes.is_empty() {
            version
        } else {
            let reply = self
                .files
                .commit(session, changes)
                .await
                .map_err(tool_error)?;
            reply["version"].as_u64().unwrap_or(version)
        };
        self.sandboxes
            .files
            .add(&[("direction", "pull"), ("op", "write")], writes);
        self.sandboxes
            .files
            .add(&[("direction", "pull"), ("op", "delete")], deletes);
        // A workspace that also changed during the call keeps the version
        // the sandbox matched, so the next push sends what changed.
        let synced = if version == synced_version {
            committed
        } else {
            synced_version
        };
        let record = Record {
            version: synced,
            at: scan.now,
        };
        self.sandboxes.remember(
            session,
            Mark {
                sandbox: state.instance.external_id.clone(),
                version: record.version,
                at: record.at.clone(),
                paths: Arc::new(listing.into_iter().filter(|p| !skipped(p)).collect()),
            },
        );
        record.save(call.context, state).await?;
        Ok(kind)
    }
}

/// Whether sandbox time `a` is after `b` (`date +%s.%N` / `find %C@`).
fn newer(a: &str, b: &str) -> bool {
    match (a.parse::<f64>(), b.parse::<f64>()) {
        (Ok(a), Ok(b)) => a > b,
        _ => true,
    }
}

#[async_trait]
impl SandboxWorkspaceSync for CellSandboxSync {
    async fn before_create(
        &self,
        context: &ToolContext,
        provider: &dyn SessionSandboxProvider,
        config: &SessionSandboxConfig,
    ) -> std::result::Result<(), ToolExecutionResult> {
        // A sandbox labeled with this session while the session records
        // none was created by a call whose executor died before it saved
        // the record: remove it before creating the one the session keeps.
        let token = context
            .sandbox_connection_token(&config.provider, &config.credential)
            .await?
            .unwrap_or_default();
        let labels = BTreeMap::from([(
            "everruns.session_id".to_owned(),
            context.session_id.to_string(),
        )]);
        let owned = match provider.list_owned(&token, &labels).await {
            Ok(owned) => owned,
            Err(everruns_contracts::session_sandbox::SandboxAdminError::Unsupported) => {
                return Ok(());
            }
            Err(error) => {
                tracing::warn!(session = %context.session_id, %error, "could not look for an orphaned sandbox");
                return Ok(());
            }
        };
        for sandbox in owned {
            if sandbox.labels.get("everruns.session_id") != labels.get("everruns.session_id") {
                continue;
            }
            match provider.delete_by_id(&token, &sandbox.external_id).await {
                Ok(()) => {
                    self.sandboxes
                        .orphans
                        .inc(&[("provider", config.provider.as_str())]);
                    tracing::info!(session = %context.session_id, sandbox = %sandbox.external_id, "removed an orphaned sandbox");
                }
                Err(error) => {
                    tracing::warn!(session = %context.session_id, sandbox = %sandbox.external_id, %error, "could not remove an orphaned sandbox");
                }
            }
        }
        Ok(())
    }

    async fn push(
        &self,
        context: &ToolContext,
        provider: &dyn SessionSandboxProvider,
        config: &SessionSandboxConfig,
        state: &mut SessionSandboxState,
        fresh: bool,
    ) -> std::result::Result<(), ToolExecutionResult> {
        let started = Instant::now();
        let lock = self.sandboxes.lock(context.session_id);
        let _held = lock.lock().await;
        let call = Call {
            context,
            provider,
            config,
        };
        let kind = self.push_inner(&call, state, fresh).await?;
        self.sandboxes
            .sync
            .observe(&[("direction", "push"), ("kind", kind)], started.elapsed());
        Ok(())
    }

    async fn pull(
        &self,
        context: &ToolContext,
        provider: &dyn SessionSandboxProvider,
        config: &SessionSandboxConfig,
        state: &mut SessionSandboxState,
    ) -> std::result::Result<(), ToolExecutionResult> {
        let started = Instant::now();
        let lock = self.sandboxes.lock(context.session_id);
        let _held = lock.lock().await;
        let call = Call {
            context,
            provider,
            config,
        };
        let kind = self.pull_inner(&call, state).await?;
        self.sandboxes
            .sync
            .observe(&[("direction", "pull"), ("kind", kind)], started.elapsed());
        Ok(())
    }
}

/// Capabilities a pinned managed sandbox replaces in its session's agent,
/// as the platform's workers apply one (`apply_sandbox_to_capabilities`).
pub const REPLACED: &[&str] = &[
    "session_sandbox",
    "container_sandbox",
    "daytona",
    "e2b",
    "docker_container",
    "bashkit_shell",
    "session_file_system",
];

/// The sandbox capability's config a control plane pinned for a session
/// (`OrgSession.sandbox`), as a definition's capability entry.
pub fn capability_entry(sandbox: &Value) -> Option<Value> {
    sandbox["ref"]
        .as_str()
        .is_some_and(|id| {
            id == everruns_capabilities::session_sandbox::SESSION_SANDBOX_CAPABILITY_ID
        })
        .then(|| sandbox.clone())
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
