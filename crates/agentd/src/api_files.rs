//! A node's side of a session's workspace API (data plane v2 step 11.1):
//! the console's and the CLI's file calls, which the gateway turns into one
//! [`FsOp`] each (`gateway_fs.rs`). The workspace itself is `files.rs`;
//! reads and commits go through the node's hosting (`hosting_files.rs`).
//!
//! Decisions:
//! - Reads are linearizable. The gateway calls any member of the shard, and
//!   a follower's replica may trail the leader, so a read first takes the
//!   leader's read index (`/shard/read-index`, a quorum heartbeat) and waits
//!   until this replica applied it. Without it an upload followed by a
//!   listing could miss the upload.
//! - Writes are unfenced, as every gateway write: a person's upload is not
//!   an executor's, and an executor's epoch must not refuse it.
//! - A failure the caller made (a missing file, an existing target, a
//!   read-only file) comes back as [`FsResult::Failed`] with the HTTP status
//!   the control plane answers for it. Only the session's own whereabouts
//!   (moved, frozen, missing, shedding) use the [`ShardReply`] variants, so
//!   the gateway's retry loop never mistakes "file not found" for "session
//!   not found".
//! - Binary content travels base64, as the control plane's file API
//!   carries it, and is stored that way; an entry's size is the decoded
//!   size, which is what the console shows and the quota counts.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use everruns_contracts::error::AgentLoopError;
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_file::{FileInfo, GrepMatch, GrepOptions, InitialFile, SessionFile};
use everruns_core::session_files::SessionFileSystem as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{ShardReply, reply_of};
use crate::files::{Change, Entry};
use crate::hosting::files::{ShardFiles, info};
use crate::node::Node;

type ApiError = (StatusCode, String);

/// The largest file the API takes in one call; the workspace quota bounds
/// the total.
pub const FILE_MAX: usize = 32 * 1024 * 1024;

pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/shard/read-index", get(read_index))
        .route("/shard/sessions/{session}/fs", post(fs))
        .layer(axum::extract::DefaultBodyLimit::max(FILE_MAX * 2))
        .with_state(node)
}

/// One workspace call, as the gateway sends it. Paths are as the client
/// wrote them; they are made canonical here.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FsOp {
    /// A file with its content, or a directory's listing.
    Get {
        path: String,
        #[serde(default)]
        recursive: bool,
    },
    Stat {
        path: String,
    },
    /// A file's stored content and encoding, for downloads and previews.
    Raw {
        path: String,
    },
    Create {
        path: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        encoding: Option<String>,
        #[serde(default)]
        readonly: bool,
        #[serde(default)]
        directory: bool,
    },
    Update {
        path: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        encoding: Option<String>,
        #[serde(default)]
        readonly: Option<bool>,
        #[serde(default)]
        expected_content: Option<String>,
        #[serde(default)]
        expected_encoding: Option<String>,
    },
    /// Write a whole file, created or replaced (a raw upload).
    Upload {
        path: String,
        content: String,
        encoding: String,
        size: u64,
    },
    Delete {
        path: String,
        #[serde(default)]
        recursive: bool,
    },
    Move {
        src: String,
        dst: String,
    },
    Copy {
        src: String,
        dst: String,
    },
    Grep {
        pattern: String,
        #[serde(default)]
        path_pattern: Option<String>,
    },
    /// Starter files, each written only where nothing is yet, in one
    /// commit: the control plane's create sends them, and a retried create
    /// must not put back a file the agent changed.
    Seed {
        files: Vec<InitialFile>,
    },
    Search {
        pattern: String,
        #[serde(default)]
        path_pattern: Option<String>,
        #[serde(default)]
        before_context: usize,
        #[serde(default)]
        after_context: usize,
        #[serde(default)]
        offset: usize,
        #[serde(default)]
        limit: Option<usize>,
        #[serde(default)]
        max_bytes: Option<usize>,
    },
    /// Every file with its content, for a git commit of the workspace
    /// (the control plane's `/git/commit`, parity Phase B) and a fork's
    /// copy of the files; refused when
    /// the files add up to more than [`SNAPSHOT_MAX`].
    Snapshot,
}

