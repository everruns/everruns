//! A session's workspace over the cell, as Engine's Environment: the
//! [`SessionFileSystem`] its file tools and shell use, reading the
//! replica (or an executor's mirror) and committing through the cell. The
//! rules a commit follows are in `files.rs`. Split from `hosting.rs` for
//! size.
//!
//! Decisions:
//! - Reads come from the session's state where this hosting reads
//!   everything else; contents over the inline limit come from the object
//!   store through the segment cache (a blob's name is its hash, so a cached
//!   copy is never stale). An executor greps a warm workspace without a
//!   round trip.
//! - Writes commit under the epoch this hosting holds for the session, so a
//!   superseded executor's writes are refused.
//! - Starter files are created only if absent. Engine seeds a definition's
//!   files at every activation; with a durable workspace that must not put
//!   back a file the agent changed (the same bug prod's first Engine
//!   session hit, step 4).

use super::*;

use std::collections::BTreeMap;

use everruns::hosting::EnvironmentFactory;
use everruns::{
    Environment, Workspace, WorkspaceBackend, WorkspaceBackendId, WorkspaceBinding,
    WorkspaceCheckpoint, WorkspaceDescriptor, WorkspaceDiff, WorkspaceError,
    WorkspaceHeadDescriptor, WorkspaceHeadId, WorkspaceHeadRequest, WorkspaceHeadResource,
    WorkspaceHeadStatus,
};
use everruns_contracts::typed_id::WorkspaceId;
use everruns_core::session_file::{
    FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, InitialFile, SessionFile,
};
use everruns_core::session_files::SessionFileSystem;

use crate::files::{Change, Content, Entry, INLINE_MAX};

const BACKEND_ID: &str = "everruns.cell.workspace.v1";
/// Grep limits, as Everruns' own stores apply them.
const GREP_FILE_MAX: usize = 512 * 1024;
const GREP_TOTAL_MAX: usize = 5 * 1024 * 1024;
const PATTERN_MAX: usize = 1_000;

/// The cell's workspaces as Engine Environments: one head per session,
/// named by the session.
#[derive(Clone)]
pub struct CellFiles {
    inner: Arc<Inner>,
}

impl RaftHosting {
    /// Sessions' workspaces in this hosting's cell, as Engine's definition
    /// Environment.
    pub fn files(&self) -> CellFiles {
        CellFiles {
            inner: self.inner.clone(),
        }
    }
}

fn path_of(input: &str) -> String {
    everruns_core::session_path::to_session_path(input)
}

fn name_of(path: &str) -> String {
    FileInfo::name_from_path(path)
}

fn time(ms: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

fn file_id(session: SessionId, path: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&session.uuid(), path.as_bytes())
}

fn info(session: SessionId, path: &str, e: &Entry) -> FileInfo {
    FileInfo {
        id: file_id(session, path),
        session_id: session.uuid(),
        path: path.to_string(),
        name: name_of(path),
        is_directory: e.dir,
        is_readonly: e.readonly,
        size_bytes: e.size as i64,
        created_at: time(e.created_ms),
        updated_at: time(e.updated_ms),
    }
}

fn root(session: SessionId) -> SessionFile {
    SessionFile {
        id: uuid::Uuid::nil(),
        session_id: session.uuid(),
        path: "/".into(),
        name: "/".into(),
        content: None,
        encoding: "text".into(),
        is_directory: true,
        is_readonly: false,
        size_bytes: 0,
        created_at: time(0),
        updated_at: time(0),
    }
}

impl CellFiles {
    fn store(&self) -> Result<crate::store::Store> {
        match &self.inner.source {
            Source::Node(_) => Ok(self.inner.node()?.shared.store.clone()),
            Source::Remote(remote) => Ok(remote.shared.store.clone()),
        }
    }

    async fn entry(&self, session: SessionId, path: &str) -> Result<Option<Entry>> {
        self.inner
            .read(session, |s| s.files.entries.get(path).cloned())
            .await
    }

