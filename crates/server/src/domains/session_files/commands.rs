use super::queries as q;
use super::types::{
    CopyFileRequest, CreateFileRequest, DeleteResponse, GetResponse, GrepRequest, MoveFileRequest,
    StatRequest, UpdateFileRequest,
};
use super::{
    CopyFileInput, CreateDirectoryInput, CreateFileInput, GrepInput, MoveFileInput, UpdateFileInput,
};
use crate::api::common::ListResponse;
use crate::domains::common::*;
use everruns_core::events::{
    EventContext, EventRequest, FILE_OP_CREATE, FILE_OP_UPDATE, FileWrittenData,
};
use everruns_core::{FileStat, GrepResult, SessionFile};
use serde::Deserialize;
use utoipa::ToSchema;

fn map_create_error(error: anyhow::Error, create_directory: bool) -> CommandError {
    let msg = error.to_string();
    if msg.contains("already exists") {
        CommandError::conflict(msg)
    } else if msg.contains("Invalid")
        || msg.contains("cannot")
        || (create_directory && msg.contains("file exists"))
    {
        CommandError::bad_request(msg)
    } else {
        q::classify_storage(error)
    }
}

fn map_update_error(error: anyhow::Error) -> CommandError {
    let msg = error.to_string();
    if msg.contains("readonly") || msg.contains("directory") {
        CommandError::bad_request(msg)
    } else {
        q::classify_storage(error)
    }
}

fn map_delete_error(error: anyhow::Error) -> CommandError {
    let msg = error.to_string();
    if msg.contains("readonly") {
        CommandError::forbidden(msg)
    } else if msg.contains("not empty") || msg.contains("Cannot delete root") {
        CommandError::bad_request(msg)
    } else {
        q::classify_storage(error)
    }
}

