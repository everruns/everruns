//! The session file surface on the command transport.
//!
//! Its own file rather than another block in `grpc_adapters.rs`, which is on the
//! source-size ratchet's debt list and may not grow, and because this mirrors
//! `grpc_sqldb_adapter`: one `SessionFileSystem` implementation expressed
//! entirely as `session_files` domain commands, the same ones the HTTP API and
//! MCP callers use.
//!
//! Before this, the worker reached files through eight bespoke gRPC RPCs that
//! duplicated those commands' logic server-side. The commands verify session
//! ownership, resolve the session's *workspace* as the store key, and apply the
//! private `/memory/user` rules in one place; the RPCs re-implemented the first
//! two and skipped the third.

use crate::grpc_adapters::GrpcAdapter;
use async_trait::async_trait;
use everruns_core::session_files::SessionFileSystem;
use everruns_core::{FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, SessionFile};
use everruns_internal_protocol::proto;
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::SessionId;
use serde_json::{Value, json};

/// The surface name carried into `require_org`'s error when this adapter has no
/// org — the cross-org sweeper context, which never serves a session's files.
const SURFACE: &str = "Session files";

/// `base64` is the only encoding the store decodes; everything else is stored as
/// raw bytes and recorded as the default.
///
/// The RPC this replaces dropped the caller's encoding on both its create and
/// update paths, so a `base64` write was stored undecoded — broken, but unused,
/// since every caller passes `text` or `utf-8`. Forwarding only `base64` keeps
/// those callers' stored `encoding` exactly as it is today and fixes the binary
/// case rather than carrying the bug across.
fn encoding_param(encoding: &str) -> Option<&'static str> {
    (encoding == "base64").then_some("base64")
}

fn kind_of(error: &proto::CommandError) -> Option<proto::command_error::Kind> {
    proto::command_error::Kind::try_from(error.kind).ok()
}

fn is_not_found(error: &proto::CommandError) -> bool {
    matches!(kind_of(error), Some(proto::command_error::Kind::NotFound))
}

/// A command failure the file surface has no softer answer for.
fn command_failure(operation: &str, error: proto::CommandError) -> AgentLoopError {
    AgentLoopError::store(format!("{operation}: {}", error.message))
}

fn decode<T: serde::de::DeserializeOwned>(operation: &str, value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| {
        AgentLoopError::store(format!("{operation} returned unexpected shape: {error}"))
    })
}

/// `get_workspace_file` and `list_workspace_files` answer this untagged union:
/// one file, or a listing of entries.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum GetResponse {
    File(Box<SessionFile>),
    Listing { data: Vec<FileInfo> },
}

impl GrpcAdapter {
    /// Run a `session_files` command, keeping the command-error channel intact
    /// so callers can tell a not-found from a genuine failure.
    async fn files_command(
        &self,
        name: &str,
        params: Value,
    ) -> Result<std::result::Result<Value, proto::CommandError>> {
        self.execute_session_command(SURFACE, name, params).await
    }
}

#[async_trait]
impl SessionFileSystem for GrpcAdapter {
    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let result = self
            .files_command(
                "get_workspace_file",
                json!({ "session_id": session_id.to_string(), "path": path }),
            )
            .await?;

        match result {
            Ok(value) => match decode::<GetResponse>("get_workspace_file", value)? {
                GetResponse::File(file) => Ok(Some(*file)),
                // A directory: `read_file` answers for files only.
                GetResponse::Listing { .. } => Ok(None),
            },
            Err(error) if is_not_found(&error) => Ok(None),
            Err(error) => Err(command_failure("read file", error)),
        }
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        // An upsert, as the RPC was: update first, create when there is nothing
        // to update. The RPC read the file first to decide; this reaches the
        // same answer in one round trip on the common path.
        let mut params = json!({
            "session_id": session_id.to_string(),
            "path": path,
            "content": content,
        });
        if let Some(encoding) = encoding_param(encoding) {
            params["encoding"] = json!(encoding);
        }

        match self
            .files_command("update_workspace_file", params.clone())
            .await?
        {
            Ok(value) => decode("update_workspace_file", value),
            Err(error) if is_not_found(&error) => {
                match self.files_command("create_workspace_file", params).await? {
                    Ok(value) => decode("create_workspace_file", value),
                    Err(error) => Err(command_failure("create file", error)),
                }
            }
            Err(error) => Err(command_failure("write file", error)),
        }
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
        let mut params = json!({
            "session_id": session_id.to_string(),
            "path": path,
            "content": content,
            "expected_content": expected_content,
        });
        if let Some(encoding) = encoding_param(encoding) {
            params["encoding"] = json!(encoding);
        }
        if let Some(encoding) = encoding_param(expected_encoding) {
            params["expected_encoding"] = json!(encoding);
        }