/// The most a snapshot carries, decoded.
pub const SNAPSHOT_MAX: u64 = 64 * 1024 * 1024;

impl FsOp {
    fn reads(&self) -> bool {
        matches!(
            self,
            FsOp::Get { .. }
                | FsOp::Stat { .. }
                | FsOp::Raw { .. }
                | FsOp::Grep { .. }
                | FsOp::Search { .. }
                | FsOp::Snapshot
        )
    }
}

/// What a workspace call came to, inside [`ShardReply::Ok`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum FsResult {
    Done { value: Value },
    Failed { status: u16, message: String },
}

fn failed(status: StatusCode, message: impl Into<String>) -> FsResult {
    FsResult::Failed {
        status: status.as_u16(),
        message: message.into(),
    }
}

fn done(value: impl Serialize) -> FsResult {
    FsResult::Done {
        value: serde_json::to_value(value).unwrap_or(Value::Null),
    }
}

/// The leader's read index: everything committed before this call.
async fn read_index(State(node): State<Arc<Node>>) -> Result<Json<u64>, ApiError> {
    let (read, _) = node
        .raft
        .get_read_log_id()
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    Ok(Json(read.map_or(0, |id| id.index)))
}

/// Wait until this replica holds everything committed before the call.
pub(crate) async fn read_barrier(node: &Node) -> anyhow::Result<()> {
    let index = if node.is_leader() {
        let (read, _) = node.raft.get_read_log_id().await?;
        read.map_or(0, |id| id.index)
    } else {
        let leader = node
            .leader_addr()
            .ok_or_else(|| anyhow::anyhow!("no leader for shard {}", node.cfg.shard))?;
        node.http
            .get(format!("http://{leader}/shard/read-index"))
            .timeout(Duration::from_secs(5))
            .send()
            .await?
            .error_for_status()?
            .json::<u64>()
            .await?
    };
    tokio::time::timeout(Duration::from_secs(10), node.shared.wait_applied(index))
        .await
        .map_err(|_| anyhow::anyhow!("replica did not catch up"))
}

async fn fs(
    State(node): State<Arc<Node>>,
    Path(session): Path<String>,
    Json(op): Json<FsOp>,
) -> Result<Json<ShardReply>, ApiError> {
    let id = SessionId::parse(&session).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if op.reads() {
        read_barrier(&node)
            .await
            .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    }
    let files = node.hosting.workspace_api();
    let reply = match run(&files, id, op).await {
        Ok(result) => ShardReply::Ok {
            value: serde_json::to_value(result).unwrap_or(Value::Null),
        },
        Err(error) => match outcome(error) {
            Ok(result) => ShardReply::Ok {
                value: serde_json::to_value(result).unwrap_or(Value::Null),
            },
            Err(reply) => reply,
        },
    };
    Ok(Json(reply))
}

/// An error as the caller's failure, or as where the session is.
fn outcome(error: AgentLoopError) -> Result<FsResult, ShardReply> {
    match error {
        AgentLoopError::SessionNotFound(_) => Err(ShardReply::NotFound),
        AgentLoopError::ToolExecution(message) => Ok(failed(status_of(&message), message)),
        other => match reply_of(&other) {
            ShardReply::Refused { reason } => Ok(failed(StatusCode::SERVICE_UNAVAILABLE, reason)),
            // "not found" in a store error is not the session's absence.
            ShardReply::NotFound => Ok(failed(StatusCode::SERVICE_UNAVAILABLE, other.to_string())),
            reply => Err(reply),
        },
    }
}

/// The status the control plane's file API answers for a refusal.
fn status_of(message: &str) -> StatusCode {
    let lower = message.to_lowercase();
    if lower.contains("not found") {
        StatusCode::NOT_FOUND
    } else if lower.contains("already exists") || lower.contains("changed since") {
        StatusCode::CONFLICT
    } else if lower.contains("read-only") || lower.contains("readonly") {
        StatusCode::FORBIDDEN
    } else if lower.contains("workspace is full") {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        StatusCode::BAD_REQUEST
    }
}