    async fn entries(&self, session: SessionId) -> Result<BTreeMap<String, Entry>> {
        self.inner.read(session, |s| s.files.entries.clone()).await
    }

    async fn content(&self, entry: &Entry) -> Result<Option<String>> {
        match &entry.content {
            Content::None => Ok(None),
            Content::Inline { data } => Ok(Some(data.clone())),
            Content::Blob { sha256 } => {
                let bytes = self.store()?.get_blob(sha256).await.map_err(store_err)?;
                String::from_utf8(bytes.to_vec())
                    .map(Some)
                    .map_err(|_| AgentLoopError::store("a workspace blob is not the text it was"))
            }
        }
    }

    async fn file(&self, session: SessionId, path: &str, e: &Entry) -> Result<SessionFile> {
        Ok(SessionFile {
            id: file_id(session, path),
            session_id: session.uuid(),
            path: path.to_string(),
            name: name_of(path),
            content: self.content(e).await?,
            encoding: e.encoding.clone(),
            is_directory: e.dir,
            is_readonly: e.readonly,
            size_bytes: e.size as i64,
            created_at: time(e.created_ms),
            updated_at: time(e.updated_ms),
        })
    }

    /// The content as a commit carries it: inline, or stored first.
    async fn stage(&self, content: &str) -> Result<Content> {
        if content.len() <= INLINE_MAX {
            return Ok(Content::Inline {
                data: content.to_string(),
            });
        }
        let sha256 = self
            .store()?
            .put_blob(content.as_bytes().to_vec())
            .await
            .map_err(store_err)?;
        Ok(Content::Blob { sha256 })
    }

    /// Commit `changes` under the epoch held here, if any.
    async fn commit(&self, session: SessionId, changes: Vec<Change>) -> Result<Value> {
        let fence = self.inner.epochs.lock().get(&session).copied();
        let response = self
            .inner
            .propose(
                session,
                Command::Files {
                    session,
                    fence,
                    op_id: uuid::Uuid::new_v4().to_string(),
                    changes,
                    now_ms: now_ms(),
                },
            )
            .await?;
        let value = match response {
            Response::Value { value } => value,
            Response::Fenced => {
                return Err(actor_lease_lost(
                    session,
                    "workspace write after the epoch ended",
                ));
            }
            other => return Err(refused(session, other)),
        };
        match value["refused"].as_str() {
            None => Ok(value),
            Some("read_only") => Err(AgentLoopError::tool(format!(
                "file is read-only: {}",
                value["path"].as_str().unwrap_or_default()
            ))),
            Some("conflict") => Err(AgentLoopError::tool(format!(
                "file changed since it was read: {}",
                value["path"].as_str().unwrap_or_default()
            ))),
            Some("not_a_directory") => Err(AgentLoopError::tool(format!(
                "not a directory: {}",
                value["path"].as_str().unwrap_or_default()
            ))),
            Some("quota") => Err(AgentLoopError::tool("the workspace is full")),
            Some(_) => Err(AgentLoopError::tool(
                value["reason"]
                    .as_str()
                    .unwrap_or("invalid workspace change")
                    .to_string(),
            )),
        }
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "one put's fields as the workspace port passes them; a struct for four call sites adds nothing"
    )]
    async fn put(
        &self,
        session: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
        readonly: bool,
        if_absent: bool,
        expected: Option<u64>,
    ) -> Result<Value> {
        let staged = self.stage(content).await?;
        self.commit(
            session,
            vec![Change::Put {
                path: path.to_string(),
                encoding: encoding.to_string(),
                content: staged,
                size: content.len() as u64,
                readonly,
                if_absent,
                expected,
            }],
        )
        .await
    }

    async fn written(&self, session: SessionId, path: &str) -> Result<SessionFile> {
        let entry = self
            .entry(session, path)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("file not found after write: {path}")))?;
        self.file(session, path, &entry).await
    }

    /// Text files to scan, within Everruns' grep limits.
    async fn scan(
        &self,
        session: SessionId,
        path_pattern: Option<&str>,
    ) -> Result<Vec<(String, String)>> {
        if path_pattern.is_some_and(|p| p.len() > PATTERN_MAX) {
            return Err(AgentLoopError::tool(format!(
                "Path pattern too long (max {PATTERN_MAX} characters)"
            )));
        }
        let matcher = path_pattern
            .map(everruns_core::session_path::GrepPathPattern::new)
            .transpose()?;
        let mut total = 0usize;
        let mut files = Vec::new();
        for (path, entry) in self.entries(session).await? {
            if entry.dir || entry.encoding != "text" || entry.size as usize > GREP_FILE_MAX {
                continue;
            }
            if matcher.as_ref().is_some_and(|m| !m.is_match(&path)) {
                continue;
            }
            total += entry.size as usize;
            if total > GREP_TOTAL_MAX {
                return Err(AgentLoopError::tool(format!(
                    "Grep scan exceeds maximum total size of {GREP_TOTAL_MAX} bytes"
                )));
            }
            if let Some(content) = self.content(&entry).await? {
                files.push((path, content));
            }
        }
        Ok(files)
    }
}