fn map_move_or_copy_error(error: anyhow::Error) -> CommandError {
    let msg = error.to_string();
    if msg.contains("not found") {
        CommandError::not_found("Source")
    } else if msg.contains("already exists") {
        CommandError::conflict(msg)
    } else if msg.contains("Invalid") || msg.contains("Cannot copy") {
        CommandError::bad_request(msg)
    } else {
        q::classify_storage(error)
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionFiles {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(default)]
    pub recursive: bool,
}

impl Command for ListSessionFiles {
    type Output = GetResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_files",
            category: "files",
            description: "Get the root directory listing of session files.",
            method: "GET",
            path: "/v1/sessions/{session_id}/fs",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<GetResponse, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session(ctx, session_id).await?;

        let files = if self.recursive {
            q::service(ctx)
                .list_all(session_id.uuid())
                .await
                .map_err(q::classify_storage)?
        } else {
            q::service(ctx)
                .list_directory(session_id.uuid(), "/")
                .await
                .map_err(q::classify_storage)?
        };
        Ok(GetResponse::Listing(ListResponse::new(files)))
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessionFiles>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetSessionFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

impl Command for GetSessionFile {
    type Output = GetResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_session_file",
            category: "files",
            description: "Get a file or directory at a path in the session filesystem.",
            method: "GET",
            path: "/v1/sessions/{session_id}/fs/{path}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<GetResponse, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session(ctx, session_id).await?;
        let path = q::normalize_path(&self.path);
        let stat = q::service(ctx)
            .stat(session_id.uuid(), &path)
            .await
            .map_err(q::classify_storage)?;

        match stat {
            Some(s) if s.is_directory => {
                let files = if self.recursive {
                    q::service(ctx)
                        .list_all(session_id.uuid())
                        .await
                        .map_err(q::classify_storage)?
                } else {
                    q::service(ctx)
                        .list_directory(session_id.uuid(), &path)
                        .await
                        .map_err(q::classify_storage)?
                };
                Ok(GetResponse::Listing(ListResponse::new(files)))
            }
            Some(_) => {
                let file = q::service(ctx)
                    .read_file(session_id.uuid(), &path)
                    .await
                    .map_err(q::classify_storage)?
                    .ok_or_else(|| CommandError::not_found("File"))?;
                Ok(GetResponse::File(file))
            }
            None if path == "/" => Ok(GetResponse::Listing(ListResponse::new(vec![]))),
            None => Err(CommandError::not_found("Path")),
        }
    }
}

inventory::submit! { CommandDescriptor::of::<GetSessionFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSessionFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub path: String,
    #[serde(flatten)]
    pub req: CreateFileRequest,
}

impl Command for CreateSessionFile {
    type Output = SessionFile;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_session_file",
            category: "files",
            description: "Create a file or directory in the session filesystem.",
            method: "POST",
            path: "/v1/sessions/{session_id}/fs/{path}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionFile, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session(ctx, session_id).await?;
        let path = q::normalize_path(&self.path);
        if q::is_reserved_path(&path) {
            return Err(CommandError::bad_request(
                "Paths starting with '_' are reserved for system actions",
            ));
        }

        let file = if self.req.is_directory.unwrap_or(false) {
            let dir = q::service(ctx)
                .create_directory(session_id.uuid(), CreateDirectoryInput { path })
                .await
                .map_err(|e| map_create_error(e, true))?;
            SessionFile {
                id: dir.id,
                session_id: dir.session_id,
                path: dir.path,
                name: dir.name,
                content: None,
                encoding: "text".to_string(),
                is_directory: true,
                is_readonly: dir.is_readonly,
                size_bytes: 0,
                created_at: dir.created_at,
                updated_at: dir.updated_at,
            }
        } else {
            q::service(ctx)
                .create_file(
                    session_id.uuid(),
                    CreateFileInput {
                        path,
                        content: self.req.content,
                        encoding: self.req.encoding,
                        is_readonly: self.req.is_readonly,
                    },
                )
                .await
                .map_err(|e| map_create_error(e, false))?
        };

        if !file.is_directory
            && let Some(event_service) = &ctx.event_service
        {
            let event = EventRequest::new(
                session_id,
                EventContext::empty(),
                FileWrittenData {
                    path: file.path.clone(),
                    operation: FILE_OP_CREATE.into(),
                    size_bytes: file.size_bytes,
                    created: true,
                },
            );
            if let Err(error) = event_service.emit(event).await {
                tracing::warn!(error = %error, path = %file.path, "Failed to emit file.written event");
            }
        }

        Ok(file)
    }
}

inventory::submit! { CommandDescriptor::of::<CreateSessionFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSessionFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub path: String,
    #[serde(flatten)]
    pub req: UpdateFileRequest,
}

impl Command for UpdateSessionFile {
    type Output = SessionFile;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_session_file",
            category: "files",
            description: "Update a file in the session filesystem.",
            method: "PUT",
            path: "/v1/sessions/{session_id}/fs/{path}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionFile, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session(ctx, session_id).await?;
        let path = q::normalize_path(&self.path);
        if q::is_reserved_path(&path) {
            return Err(CommandError::bad_request(
                "Paths starting with '_' are reserved for system actions",
            ));
        }

        let file = q::service(ctx)
            .update_file(
                session_id.uuid(),
                &path,
                UpdateFileInput {
                    content: self.req.content,
                    encoding: self.req.encoding,
                    is_readonly: self.req.is_readonly,
                },
            )
            .await
            .map_err(map_update_error)?
            .ok_or_else(|| CommandError::not_found("File"))?;

        if let Some(event_service) = &ctx.event_service {
            let event = EventRequest::new(
                session_id,
                EventContext::empty(),
                FileWrittenData {
                    path: file.path.clone(),
                    operation: FILE_OP_UPDATE.into(),
                    size_bytes: file.size_bytes,
                    created: false,
                },
            );
            if let Err(error) = event_service.emit(event).await {
                tracing::warn!(error = %error, path = %file.path, "Failed to emit file.written event");
            }
        }