fn path_of(input: &str) -> String {
    everruns_core::session_path::to_session_path(input)
}

/// Only `/_` and below is reserved, for the API's actions.
fn reserved(path: &str) -> bool {
    path == "/_" || path.starts_with("/_/")
}

fn decoded_size(content: &str, encoding: &str) -> Result<u64, FsResult> {
    SessionFile::decode_content(content, encoding)
        .map(|bytes| bytes.len() as u64)
        .map_err(|_| failed(StatusCode::BAD_REQUEST, "Invalid base64 content"))
}

async fn put(
    files: &ShardFiles,
    session: SessionId,
    change: impl FnOnce(crate::files::Content) -> Change,
    content: &str,
) -> everruns_contracts::error::Result<Value> {
    let staged = files.stage(content).await?;
    files.commit(session, vec![change(staged)]).await
}

/// Entries under `path` (all of them with `recursive`), as listed.
fn listing(
    session: SessionId,
    entries: &BTreeMap<String, Entry>,
    path: &str,
    recursive: bool,
) -> Vec<FileInfo> {
    let prefix = if path == "/" {
        "/".to_owned()
    } else {
        format!("{path}/")
    };
    entries
        .range(prefix.clone()..)
        .take_while(|(p, _)| p.starts_with(&prefix))
        .filter(|(p, _)| recursive || FileInfo::parent_path(p).as_deref() == Some(path))
        .map(|(p, e)| info(session, p, e))
        .collect()
}