fn regex_of(pattern: &str) -> Result<regex::Regex> {
    if pattern.len() > PATTERN_MAX {
        return Err(AgentLoopError::tool(format!(
            "Regex pattern too long (max {PATTERN_MAX} characters)"
        )));
    }
    regex::RegexBuilder::new(pattern)
        .size_limit(512 * 1024)
        .build()
        .map_err(|error| AgentLoopError::tool(format!("Invalid regex pattern: {error}")))
}

#[async_trait]
impl SessionFileSystem for CellFiles {
    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn read_file(&self, session: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let path = path_of(path);
        if path == "/" {
            return Ok(Some(root(session)));
        }
        match self.entry(session, &path).await? {
            Some(entry) => Ok(Some(self.file(session, &path, &entry).await?)),
            None => Ok(None),
        }
    }

    async fn create_file_if_absent(
        &self,
        session: SessionId,
        path: &str,
        content: &str,
    ) -> Result<bool> {
        let path = path_of(path);
        let reply = self
            .put(session, &path, content, "text", false, true, None)
            .await?;
        Ok(reply["results"][0]["created"] == true)
    }

    async fn write_file(
        &self,
        session: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let path = path_of(path);
        self.put(session, &path, content, encoding, false, false, None)
            .await?;
        self.written(session, &path).await
    }

