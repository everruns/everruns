//! A session's Memory on the data plane (data plane v2 parity, Phase B:
//! Console Chat and every agent with Memory): `/memory/agent`,
//! `/memory/user`, `/memory/shared`, `/team` and `/room`, mounted over the
//! session's workspace.
//!
//! Decisions:
//! - Memory stays in the control plane: it is shared across sessions,
//!   people and agents of the org, so no one session's shard can own it.
//!   The control plane serves it as files ([`MEMORY_FILES`], an injected
//!   files server speaking `io.everruns/files`): each operation is one MCP
//!   request there with a session token naming the session and its org,
//!   served only inside the session's own mounts, resolved from its record
//!   (`data_plane::memory` in the server).
//! - A session mounts only what the control plane sent with its create
//!   ([`crate::platform::Platform::memory`]), so a session without Memory
//!   never calls. A subagent's child gets its parent's mounts; the control
//!   plane resolves them against the child's own record.
//! - Nothing is cached: a read sees what another session wrote a moment
//!   ago, as on a v1 worker.
//! - No content search: the control plane's file service searches a whole
//!   workspace, not one mount. A shell's `grep` reads the files instead.
//! - A session whose create brought no files server (a standalone shard)
//!   has no Memory.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::capabilities::injected::MEMORY_FILES;
use everruns_contracts::runtime::mcp_extensions::files_method;
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::HostBackends;
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use everruns_core::session_files::SessionFileSystem;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::providers::OrgMaterial;

/// A session's Memory as its control plane gives it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMemory {
    /// The mounts it has ([`MOUNTS`]).
    pub mounts: Vec<String>,
    /// The control plane's files server they live on
    /// ([`crate::platform::Platform::servers`]).
    pub server: Option<String>,
}

impl SessionMemory {
    /// A session's Memory, from its platform record.
    pub fn of(platform: &crate::platform::Platform) -> Self {
        Self {
            mounts: platform.memory.clone(),
            server: platform
                .servers
                .get(everruns_contracts::runtime::capabilities::injected::MEMORY_FILES)
                .cloned(),
        }
    }
}

/// The Memory mount points a control plane may give a session.
pub const MOUNTS: &[&str] = &[
    "/memory/agent",
    "/memory/user",
    "/memory/shared",
    "/team",
    "/room",
];

/// `backends` with the mounts of `session`'s Memory, served by its files
/// server. None (no control plane): no Memory.
pub(crate) fn attach(
    mut backends: HostBackends,
    material: Option<&std::sync::Weak<dyn OrgMaterial>>,
    session: SessionId,
    memory: &SessionMemory,
) -> HostBackends {
    if memory.mounts.is_empty() {
        return backends;
    }
    let Some(server) = memory.server.as_deref().and_then(|url| {
        crate::mcp::ControllerServer::for_session(material, session, MEMORY_FILES, url)
    }) else {
        return backends;
    };
    for mount in memory
        .mounts
        .iter()
        .filter(|m| MOUNTS.contains(&m.as_str()))
    {
        let backend: Arc<dyn SessionFileSystem> = Arc::new(ShardMemory {
            server: server.clone(),
            session,
            mount: mount.clone(),
        });
        backends.memory_mounts.push((
            mount.clone(),
            backend,
            everruns_core::dreaming::DreamMemoryScope {
                key: format!("{session}:{mount}"),
                private: mount == "/memory/user",
            },
        ));
    }
    backends
}

/// One Memory mount of one session, in the control plane.
struct ShardMemory {
    /// The control plane's files server, with a session token.
    server: crate::mcp::ControllerServer,
    session: SessionId,
    /// Where it hangs (`/memory/user`).
    mount: String,
}

impl ShardMemory {
    /// The control plane's path for `path` inside this mount.
    fn outer(&self, path: &str) -> String {
        let path = path.trim_end_matches('/');
        if path.is_empty() {
            self.mount.clone()
        } else if path.starts_with('/') {
            format!("{}{path}", self.mount)
        } else {
            format!("{}/{path}", self.mount)
        }
    }

    /// `path` from the control plane, as this mount's own.
    fn inner(&self, path: &mut String) {
        if *path == self.mount {
            *path = "/".into();
        } else if let Some(rest) = path
            .strip_prefix(&self.mount)
            .filter(|rest| rest.starts_with('/'))
        {
            *path = rest.to_owned();
        }
    }

    /// Run `op` on `path` with `args`; the answer as `T`.
    async fn call<T: DeserializeOwned>(
        &self,
        session: SessionId,
        op: &str,
        path: &str,
        args: Value,
    ) -> Result<T> {
        if session != self.session {
            return Err(AgentLoopError::tool(
                "a session's tools reach only its own Memory",
            ));
        }
        let mut params = json!({ "path": self.outer(path) });
        if let (Some(params), Value::Object(args)) = (params.as_object_mut(), args) {
            params.extend(args);
        }
        let answer = self
            .server
            .client
            .files(&self.server.connection, &files_method(op), params)
            .await
            .map_err(|error| AgentLoopError::tool(error.to_string()))?;
        serde_json::from_value(answer).map_err(|error| AgentLoopError::store(error.to_string()))
    }

    fn file(&self, mut file: SessionFile) -> SessionFile {
        self.inner(&mut file.path);
        file
    }

    fn info(&self, mut info: FileInfo) -> FileInfo {
        self.inner(&mut info.path);
        info
    }
}

#[async_trait]
impl SessionFileSystem for ShardMemory {
    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn read_file(&self, session: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let file: Option<SessionFile> = self.call(session, "read", path, json!({})).await?;
        Ok(file.map(|file| self.file(file)))
    }

    async fn create_file_if_absent(
        &self,
        session: SessionId,
        path: &str,
        content: &str,
    ) -> Result<bool> {
        self.call(
            session,
            "create_if_absent",
            path,
            json!({ "content": content }),
        )
        .await
    }

    async fn write_file(
        &self,
        session: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let file: SessionFile = self
            .call(
                session,
                "write",
                path,
                json!({ "content": content, "encoding": encoding }),
            )
            .await?;
        Ok(self.file(file))
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
        let file: Option<SessionFile> = self
            .call(
                session,
                "write_if_matches",
                path,
                json!({
                    "expected_content": expected_content,
                    "expected_encoding": expected_encoding,
                    "content": content,
                    "encoding": encoding,
                }),
            )
            .await?;
        Ok(file.map(|file| self.file(file)))
    }

    async fn delete_file(&self, session: SessionId, path: &str, recursive: bool) -> Result<bool> {
        self.call(session, "delete", path, json!({ "recursive": recursive }))
            .await
    }

    async fn list_directory(&self, session: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let entries: Vec<FileInfo> = self.call(session, "list", path, json!({})).await?;
        Ok(entries.into_iter().map(|info| self.info(info)).collect())
    }

    async fn stat_file(&self, session: SessionId, path: &str) -> Result<Option<FileStat>> {
        let stat: Option<FileStat> = self.call(session, "stat", path, json!({})).await?;
        Ok(stat.map(|mut stat| {
            self.inner(&mut stat.path);
            stat
        }))
    }

    async fn grep_files(
        &self,
        _session: SessionId,
        _pattern: &str,
        _path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        Ok(Vec::new())
    }

    async fn create_directory(&self, session: SessionId, path: &str) -> Result<FileInfo> {
        let info: FileInfo = self.call(session, "mkdir", path, json!({})).await?;
        Ok(self.info(info))
    }
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