async fn run(
    files: &ShardFiles,
    session: SessionId,
    op: FsOp,
) -> everruns_contracts::error::Result<FsResult> {
    Ok(match op {
        FsOp::Get { path, recursive } => {
            let path = path_of(&path);
            let entries = files.entries(session).await?;
            if path == "/" {
                return Ok(done(
                    json!({ "data": listing(session, &entries, "/", recursive) }),
                ));
            }
            match entries.get(&path) {
                None => failed(StatusCode::NOT_FOUND, "Not found"),
                Some(entry) if entry.dir => {
                    done(json!({ "data": listing(session, &entries, &path, recursive) }))
                }
                Some(entry) => done(files.file(session, &path, entry).await?),
            }
        }
        FsOp::Stat { path } => match files.stat_file(session, &path).await? {
            Some(stat) => done(stat),
            None => failed(StatusCode::NOT_FOUND, "Not found"),
        },
        FsOp::Snapshot => {
            let entries = files.entries(session).await?;
            let total: u64 = entries.values().filter(|e| !e.dir).map(|e| e.size).sum();
            if total > SNAPSHOT_MAX {
                return Ok(failed(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    format!(
                        "the workspace holds {total} bytes; a commit takes at most {SNAPSHOT_MAX}"
                    ),
                ));
            }
            let mut out = Vec::new();
            for (path, entry) in entries.iter().filter(|(_, e)| !e.dir) {
                out.push(json!({
                    "path": path,
                    "encoding": entry.encoding,
                    "content": files.content(entry).await?.unwrap_or_default(),
                    "readonly": entry.readonly,
                }));
            }
            done(json!({ "files": out }))
        }
        FsOp::Raw { path } => {
            let path = path_of(&path);
            match files.entry(session, &path).await? {
                None => failed(StatusCode::NOT_FOUND, "File not found"),
                Some(entry) if entry.dir => {
                    failed(StatusCode::BAD_REQUEST, "Cannot download a directory")
                }
                Some(entry) => done(json!({
                    "name": FileInfo::name_from_path(&path),
                    "encoding": entry.encoding,
                    "content": files.content(&entry).await?.unwrap_or_default(),
                })),
            }
        }
        FsOp::Create {
            path,
            content,
            encoding,
            readonly,
            directory,
        } => {
            let path = path_of(&path);
            if path == "/" {
                return Ok(failed(
                    StatusCode::BAD_REQUEST,
                    "Cannot create at root path, specify a path",
                ));
            }
            if reserved(&path) {
                return Ok(failed(
                    StatusCode::BAD_REQUEST,
                    "The top-level folder '_' is reserved for file actions",
                ));
            }
            if directory {
                let dir = files.create_directory(session, &path).await?;
                return Ok(done(SessionFile {
                    id: dir.id,
                    session_id: dir.session_id,
                    path: dir.path,
                    name: dir.name,
                    content: None,
                    encoding: "text".into(),
                    is_directory: true,
                    is_readonly: dir.is_readonly,
                    size_bytes: 0,
                    created_at: dir.created_at,
                    updated_at: dir.updated_at,
                }));
            }
            let encoding = encoding.unwrap_or_else(|| "text".into());
            let content = content.unwrap_or_default();
            let size = match decoded_size(&content, &encoding) {
                Ok(size) => size,
                Err(refused) => return Ok(refused),
            };
            let target = path.clone();
            put(
                files,
                session,
                move |content| Change::Create {
                    path: target,
                    encoding,
                    content,
                    size,
                    readonly,
                },
                &content,
            )
            .await?;
            done(files.written(session, &path).await?)
        }
        FsOp::Update {
            path,
            content,
            encoding,
            readonly,
            expected_content,
            expected_encoding,
        } => {
            let path = path_of(&path);
            if reserved(&path) {
                return Ok(failed(
                    StatusCode::BAD_REQUEST,
                    "The top-level folder '_' is reserved for file actions",
                ));
            }
            let Some(entry) = files.entry(session, &path).await? else {
                return Ok(failed(StatusCode::NOT_FOUND, "Not found"));
            };
            if entry.dir {
                return Ok(failed(
                    StatusCode::BAD_REQUEST,
                    format!("Cannot update directory: {path}"),
                ));
            }
            if let Some(expected) = &expected_content {
                let encoding = expected_encoding.as_deref().unwrap_or("text");
                if entry.encoding != encoding
                    || files.content(&entry).await?.as_deref() != Some(expected.as_str())
                {
                    return Ok(failed(
                        StatusCode::CONFLICT,
                        format!("file changed since it was read: {path}"),
                    ));
                }
            }
            if let Some(content) = content {
                let encoding = encoding.unwrap_or_else(|| "text".into());
                let size = match decoded_size(&content, &encoding) {
                    Ok(size) => size,
                    Err(refused) => return Ok(refused),
                };
                // Compare-and-set on the version read, so a write between
                // the check above and this commit is a conflict, not lost.
                let target = path.clone();
                let version = entry.version;
                put(
                    files,
                    session,
                    move |content| Change::Put {
                        path: target,
                        encoding,
                        content,
                        size,
                        readonly: readonly.unwrap_or(entry.readonly),
                        if_absent: false,
                        expected: Some(version),
                    },
                    &content,
                )
                .await?;
            } else if let Some(readonly) = readonly {
                files
                    .commit(
                        session,
                        vec![Change::Mark {
                            path: path.clone(),
                            readonly,
                        }],
                    )
                    .await?;
            }
            done(files.written(session, &path).await?)
        }
        FsOp::Upload {
            path,
            content,
            encoding,
            size,
        } => {
            let path = path_of(&path);
            if path == "/" || reserved(&path) {
                return Ok(failed(
                    StatusCode::BAD_REQUEST,
                    "The top-level folder '_' is reserved for file actions",
                ));
            }
            let target = path.clone();
            put(
                files,
                session,
                move |content| Change::Put {
                    path: target,
                    encoding,
                    content,
                    size,
                    readonly: false,
                    if_absent: false,
                    expected: None,
                },
                &content,
            )
            .await?;
            match files.entry(session, &path).await? {
                Some(entry) => done(info(session, &path, &entry)),
                None => failed(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "file not found after write",
                ),
            }
        }
        FsOp::Delete { path, recursive } => {
            let path = path_of(&path);
            if path == "/" {
                return Ok(failed(
                    StatusCode::BAD_REQUEST,
                    "Cannot delete root directory",
                ));
            }
            let reply = files
                .commit(session, vec![Change::Delete { path, recursive }])
                .await?;
            let result = &reply["results"][0];
            if result["read_only"] == true {
                failed(StatusCode::FORBIDDEN, "Cannot delete readonly file")
            } else if result["not_empty"] == true {
                failed(
                    StatusCode::BAD_REQUEST,
                    "Directory not empty, use recursive=true",
                )
            } else {
                done(json!({ "deleted": result["changed"] == true }))
            }
        }
        FsOp::Move { src, dst } => {
            let (from, to) = (path_of(&src), path_of(&dst));
            if reserved(&to) {
                return Ok(failed(
                    StatusCode::BAD_REQUEST,
                    "The top-level folder '_' is reserved for file actions",
                ));
            }
            files
                .commit(
                    session,
                    vec![Change::Move {
                        from,
                        to: to.clone(),
                    }],
                )
                .await?;
            done(files.written(session, &to).await?)
        }
        FsOp::Copy { src, dst } => {
            let (from, to) = (path_of(&src), path_of(&dst));
            if reserved(&to) {
                return Ok(failed(
                    StatusCode::BAD_REQUEST,
                    "The top-level folder '_' is reserved for file actions",
                ));
            }
            files
                .commit(
                    session,
                    vec![Change::Copy {
                        from,
                        to: to.clone(),
                    }],
                )
                .await?;
            done(files.written(session, &to).await?)
        }
        FsOp::Grep {
            pattern,
            path_pattern,
        } => {
            let matches = files
                .grep_files(session, &pattern, path_pattern.as_deref())
                .await?;
            done(json!({ "data": grouped(matches) }))
        }
        FsOp::Seed { files: seeds } => {
            let mut changes = Vec::with_capacity(seeds.len());
            for file in seeds {
                let path = path_of(&file.path);
                if path == "/" || reserved(&path) {
                    return Ok(failed(
                        StatusCode::BAD_REQUEST,
                        format!("not a starter file path: {}", file.path),
                    ));
                }
                let size = match decoded_size(&file.content, &file.encoding) {
                    Ok(size) => size,
                    Err(refused) => return Ok(refused),
                };
                changes.push(Change::Put {
                    path,
                    encoding: file.encoding,
                    content: files.stage(&file.content).await?,
                    size,
                    readonly: file.is_readonly,
                    if_absent: true,
                    expected: None,
                });
            }
            let count = changes.len();
            if count > 0 {
                files.commit(session, changes).await?;
            }
            done(json!({ "seeded": count }))
        }
        FsOp::Search {
            pattern,
            path_pattern,
            before_context,
            after_context,
            offset,
            limit,
            max_bytes,
        } => {
            let defaults = GrepOptions::default();
            let options = GrepOptions {
                path_pattern,
                before_context,
                after_context,
                offset,
                limit: limit.unwrap_or(defaults.limit),
                max_bytes: max_bytes.unwrap_or(defaults.max_bytes),
            };
            done(
                files
                    .grep_files_with_options(session, &pattern, &options)
                    .await?,
            )
        }
    })
}

/// Grep matches grouped by file, as the control plane lists them.
fn grouped(matches: Vec<GrepMatch>) -> Vec<Value> {
    let mut by_path: Vec<(String, Vec<GrepMatch>)> = Vec::new();
    for m in matches {
        match by_path.last_mut() {
            Some((path, list)) if *path == m.path => list.push(m),
            _ => by_path.push((m.path.clone(), vec![m])),
        }
    }
    by_path
        .into_iter()
        .map(|(path, matches)| json!({ "path": path, "matches": matches }))
        .collect()
}

#[cfg(test)]
#[path = "api_files_tests.rs"]
mod tests;