    async fn write_file_if_content_matches(
        &self,
        session: SessionId,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        let path = path_of(path);
        let Some(entry) = self.entry(session, &path).await? else {
            return Ok(None);
        };
        if entry.dir
            || entry.encoding != expected_encoding
            || self.content(&entry).await?.as_deref() != Some(expected_content)
        {
            return Ok(None);
        }
        // Compare-and-set on the version read, so a write in between is a
        // conflict here rather than lost.
        match self
            .put(
                session,
                &path,
                content,
                encoding,
                false,
                false,
                Some(entry.version),
            )
            .await
        {
            Ok(_) => Ok(Some(self.written(session, &path).await?)),
            Err(error) if error.to_string().contains("changed since it was read") => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn delete_file(&self, session: SessionId, path: &str, recursive: bool) -> Result<bool> {
        let path = path_of(path);
        if path == "/" {
            return Ok(false);
        }
        let reply = self
            .commit(session, vec![Change::Delete { path, recursive }])
            .await?;
        Ok(reply["results"][0]["changed"] == true)
    }

    async fn list_directory(&self, session: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let path = path_of(path);
        let entries = self.entries(session).await?;
        if path != "/" && !entries.get(&path).is_some_and(|e| e.dir) {
            return Ok(Vec::new());
        }
        Ok(entries
            .iter()
            .filter(|(p, _)| FileInfo::parent_path(p).as_deref() == Some(path.as_str()))
            .map(|(p, e)| info(session, p, e))
            .collect())
    }

    async fn list_tree(&self, session: SessionId, path: &str) -> Result<Option<Vec<FileInfo>>> {
        let path = path_of(path);
        let prefix = if path == "/" {
            "/".to_string()
        } else {
            format!("{path}/")
        };
        let entries = self.entries(session).await?;
        Ok(Some(
            entries
                .iter()
                .filter_map(|(p, e)| {
                    let rest = p.strip_prefix(&prefix)?;
                    let mut entry = info(session, p, e);
                    entry.path = rest.to_string();
                    Some(entry)
                })
                .collect(),
        ))
    }

    async fn stat_file(&self, session: SessionId, path: &str) -> Result<Option<FileStat>> {
        let path = path_of(path);
        if path == "/" {
            let root = root(session);
            return Ok(Some(FileStat {
                path: root.path,
                name: root.name,
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: root.created_at,
                updated_at: root.updated_at,
            }));
        }
        Ok(self.entry(session, &path).await?.map(|e| FileStat {
            path: path.clone(),
            name: name_of(&path),
            is_directory: e.dir,
            is_readonly: e.readonly,
            size_bytes: e.size as i64,
            created_at: time(e.created_ms),
            updated_at: time(e.updated_ms),
        }))
    }

    async fn grep_files(
        &self,
        session: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let regex = regex_of(pattern)?;
        let mut matches = Vec::new();
        for (path, content) in self.scan(session, path_pattern).await? {
            for (index, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(GrepMatch {
                        path: path.clone(),
                        line_number: index + 1,
                        line: line.to_string(),
                    });
                }
            }
        }
        Ok(matches)
    }

    async fn grep_files_with_options(
        &self,
        session: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        let regex = regex_of(pattern)?;
        let files = self.scan(session, options.path_pattern.as_deref()).await?;
        Ok(everruns_core::session_file::build_grep_search_result(
            files, &regex, options,
        ))
    }

    async fn create_directory(&self, session: SessionId, path: &str) -> Result<FileInfo> {
        let path = path_of(path);
        if path == "/" {
            let root = root(session);
            return Ok(FileInfo {
                id: root.id,
                session_id: root.session_id,
                path: root.path,
                name: root.name,
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: root.created_at,
                updated_at: root.updated_at,
            });
        }
        self.commit(session, vec![Change::Mkdir { path: path.clone() }])
            .await?;
        let entry = self
            .entry(session, &path)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("directory not found: {path}")))?;
        Ok(info(session, &path, &entry))
    }

    async fn seed_initial_file(&self, session: SessionId, file: &InitialFile) -> Result<()> {
        let path = path_of(&file.path);
        self.put(
            session,
            &path,
            &file.content,
            &file.encoding,
            file.is_readonly,
            true,
            None,
        )
        .await
        .map(|_| ())
    }
}

#[expect(
    clippy::expect_used,
    reason = "BACKEND_ID is a constant valid backend id; the workspace test opens a head with it"
)]
fn backend_id() -> WorkspaceBackendId {
    WorkspaceBackendId::new(BACKEND_ID).expect("static backend id is valid")
}

/// A session's workspace has the session's id, and so has its one head.
///
/// Decision: Engine's file tools key a workspace's files by the workspace's
/// id, so each session is its own workspace; one shared workspace for the
/// cell would put every session's files under one key.
fn workspace_of(session: uuid::Uuid) -> WorkspaceId {
    WorkspaceId::from_uuid(session)
}

impl CellFiles {
    fn descriptor(session: uuid::Uuid) -> WorkspaceDescriptor {
        WorkspaceDescriptor {
            id: workspace_of(session),
            name: format!("session {session}"),
            metadata: BTreeMap::new(),
        }
    }

    /// The session a binding is the workspace of.
    fn check(binding: &WorkspaceBinding) -> std::result::Result<uuid::Uuid, WorkspaceError> {
        let session = binding.head_id.uuid();
        if binding.provider_id != backend_id() || binding.workspace_id != workspace_of(session) {
            return Err(WorkspaceError::BindingMismatch);
        }
        Ok(session)
    }