        match self.files_command("update_workspace_file", params).await? {
            Ok(value) => decode("update_workspace_file", value).map(Some),
            // Both "the content moved under you" and "there is nothing there"
            // mean the same thing to this caller: the write did not happen.
            Err(error)
                if is_not_found(&error)
                    || matches!(kind_of(&error), Some(proto::command_error::Kind::Conflict)) =>
            {
                Ok(None)
            }
            Err(error) => Err(command_failure("conditional write", error)),
        }
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        #[derive(serde::Deserialize)]
        struct DeleteResponse {
            deleted: bool,
        }

        match self
            .files_command(
                "delete_workspace_file",
                json!({ "session_id": session_id.to_string(), "path": path, "recursive": recursive }),
            )
            .await?
        {
            Ok(value) => Ok(decode::<DeleteResponse>("delete_workspace_file", value)?.deleted),
            Err(error) if is_not_found(&error) => Ok(false),
            Err(error) => Err(command_failure("delete file", error)),
        }
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        match self
            .files_command(
                "get_workspace_file",
                json!({ "session_id": session_id.to_string(), "path": path }),
            )
            .await?
        {
            Ok(value) => match decode::<GetResponse>("get_workspace_file", value)? {
                GetResponse::Listing { data } => Ok(data),
                // A file, not a directory: nothing to list.
                GetResponse::File(_) => Ok(Vec::new()),
            },
            Err(error) if is_not_found(&error) => Ok(Vec::new()),
            Err(error) => Err(command_failure("list directory", error)),
        }
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        match self
            .files_command(
                "stat_workspace_file",
                json!({ "session_id": session_id.to_string(), "path": path }),
            )
            .await?
        {
            Ok(value) => decode("stat_workspace_file", value).map(Some),
            Err(error) if is_not_found(&error) => Ok(None),
            Err(error) => Err(command_failure("stat file", error)),
        }
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        #[derive(serde::Deserialize)]
        struct GrepResult {
            matches: Vec<GrepMatch>,
        }

        let mut params = json!({ "session_id": session_id.to_string(), "pattern": pattern });
        if let Some(path_pattern) = path_pattern {
            params["path_pattern"] = json!(path_pattern);
        }

        match self.files_command("grep_workspace_files", params).await? {
            Ok(value) => {
                let grouped: Vec<GrepResult> = decode("grep_workspace_files", value)?;
                Ok(grouped.into_iter().flat_map(|r| r.matches).collect())
            }
            Err(error) => Err(command_failure("grep files", error)),
        }
    }

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        let defaults = GrepOptions::default();
        let mut params = json!({
            "session_id": session_id.to_string(),
            "pattern": pattern,
            "before_context": options.before_context,
            "after_context": options.after_context,
            "offset": options.offset,
        });
        if let Some(path_pattern) = &options.path_pattern {
            params["path_pattern"] = json!(path_pattern);
        }
        // Only send the bounds the caller actually narrowed; the command
        // applies the same service defaults this adapter would.
        if options.limit != defaults.limit {
            params["limit"] = json!(options.limit);
        }
        if options.max_bytes != defaults.max_bytes {
            params["max_bytes"] = json!(options.max_bytes);
        }

        match self.files_command("search_workspace_files", params).await? {
            Ok(value) => decode("search_workspace_files", value),
            Err(error) => Err(command_failure("search files", error)),
        }
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        match self
            .files_command(
                "create_workspace_file",
                json!({
                    "session_id": session_id.to_string(),
                    "path": path,
                    "is_directory": true,
                }),
            )
            .await?
        {
            Ok(value) => {
                let file: SessionFile = decode("create_workspace_file", value)?;
                Ok(FileInfo {
                    id: file.id,
                    session_id: file.session_id,
                    path: file.path,
                    name: file.name,
                    size_bytes: file.size_bytes,
                    is_directory: file.is_directory,
                    is_readonly: file.is_readonly,
                    created_at: file.created_at,
                    updated_at: file.updated_at,
                })
            }
            Err(error) => Err(command_failure("create directory", error)),
        }
    }
}
