use super::queries as q;
use super::types::{
    CopyFileRequest, CreateFileRequest, DeleteResponse, GetResponse, GrepRequest, MoveFileRequest,
    SearchRequest, StatRequest, UpdateFileRequest,
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
pub struct ListWorkspaceFiles {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(default)]
    pub recursive: bool,
}

impl Command for ListWorkspaceFiles {
    type Output = GetResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_workspace_files",
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
        let access = q::verify_session(ctx, session_id).await?;

        let mut files = if self.recursive {
            q::service(ctx)
                .list_all(access.workspace_key)
                .await
                .map_err(q::classify_storage)?
        } else {
            q::service(ctx)
                .list_directory(access.workspace_key, "/")
                .await
                .map_err(q::classify_storage)?
        };
        if !access.user_memory_allowed {
            files = q::redact_user_memory_files(files, |file| &file.path);
        }
        Ok(GetResponse::Listing(ListResponse::new(files)))
    }
}

inventory::submit! { CommandDescriptor::of::<ListWorkspaceFiles>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetWorkspaceFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

impl Command for GetWorkspaceFile {
    type Output = GetResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_workspace_file",
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
        let access = q::verify_session(ctx, session_id).await?;
        let path = q::normalize_path(&self.path);
        access.ensure_user_memory_access(&path)?;
        let stat = q::service(ctx)
            .stat(access.workspace_key, &path)
            .await
            .map_err(q::classify_storage)?;

        match stat {
            Some(s) if s.is_directory => {
                let mut files = if self.recursive {
                    q::service(ctx)
                        .list_all(access.workspace_key)
                        .await
                        .map_err(q::classify_storage)?
                } else {
                    q::service(ctx)
                        .list_directory(access.workspace_key, &path)
                        .await
                        .map_err(q::classify_storage)?
                };
                if self.recursive && !access.user_memory_allowed {
                    files = q::redact_user_memory_files(files, |file| &file.path);
                }
                Ok(GetResponse::Listing(ListResponse::new(files)))
            }
            Some(_) => {
                let file = q::service(ctx)
                    .read_file(access.workspace_key, &path)
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

inventory::submit! { CommandDescriptor::of::<GetWorkspaceFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWorkspaceFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub path: String,
    #[serde(flatten)]
    pub req: CreateFileRequest,
}

impl Command for CreateWorkspaceFile {
    type Output = SessionFile;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_workspace_file",
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
        let workspace_key = q::verify_session_for_write(ctx, session_id).await?;
        let path = q::normalize_path(&self.path);
        let access = q::verify_session(ctx, session_id).await?;
        access.ensure_user_memory_mutation_access(&path)?;
        if q::is_reserved_path(&path) {
            return Err(CommandError::bad_request(
                "Paths starting with '_' are reserved for system actions",
            ));
        }

        let file = if self.req.is_directory.unwrap_or(false) {
            let dir = q::service(ctx)
                .create_directory(workspace_key, CreateDirectoryInput { path })
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
                    workspace_key,
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

inventory::submit! { CommandDescriptor::of::<CreateWorkspaceFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateWorkspaceFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub path: String,
    #[serde(flatten)]
    pub req: UpdateFileRequest,
}

impl Command for UpdateWorkspaceFile {
    type Output = SessionFile;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_workspace_file",
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
        let workspace_key = q::verify_session_for_write(ctx, session_id).await?;
        let path = q::normalize_path(&self.path);
        let access = q::verify_session(ctx, session_id).await?;
        access.ensure_user_memory_mutation_access(&path)?;
        if q::is_reserved_path(&path) {
            return Err(CommandError::bad_request(
                "Paths starting with '_' are reserved for system actions",
            ));
        }

        // A compare-and-swap write when the caller states what it expects to
        // find. `update_file_if_content_matches` answers `None` both for "no
        // such file" and "content moved under you"; only the latter is a
        // conflict, so the existence check comes first and the two stay
        // distinguishable to the caller.
        let file = if let Some(expected_content) = self.req.expected_content {
            let content = self.req.content.ok_or_else(|| {
                CommandError::bad_request("expected_content requires content to write")
            })?;
            let service = q::service(ctx);
            if service
                .stat(workspace_key, &path)
                .await
                .map_err(q::classify_storage)?
                .is_none()
            {
                return Err(CommandError::not_found("File"));
            }
            service
                .update_file_if_content_matches(
                    workspace_key,
                    &path,
                    &expected_content,
                    self.req.expected_encoding.as_deref().unwrap_or("text"),
                    &content,
                    self.req.encoding.as_deref().unwrap_or("text"),
                )
                .await
                .map_err(map_update_error)?
                .ok_or_else(|| {
                    CommandError::conflict(
                        "File content changed since it was read; re-read and retry",
                    )
                })?
        } else {
            q::service(ctx)
                .update_file(
                    workspace_key,
                    &path,
                    UpdateFileInput {
                        content: self.req.content,
                        encoding: self.req.encoding,
                        is_readonly: self.req.is_readonly,
                    },
                )
                .await
                .map_err(map_update_error)?
                .ok_or_else(|| CommandError::not_found("File"))?
        };

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

inventory::submit! { CommandDescriptor::of::<UpdateWorkspaceFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteWorkspaceFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
}

impl Command for DeleteWorkspaceFile {
    type Output = DeleteResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_workspace_file",
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
        let workspace_key = q::verify_session_for_write(ctx, session_id).await?;
        let path = q::normalize_path(&self.path);
        let access = q::verify_session(ctx, session_id).await?;
        access.ensure_user_memory_mutation_access(&path)?;
        let deleted = q::service(ctx)
            .delete(workspace_key, &path, self.recursive)
            .await
            .map_err(map_delete_error)?;
        Ok(DeleteResponse { deleted })
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteWorkspaceFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct MoveWorkspaceFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(flatten)]
    pub req: MoveFileRequest,
}

impl Command for MoveWorkspaceFile {
    type Output = SessionFile;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "move_workspace_file",
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
        let workspace_key = q::verify_session_for_write(ctx, session_id).await?;
        let src_path = q::normalize_path(&self.req.src_path);
        let dst_path = q::normalize_path(&self.req.dst_path);
        let access = q::verify_session(ctx, session_id).await?;
        access.ensure_user_memory_mutation_access(&src_path)?;
        access.ensure_user_memory_mutation_access(&dst_path)?;
        q::service(ctx)
            .move_file(workspace_key, MoveFileInput { src_path, dst_path })
            .await
            .map_err(map_move_or_copy_error)?
            .ok_or_else(|| CommandError::not_found("Source"))
    }
}

inventory::submit! { CommandDescriptor::of::<MoveWorkspaceFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CopyWorkspaceFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(flatten)]
    pub req: CopyFileRequest,
}

impl Command for CopyWorkspaceFile {
    type Output = SessionFile;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "copy_workspace_file",
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
        let workspace_key = q::verify_session_for_write(ctx, session_id).await?;
        let src_path = q::normalize_path(&self.req.src_path);
        let dst_path = q::normalize_path(&self.req.dst_path);
        let access = q::verify_session(ctx, session_id).await?;
        access.ensure_user_memory_mutation_access(&src_path)?;
        access.ensure_user_memory_mutation_access(&dst_path)?;
        q::service(ctx)
            .copy_file(workspace_key, CopyFileInput { src_path, dst_path })
            .await
            .map_err(map_move_or_copy_error)?
            .ok_or_else(|| CommandError::not_found("Source"))
    }
}

inventory::submit! { CommandDescriptor::of::<CopyWorkspaceFile>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GrepWorkspaceFiles {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(flatten)]
    pub req: GrepRequest,
}

impl Command for GrepWorkspaceFiles {
    type Output = Vec<GrepResult>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "grep_workspace_files",
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
        let access = q::verify_session(ctx, session_id).await?;
        let results = q::service(ctx)
            .grep(
                access.workspace_key,
                GrepInput {
                    pattern: self.req.pattern,
                    path_pattern: self.req.path_pattern,
                    excluded_path_prefix: (!access.user_memory_allowed)
                        .then(|| q::USER_MEMORY_MOUNT_PATH.to_string()),
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
            })?;
        // Defense in depth: storage already excludes private memory before
        // matching or scan accounting, but keep response redaction here too.
        let results = if access.user_memory_allowed {
            results
        } else {
            q::redact_user_memory_files(results, |result| &result.path)
        };
        Ok(results)
    }
}

inventory::submit! { CommandDescriptor::of::<GrepWorkspaceFiles>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchWorkspaceFiles {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(flatten)]
    pub req: SearchRequest,
}

impl Command for SearchWorkspaceFiles {
    type Output = everruns_core::GrepSearchResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "search_workspace_files",
            category: "files",
            description: "Search files in the session filesystem, with surrounding context and paging.",
            method: "POST",
            path: "/v1/sessions/{session_id}/fs/_/search",
        }
    }

    fn read_only() -> bool {
        true
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&crate::domains::sessions::SESSION_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<everruns_core::GrepSearchResult, CommandError> {
        use everruns_core::session_files::SessionFileSystem;

        let session_id = q::parse_session_id(&self.session_id)?;
        let access = q::verify_session(ctx, session_id).await?;

        let defaults = everruns_core::GrepOptions::default();
        let options = everruns_core::GrepOptions {
            path_pattern: self.req.path_pattern,
            before_context: self.req.before_context,
            after_context: self.req.after_context,
            offset: self.req.offset,
            limit: self.req.limit.unwrap_or(defaults.limit),
            max_bytes: self.req.max_bytes.unwrap_or(defaults.max_bytes),
        };

        let mut result = q::service(ctx)
            .grep_files_with_options(
                everruns_provider::typed_id::SessionId::from_uuid(access.workspace_key),
                &self.req.pattern,
                &options,
            )
            .await
            .map_err(|error| {
                let msg = error.to_string();
                if msg.contains("regex") || msg.contains("pattern") {
                    CommandError::bad_request(format!("Invalid regex: {msg}"))
                } else {
                    tracing::error!("Session file search error: {error}");
                    CommandError::internal(anyhow::anyhow!("{error}"))
                }
            })?;

        // The private memory mount is redacted from both projections; the
        // counts stay as the search reported them, so paging is unaffected by
        // what this caller may not see.
        if !access.user_memory_allowed {
            result.matches = q::redact_user_memory_files(result.matches, |m| &m.path);
            result.blocks = q::redact_user_memory_files(result.blocks, |b| &b.path);
        }

        Ok(result)
    }
}

inventory::submit! { CommandDescriptor::of::<SearchWorkspaceFiles>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct StatWorkspaceFile {
    /// Session's prefixed public identifier.
    pub session_id: String,
    #[serde(flatten)]
    pub req: StatRequest,
}

impl Command for StatWorkspaceFile {
    type Output = FileStat;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "stat_workspace_file",
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
        let access = q::verify_session(ctx, session_id).await?;
        let path = q::normalize_path(&self.req.path);
        access.ensure_user_memory_access(&path)?;
        q::service(ctx)
            .stat(access.workspace_key, &path)
            .await
            .map_err(q::classify_storage)?
            .ok_or_else(|| CommandError::not_found("Path"))
    }
}

inventory::submit! { CommandDescriptor::of::<StatWorkspaceFile>() }

#[cfg(test)]
mod tests {
    use super::{
        CreateWorkspaceFile, GetWorkspaceFile, GrepWorkspaceFiles, SearchWorkspaceFiles,
        UpdateWorkspaceFile,
    };
    use crate::domains::common::{Command, CommandErrorKind, Ctx};
    use crate::domains::session_files::queries as q;
    use crate::domains::session_files::types::{
        CreateFileRequest, GrepRequest, SearchRequest, UpdateFileRequest,
    };
    use crate::services::CapabilityService;
    use crate::storage::StorageBackend;
    use crate::storage::models::{CreateSessionRow, CreateWorkspaceRow};
    use everruns_core::{
        Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole, Permission, PermissionResolver,
    };
    use everruns_provider::typed_id::PrincipalId;
    use serde_json::json;
    use std::sync::Arc;
    use uuid::Uuid;

    /// Grants only `OrgSessionsManage` — models a caller authorized for session
    /// management but not for managing shared workspaces (`OrgSettingsManage`).
    struct SessionsManageOnlyResolver;

    impl PermissionResolver for SessionsManageOnlyResolver {
        fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
            matches!(permission, Permission::OrgSessionsManage)
        }
        fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
            vec![Permission::OrgSessionsManage]
        }
    }

    fn session_row(workspace_id: Option<Uuid>) -> CreateSessionRow {
        CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            org_id: DEFAULT_ORG_ID,
            app_id: None,
            endpoint_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            title: Some("session-fs write guard".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            system_prompt: None,
            initial_files: json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
            workspace_id,
        }
    }

    fn file_req(content: &str) -> CreateFileRequest {
        CreateFileRequest {
            content: Some(content.to_string()),
            encoding: None,
            is_readonly: None,
            is_directory: None,
        }
    }

    fn owner_caller() -> Caller {
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(Uuid::nil()),
            role: OrgRole::Owner,
            is_platform_user: false,
            is_internal: false,
        }
    }

    #[tokio::test]
    async fn legacy_session_file_read_rejects_other_users_private_memory() {
        let db = Arc::new(StorageBackend::in_memory());
        let owner_user_id = Uuid::new_v4();
        let mut row = session_row(None);
        row.resolved_owner_user_id = Some(owner_user_id);
        let session = db.create_session(row).await.expect("create session");
        let file_service = crate::domains::session_files::WorkspaceFileService::new(db.clone());
        file_service
            .create_file(
                session.workspace_id,
                crate::domains::session_files::CreateFileInput {
                    path: "/memory/user/secret.md".to_string(),
                    content: Some("private".to_string()),
                    encoding: None,
                    is_readonly: None,
                },
            )
            .await
            .expect("seed private user memory file");

        let mut caller = owner_caller();
        caller.user_id = Some(Uuid::new_v4());
        let ctx = Ctx::minimal_for_test(caller, db, None);

        let err = GetWorkspaceFile {
            session_id: session.id.to_string(),
            path: "/memory/user/secret.md".to_string(),
            recursive: false,
        }
        .run(&ctx)
        .await
        .expect_err("other users must not read private user memory");
        assert!(matches!(err.kind, CommandErrorKind::Forbidden(_)));
    }

    /// Grep must stay available to non-owners (and unresolved callers such as
    /// no-auth/dev), returning workspace matches while redacting any hits under
    /// the private `/memory/user` subtree rather than failing the whole scan.
    #[tokio::test]
    async fn grep_redacts_private_memory_for_non_owner() {
        let db = Arc::new(StorageBackend::in_memory());
        let owner_user_id = Uuid::new_v4();
        let mut row = session_row(None);
        row.resolved_owner_user_id = Some(owner_user_id);
        let session = db.create_session(row).await.expect("create session");
        let file_service = crate::domains::session_files::WorkspaceFileService::new(db.clone());
        file_service
            .create_file(
                session.workspace_id,
                crate::domains::session_files::CreateFileInput {
                    path: "/notes.txt".to_string(),
                    content: Some("needle in workspace".to_string()),
                    encoding: None,
                    is_readonly: None,
                },
            )
            .await
            .expect("seed workspace file");
        file_service
            .create_file(
                session.workspace_id,
                crate::domains::session_files::CreateFileInput {
                    path: "/memory/user/secret.md".to_string(),
                    content: Some("needle in private memory".to_string()),
                    encoding: None,
                    is_readonly: None,
                },
            )
            .await
            .expect("seed private user memory file");

        // Caller is not the resolved owner -> user memory must be redacted.
        let mut caller = owner_caller();
        caller.user_id = Some(Uuid::new_v4());
        let ctx = Ctx::minimal_for_test(caller, db, None);

        let results = GrepWorkspaceFiles {
            session_id: session.id.to_string(),
            req: GrepRequest {
                pattern: "needle".to_string(),
                path_pattern: None,
            },
        }
        .run(&ctx)
        .await
        .expect("grep must remain available to non-owners");

        assert!(
            results.iter().any(|r| r.path == "/notes.txt"),
            "workspace match must be returned"
        );
        assert!(
            !results.iter().any(|r| q::is_user_memory_path(&r.path)),
            "private /memory/user matches must be redacted"
        );
    }

    #[tokio::test]
    async fn legacy_session_file_read_allows_owner_private_memory() {
        let db = Arc::new(StorageBackend::in_memory());
        let owner_user_id = Uuid::new_v4();
        let mut row = session_row(None);
        row.resolved_owner_user_id = Some(owner_user_id);
        let session = db.create_session(row).await.expect("create session");
        let file_service = crate::domains::session_files::WorkspaceFileService::new(db.clone());
        file_service
            .create_file(
                session.workspace_id,
                crate::domains::session_files::CreateFileInput {
                    path: "/memory/user/secret.md".to_string(),
                    content: Some("private".to_string()),
                    encoding: None,
                    is_readonly: None,
                },
            )
            .await
            .expect("seed private user memory file");

        let mut caller = owner_caller();
        caller.user_id = Some(owner_user_id);
        let ctx = Ctx::minimal_for_test(caller, db, None);

        GetWorkspaceFile {
            session_id: session.id.to_string(),
            path: "/memory/user/secret.md".to_string(),
            recursive: false,
        }
        .run(&ctx)
        .await
        .expect("owner can read private user memory");
    }

    /// Active-status gate: once a workspace is archived, legacy session-fs
    /// writes must be rejected (archived workspaces are read-only).
    #[tokio::test]
    async fn legacy_session_file_write_rejects_archived_workspace() {
        let db = Arc::new(StorageBackend::in_memory());
        let session = db
            .create_session(session_row(None))
            .await
            .expect("create session");
        let workspace_id = session.workspace_id;
        let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db.clone(), None);

        CreateWorkspaceFile {
            session_id: session.id.to_string(),
            path: "before.txt".to_string(),
            req: file_req("before archive"),
        }
        .execute(&ctx)
        .await
        .expect("write allowed while workspace is active");

        db.archive_workspace(DEFAULT_ORG_ID, workspace_id)
            .await
            .expect("archive workspace");

        let err = CreateWorkspaceFile {
            session_id: session.id.to_string(),
            path: "after.txt".to_string(),
            req: file_req("after archive"),
        }
        .execute(&ctx)
        .await
        .expect_err("write must be rejected after archive");
        assert!(matches!(err.kind, CommandErrorKind::Forbidden(_)));
    }

    /// Authorization parity: writing to an attached *shared* workspace requires
    /// `WORKSPACE_MANAGE`, not just the session policy. A caller with only
    /// `OrgSessionsManage` must be denied.
    #[tokio::test]
    async fn legacy_session_write_to_shared_workspace_requires_workspace_manage() {
        let db = Arc::new(StorageBackend::in_memory());
        let capability_service = Arc::new(CapabilityService::new(db.clone(), None));

        let workspace = db
            .create_workspace(
                DEFAULT_ORG_ID,
                CreateWorkspaceRow {
                    id: None,
                    public_id: format!("workspace_{:032x}", 1u128),
                    name: "shared".to_string(),
                    description: None,
                    owner_principal_id: None,
                    resolved_owner_user_id: None,
                },
            )
            .await
            .expect("create shared workspace");
        let session = db
            .create_session(session_row(Some(workspace.id)))
            .await
            .expect("create session attached to shared workspace");
        assert_ne!(
            session.id.uuid(),
            workspace.id,
            "session must attach to a shared (non-1:1) workspace"
        );

        let ctx = Ctx::new(
            owner_caller(),
            db,
            capability_service,
            None,
            Arc::new(SessionsManageOnlyResolver),
        );

        let err = CreateWorkspaceFile {
            session_id: session.id.to_string(),
            path: "x.txt".to_string(),
            req: file_req("x"),
        }
        .run(&ctx)
        .await
        .expect_err("shared workspace write must require WORKSPACE_MANAGE");
        assert!(matches!(err.kind, CommandErrorKind::Forbidden(_)));
    }

    fn update_req(content: &str, expected: Option<&str>) -> UpdateFileRequest {
        UpdateFileRequest {
            content: Some(content.to_string()),
            encoding: None,
            is_readonly: None,
            expected_content: expected.map(str::to_string),
            expected_encoding: None,
        }
    }

    /// Seed one session with a single file and return (db, session, ctx).
    async fn seeded(
        path: &str,
        content: &str,
    ) -> (Arc<StorageBackend>, crate::storage::models::SessionRow, Ctx) {
        let db = Arc::new(StorageBackend::in_memory());
        let session = db
            .create_session(session_row(None))
            .await
            .expect("create session");
        crate::domains::session_files::WorkspaceFileService::new(db.clone())
            .create_file(
                session.workspace_id,
                crate::domains::session_files::CreateFileInput {
                    path: path.to_string(),
                    content: Some(content.to_string()),
                    encoding: None,
                    is_readonly: None,
                },
            )
            .await
            .expect("seed file");
        let ctx = Ctx::minimal_for_test(owner_caller(), db.clone(), None);
        (db, session, ctx)
    }

    /// A compare-and-swap write lands when the file still holds what the caller
    /// last read. This is the worker's `write_file_if_content_matches`.
    #[tokio::test]
    async fn update_with_matching_expected_content_writes() {
        let (_db, session, ctx) = seeded("/notes.txt", "before").await;

        let file = UpdateWorkspaceFile {
            session_id: session.id.to_string(),
            path: "/notes.txt".to_string(),
            req: update_req("after", Some("before")),
        }
        .run(&ctx)
        .await
        .expect("matching expected content must write");

        assert_eq!(file.content.as_deref(), Some("after"));
    }

    /// A mismatch must not overwrite the concurrent writer, and must be
    /// distinguishable from "no such file" so the caller can re-read and retry.
    #[tokio::test]
    async fn update_with_stale_expected_content_conflicts() {
        let (db, session, ctx) = seeded("/notes.txt", "moved on").await;

        let err = UpdateWorkspaceFile {
            session_id: session.id.to_string(),
            path: "/notes.txt".to_string(),
            req: update_req("after", Some("what I last read")),
        }
        .run(&ctx)
        .await
        .expect_err("stale expected content must not overwrite");
        assert!(
            matches!(err.kind, CommandErrorKind::Conflict(_)),
            "a lost race is a conflict, not a not-found: {err:?}"
        );

        let still = crate::domains::session_files::WorkspaceFileService::new(db)
            .read_file(session.workspace_id, "/notes.txt")
            .await
            .expect("read back")
            .expect("file still there");
        assert_eq!(
            still.content.as_deref(),
            Some("moved on"),
            "the concurrent writer's content must survive"
        );
    }

    /// A CAS against a path that does not exist is a not-found, not a conflict.
    #[tokio::test]
    async fn update_with_expected_content_on_missing_file_is_not_found() {
        let (_db, session, ctx) = seeded("/notes.txt", "x").await;

        let err = UpdateWorkspaceFile {
            session_id: session.id.to_string(),
            path: "/absent.txt".to_string(),
            req: update_req("after", Some("anything")),
        }
        .run(&ctx)
        .await
        .expect_err("missing file must not be reported as a conflict");
        assert!(matches!(err.kind, CommandErrorKind::NotFound(_)));
    }

    /// The search command returns the context lines the grouped grep cannot,
    /// which is why it exists alongside it.
    #[tokio::test]
    async fn search_returns_surrounding_context() {
        let (_db, session, ctx) = seeded("/notes.txt", "alpha\nneedle\nomega\n").await;

        let result = SearchWorkspaceFiles {
            session_id: session.id.to_string(),
            req: SearchRequest {
                pattern: "needle".to_string(),
                path_pattern: None,
                before_context: 1,
                after_context: 1,
                offset: 0,
                limit: None,
                max_bytes: None,
            },
        }
        .run(&ctx)
        .await
        .expect("search must succeed");

        // With context requested the results arrive as blocks rather than flat
        // matches; `total_matches` is the count either way.
        assert_eq!(result.total_matches, 1, "one match expected");
        let block = result
            .blocks
            .iter()
            .find(|b| b.path == "/notes.txt")
            .expect("a context block for the matching file");
        assert!(
            block.lines.iter().any(|l| l.line == "alpha" && !l.is_match),
            "the line before the match must come back as context: {block:?}"
        );
        assert!(
            block.lines.iter().any(|l| l.line == "needle" && l.is_match),
            "the matching line must be flagged: {block:?}"
        );
    }

    /// A session may be attached to an existing workspace at creation, which
    /// grants its agent read/write access to that workspace's files. Since
    /// migration 056 re-keyed `workspace_files` by `workspace_id`, everything
    /// reaching `WorkspaceFileService` must pass the session's *workspace*, not
    /// its id — the parameter there is still spelled `session_id`, which is what
    /// makes passing the wrong one look right.
    ///
    /// Ported from the worker file RPCs' own guard when those RPCs were deleted
    /// in favour of these commands: the property is the same, and the commands
    /// are now the only path to it.
    #[tokio::test]
    async fn commands_address_the_sessions_workspace_not_its_id() {
        let db = Arc::new(StorageBackend::in_memory());

        // A workspace owner, then a second session attached to that workspace.
        let owner = db
            .create_session(session_row(None))
            .await
            .expect("owner session");
        let attached = db
            .create_session(session_row(Some(owner.workspace_id)))
            .await
            .expect("attached session");
        assert_ne!(
            attached.id.uuid(),
            attached.workspace_id,
            "the attached session's id must differ from its workspace for this to prove anything"
        );

        // Seed through the workspace the owner established.
        crate::domains::session_files::WorkspaceFileService::new(db.clone())
            .create_file(
                owner.workspace_id,
                crate::domains::session_files::CreateFileInput {
                    path: "/shared.txt".to_string(),
                    content: Some("shared bytes".to_string()),
                    encoding: None,
                    is_readonly: None,
                },
            )
            .await
            .expect("seed shared file");

        let ctx = Ctx::minimal_for_test(owner_caller(), db, None);

        // The attached session must see the workspace's file, not an empty
        // store of its own keyed by session id.
        let got = GetWorkspaceFile {
            session_id: attached.id.to_string(),
            path: "/shared.txt".to_string(),
            recursive: false,
        }
        .run(&ctx)
        .await
        .expect("attached session must reach the shared workspace");

        match got {
            super::GetResponse::File(file) => {
                assert_eq!(file.content.as_deref(), Some("shared bytes"));
            }
            other => panic!("expected the shared file, got {other:?}"),
        }
    }
}