    fn resource(
        &self,
        binding: WorkspaceBinding,
    ) -> std::result::Result<WorkspaceHeadResource, WorkspaceError> {
        let session = Self::check(&binding)?;
        Ok(WorkspaceHeadResource {
            workspace: Self::descriptor(session),
            head: WorkspaceHeadDescriptor {
                id: binding.head_id,
                name: session.to_string(),
                base: None,
                access: binding.access,
                metadata: BTreeMap::new(),
            },
            binding,
            file_system: Arc::new(self.clone()),
        })
    }
}

#[async_trait]
impl WorkspaceBackend for CellFiles {
    fn id(&self) -> WorkspaceBackendId {
        backend_id()
    }

    /// `locator` is the session's uuid.
    async fn open_workspace(
        &self,
        locator: &str,
    ) -> std::result::Result<WorkspaceDescriptor, WorkspaceError> {
        let session: uuid::Uuid = locator.parse().map_err(|_| WorkspaceError::NotFound)?;
        Ok(Self::descriptor(session))
    }

    async fn open_workspace_from_binding(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceDescriptor, WorkspaceError> {
        Ok(Self::descriptor(Self::check(binding)?))
    }

    async fn create_head(
        &self,
        workspace: &WorkspaceDescriptor,
        request: WorkspaceHeadRequest,
    ) -> std::result::Result<WorkspaceHeadResource, WorkspaceError> {
        let session = workspace.id.uuid();
        if request.base.is_some() || request.name != session.to_string() {
            return Err(WorkspaceError::InvalidRequest(
                "a cell workspace has one head, named by its session, with no base".into(),
            ));
        }
        self.resource(WorkspaceBinding {
            provider_id: backend_id(),
            workspace_id: workspace_of(session),
            head_id: WorkspaceHeadId::from_uuid(session),
            access: request.access,
            payload: Vec::new(),
        })
    }

    async fn reopen_head(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceHeadResource, WorkspaceError> {
        self.resource(binding.clone())
    }

    async fn checkpoint(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceCheckpoint, WorkspaceError> {
        Self::check(binding)?;
        Ok(WorkspaceCheckpoint {
            revision: binding.head_id.to_string(),
            metadata: BTreeMap::new(),
        })
    }

    async fn status(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceHeadStatus, WorkspaceError> {
        Self::check(binding)?;
        Ok(WorkspaceHeadStatus::default())
    }

    async fn diff(
        &self,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<WorkspaceDiff, WorkspaceError> {
        Self::check(binding)?;
        Ok(WorkspaceDiff::default())
    }

    async fn archive(&self, binding: &WorkspaceBinding) -> std::result::Result<(), WorkspaceError> {
        Self::check(binding)?;
        Err(WorkspaceError::InvalidRequest(
            "a cell workspace lives as long as its session".into(),
        ))
    }

    async fn destroy(&self, binding: &WorkspaceBinding) -> std::result::Result<(), WorkspaceError> {
        Self::check(binding)?;
        Err(WorkspaceError::InvalidRequest(
            "a cell workspace lives as long as its session".into(),
        ))
    }
}

#[async_trait]
impl EnvironmentFactory for CellFiles {
    async fn open(&self, session: SessionId) -> Result<Environment> {
        let workspace_error = |error: WorkspaceError| AgentLoopError::store(error.to_string());
        let backend: Arc<dyn WorkspaceBackend> = Arc::new(self.clone());
        let head = Workspace::open(backend, &session.uuid().to_string())
            .await
            .map_err(workspace_error)?
            .head(session.uuid().to_string())
            .create()
            .await
            .map_err(workspace_error)?;
        Environment::builder()
            .workspace(head)
            .build()
            .map_err(|error| AgentLoopError::config(error.to_string()))
    }
}