        Ok(file)
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateSessionFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteSessionFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

impl Command for DeleteSessionFile {
    type Output = DeleteResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_session_file",
            category: "files",
            description: "Delete a file or directory in the session filesystem.",
            method: "DELETE",
            path: "/v1/sessions/{session_id}/fs/{path}",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<DeleteResponse, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session(ctx, session_id).await?;
        let path = q::normalize_path(&self.path);
        let deleted = q::service(ctx)
            .delete(session_id.uuid(), &path, self.recursive)
            .await
            .map_err(map_delete_error)?;
        Ok(DeleteResponse { deleted })
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteSessionFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct MoveSessionFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(flatten)]
    pub req: MoveFileRequest,
}

impl Command for MoveSessionFile {
    type Output = SessionFile;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "move_session_file",
            category: "files",
            description: "Move or rename a file in the session filesystem.",
            method: "POST",
            path: "/v1/sessions/{session_id}/fs/_/move",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionFile, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session(ctx, session_id).await?;
        q::service(ctx)
            .move_file(
                session_id.uuid(),
                MoveFileInput {
                    src_path: self.req.src_path,
                    dst_path: self.req.dst_path,
                },
            )
            .await
            .map_err(map_move_or_copy_error)?
            .ok_or_else(|| CommandError::not_found("Source"))
    }
}

inventory::submit! { CommandDescriptor::of::<MoveSessionFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CopySessionFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(flatten)]
    pub req: CopyFileRequest,
}

impl Command for CopySessionFile {
    type Output = SessionFile;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "copy_session_file",
            category: "files",
            description: "Copy a file in the session filesystem.",
            method: "POST",
            path: "/v1/sessions/{session_id}/fs/_/copy",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<SessionFile, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session(ctx, session_id).await?;
        q::service(ctx)
            .copy_file(
                session_id.uuid(),
                CopyFileInput {
                    src_path: self.req.src_path,
                    dst_path: self.req.dst_path,
                },
            )
            .await
            .map_err(map_move_or_copy_error)?
            .ok_or_else(|| CommandError::not_found("Source"))
    }
}

inventory::submit! { CommandDescriptor::of::<CopySessionFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GrepSessionFiles {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(flatten)]
    pub req: GrepRequest,
}

impl Command for GrepSessionFiles {
    type Output = Vec<GrepResult>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "grep_session_files",
            category: "files",
            description: "Search files in the session filesystem.",
            method: "POST",
            path: "/v1/sessions/{session_id}/fs/_/grep",
        }
    }

    fn read_only() -> bool {
        true
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<GrepResult>, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session(ctx, session_id).await?;
        q::service(ctx)
            .grep(
                session_id.uuid(),
                GrepInput {
                    pattern: self.req.pattern,
                    path_pattern: self.req.path_pattern,
                },
            )
            .await
            .map_err(|error| {
                let msg = error.to_string();
                if msg.contains("regex") || msg.contains("pattern") {
                    CommandError::bad_request(format!("Invalid regex: {msg}"))
                } else {
                    q::classify_storage(error)
                }
            })
    }
}

inventory::submit! { CommandDescriptor::of::<GrepSessionFiles>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct StatSessionFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(flatten)]
    pub req: StatRequest,
}

impl Command for StatSessionFile {
    type Output = FileStat;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "stat_session_file",
            category: "files",
            description: "Get file metadata in the session filesystem.",
            method: "POST",
            path: "/v1/sessions/{session_id}/fs/_/stat",
        }
    }

    fn read_only() -> bool {
        true
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<FileStat, CommandError> {
        let session_id = q::parse_session_id(&self.session_id)?;
        q::verify_session(ctx, session_id).await?;
        let path = q::normalize_path(&self.req.path);
        q::service(ctx)
            .stat(session_id.uuid(), &path)
            .await
            .map_err(q::classify_storage)?
            .ok_or_else(|| CommandError::not_found("Path"))
    }
}

inventory::submit! { CommandDescriptor::of::<StatSessionFile>() }
