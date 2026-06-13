// Workspace virtual filesystem HTTP routes.
//
// Mounted on /v1/workspaces/{workspace_id}/fs/*. The migration that renamed
// `session_files` to `workspace_files` (054) and the equality invariant
// `workspace.id == session.id` for default 1:1 sessions mean
// WorkspaceFileService is already keyed by workspace UUID — these handlers just
// resolve the public `wsp_<32-hex>` ID to that UUID and delegate.
//
// This is the canonical filesystem surface. The legacy
// /v1/sessions/{session_id}/fs/* routes remain mounted but are delisted from
// the published OpenAPI spec (see crates/server/src/openapi.rs).

use crate::api::session_files::{
    CopyFileRequest, CreateFileRequest, DeleteQuery, DeleteResponse, GetQuery, GetResponse,
    GrepRequest, MoveFileRequest, StatRequest, UpdateFileRequest, raw_file_response,
    wants_raw_file,
};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::session_files::{
    CopyFileInput, CreateDirectoryInput, CreateFileInput, GrepInput, MoveFileInput,
    UpdateFileInput, WorkspaceFileService,
};
use crate::domains::workspaces::{WORKSPACE_MANAGE, WORKSPACE_VIEW};
use crate::storage::StorageBackend;
use crate::storage::models::WorkspaceRow;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use everruns_core::typed_id::WorkspaceId;
use everruns_core::{Caller, FileInfo, FileStat, GrepResult, Policy, SessionFile};
use std::sync::Arc;
use uuid::Uuid;

use super::common::{ApiPolicyResultExt, ListResponse, impl_auth_state};

#[derive(Clone)]
pub struct AppState {
    pub file_service: Arc<WorkspaceFileService>,
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            file_service: Arc::new(WorkspaceFileService::new(db.clone())),
            db,
            auth,
        }
    }

    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::domains::session_files::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.file_service =
            Arc::new(WorkspaceFileService::new(self.db.clone()).with_virtual_registry(registry));
        self
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/workspaces/{workspace_id}/fs/_/move", post(move_file))
        .route("/v1/workspaces/{workspace_id}/fs/_/copy", post(copy_file))
        .route("/v1/workspaces/{workspace_id}/fs/_/grep", post(grep_files))
        .route("/v1/workspaces/{workspace_id}/fs/_/stat", post(stat_file))
        .route(
            "/v1/workspaces/{workspace_id}/fs/_/download/{*path}",
            get(download_path),
        )
        .route(
            "/v1/workspaces/{workspace_id}/fs",
            get(get_root).post(create_root).delete(delete_root),
        )
        .route(
            "/v1/workspaces/{workspace_id}/fs/{*path}",
            get(get_path)
                .post(create_path)
                .put(update_path)
                .delete(delete_path),
        )
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(state)
}

const WORKSPACE_PREFIX: &str = "/workspace";

fn normalize_path(path: &str) -> String {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return "/".to_string();
    }
    let abs_path = format!("/{path}");
    if abs_path == WORKSPACE_PREFIX {
        "/".to_string()
    } else if let Some(stripped) = abs_path.strip_prefix(WORKSPACE_PREFIX) {
        if stripped.starts_with('/') {
            stripped.to_string()
        } else {
            abs_path
        }
    } else {
        abs_path
    }
}

fn is_reserved_path(path: &str) -> bool {
    let path = path.trim_start_matches('/');
    path.starts_with('_') || path.split('/').any(|segment| segment.starts_with('_'))
}

async fn resolve_workspace(
    state: &AppState,
    org: &ResolvedOrg,
    workspace_id: &str,
    policy: &Policy,
    operation: &str,
) -> Result<WorkspaceRow, (StatusCode, String)> {
    let caller = Caller::from(org);
    policy
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(anyhow::Error::from)
        .map_policy_or_internal(operation)
        .map_err(|(status, body)| {
            let err = body.0;
            (status, err.detail.unwrap_or(err.title))
        })?;
    let id: WorkspaceId = workspace_id
        .parse()
        .map_err(|_| (StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;
    let row = state
        .db
        .get_workspace(org.org_id, id)
        .await
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, "Workspace not found".to_string()))?;
    Ok(row)
}

async fn resolve_for_read(
    state: &AppState,
    org: &ResolvedOrg,
    workspace_id: &str,
    operation: &str,
) -> Result<Uuid, (StatusCode, String)> {
    let row = resolve_workspace(state, org, workspace_id, &WORKSPACE_VIEW, operation).await?;
    Ok(row.id)
}

async fn resolve_for_write(
    state: &AppState,
    org: &ResolvedOrg,
    workspace_id: &str,
    operation: &str,
) -> Result<Uuid, (StatusCode, String)> {
    let row = resolve_workspace(state, org, workspace_id, &WORKSPACE_MANAGE, operation).await?;
    if row.status != "active" {
        return Err((
            StatusCode::FORBIDDEN,
            format!("Workspace is {} and cannot be modified", row.status),
        ));
    }
    Ok(row.id)
}

fn internal_error<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    tracing::error!(error = %e, "workspace_files: internal error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
    )
}

fn service_error<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    let msg = e.to_string();
    let lower = msg.to_lowercase();
    if lower.contains("not found") {
        (StatusCode::NOT_FOUND, msg)
    } else if lower.contains("already exists") {
        (StatusCode::CONFLICT, msg)
    } else if lower.contains("readonly") || lower.contains("read-only") {
        (StatusCode::FORBIDDEN, msg)
    } else if lower.contains("invalid") || lower.contains("not empty") {
        (StatusCode::BAD_REQUEST, msg)
    } else {
        tracing::error!(error = %msg, "workspace_files: service error");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error".to_string(),
        )
    }
}

/// GET /v1/workspaces/{workspace_id}/fs - Workspace root listing
#[utoipa::path(
    get,
    path = "/v1/workspaces/{workspace_id}/fs",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
        ("recursive" = Option<bool>, Query, description = "List recursively"),
    ),
    responses(
        (status = 200, description = "Directory listing", body = ListResponse<FileInfo>),
        (status = 404, description = "Workspace not found"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn get_root(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Query(query): Query<GetQuery>,
) -> Result<Json<ListResponse<FileInfo>>, (StatusCode, String)> {
    let uuid = resolve_for_read(
        &state,
        &org,
        &workspace_id,
        "authorize list workspace files",
    )
    .await?;
    let files = if query.recursive {
        state.file_service.list_all(uuid).await
    } else {
        state.file_service.list_directory(uuid, "/").await
    }
    .map_err(service_error)?;
    Ok(Json(ListResponse::new(files)))
}

/// GET /v1/workspaces/{workspace_id}/fs/{path} - Read file or list directory
#[utoipa::path(
    get,
    path = "/v1/workspaces/{workspace_id}/fs/{path}",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
        ("path" = String, Path, description = "File or directory path. Wildcard route: nested paths are literal `/`-separated segments, not a single URL-encoded value."),
        ("recursive" = Option<bool>, Query, description = "List recursively"),
    ),
    responses(
        (status = 200, description = "File content or directory listing", body = GetResponse),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn get_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((workspace_id, path)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<GetQuery>,
) -> Result<Response, (StatusCode, String)> {
    let uuid =
        resolve_for_read(&state, &org, &workspace_id, "authorize read workspace file").await?;
    let path = normalize_path(&path);
    let stat = state
        .file_service
        .stat(uuid, &path)
        .await
        .map_err(service_error)?
        .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;
    if stat.is_directory {
        let files = if query.recursive {
            state.file_service.list_all(uuid).await
        } else {
            state.file_service.list_directory(uuid, &path).await
        }
        .map_err(service_error)?;
        Ok(Json(GetResponse::Listing(ListResponse::new(files))).into_response())
    } else {
        let file = state
            .file_service
            .read_file(uuid, &path)
            .await
            .map_err(service_error)?
            .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;
        if wants_raw_file(&headers) {
            return raw_file_response(file, false);
        }
        Ok(Json(GetResponse::File(file)).into_response())
    }
}

pub async fn create_root(_org: ResolvedOrg) -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        "Cannot create at root path, specify a path".to_string(),
    )
}

/// POST /v1/workspaces/{workspace_id}/fs/{path} - Create file or directory
#[utoipa::path(
    post,
    path = "/v1/workspaces/{workspace_id}/fs/{path}",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
        ("path" = String, Path, description = "File or directory path"),
    ),
    request_body = CreateFileRequest,
    responses(
        (status = 201, description = "Created", body = SessionFile),
        (status = 400, description = "Invalid request"),
        (status = 409, description = "Already exists"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn create_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((workspace_id, path)): Path<(String, String)>,
    Json(req): Json<CreateFileRequest>,
) -> Result<(StatusCode, Json<SessionFile>), (StatusCode, String)> {
    let uuid = resolve_for_write(
        &state,
        &org,
        &workspace_id,
        "authorize create workspace file",
    )
    .await?;
    let path = normalize_path(&path);
    if is_reserved_path(&path) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Paths starting with '_' are reserved for system actions".to_string(),
        ));
    }

    let file = if req.is_directory.unwrap_or(false) {
        let dir = state
            .file_service
            .create_directory(uuid, CreateDirectoryInput { path })
            .await
            .map_err(service_error)?;
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
        state
            .file_service
            .create_file(
                uuid,
                CreateFileInput {
                    path,
                    content: req.content,
                    encoding: req.encoding,
                    is_readonly: req.is_readonly,
                },
            )
            .await
            .map_err(service_error)?
    };
    Ok((StatusCode::CREATED, Json(file)))
}

/// PUT /v1/workspaces/{workspace_id}/fs/{path} - Update file content
#[utoipa::path(
    put,
    path = "/v1/workspaces/{workspace_id}/fs/{path}",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
        ("path" = String, Path, description = "File path"),
    ),
    request_body = UpdateFileRequest,
    responses(
        (status = 200, description = "Updated", body = SessionFile),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn update_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((workspace_id, path)): Path<(String, String)>,
    Json(req): Json<UpdateFileRequest>,
) -> Result<Json<SessionFile>, (StatusCode, String)> {
    let uuid = resolve_for_write(
        &state,
        &org,
        &workspace_id,
        "authorize update workspace file",
    )
    .await?;
    let path = normalize_path(&path);
    if is_reserved_path(&path) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Paths starting with '_' are reserved for system actions".to_string(),
        ));
    }
    let file = state
        .file_service
        .update_file(
            uuid,
            &path,
            UpdateFileInput {
                content: req.content,
                encoding: req.encoding,
                is_readonly: req.is_readonly,
            },
        )
        .await
        .map_err(service_error)?
        .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;
    Ok(Json(file))
}

pub async fn delete_root(_org: ResolvedOrg) -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        "Cannot delete root directory".to_string(),
    )
}

/// DELETE /v1/workspaces/{workspace_id}/fs/{path} - Delete file or directory
#[utoipa::path(
    delete,
    path = "/v1/workspaces/{workspace_id}/fs/{path}",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
        ("path" = String, Path, description = "File or directory path"),
        ("recursive" = Option<bool>, Query, description = "Delete recursively"),
    ),
    responses(
        (status = 200, description = "Deleted", body = DeleteResponse),
        (status = 400, description = "Invalid request"),
        (status = 403, description = "Cannot delete readonly content"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn delete_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((workspace_id, path)): Path<(String, String)>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<DeleteResponse>, (StatusCode, String)> {
    let uuid = resolve_for_write(
        &state,
        &org,
        &workspace_id,
        "authorize delete workspace file",
    )
    .await?;
    let path = normalize_path(&path);
    let deleted = state
        .file_service
        .delete(uuid, &path, query.recursive)
        .await
        .map_err(service_error)?;
    Ok(Json(DeleteResponse { deleted }))
}

/// POST /v1/workspaces/{workspace_id}/fs/_/stat - File metadata
#[utoipa::path(
    post,
    path = "/v1/workspaces/{workspace_id}/fs/_/stat",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
    ),
    request_body = StatRequest,
    responses(
        (status = 200, description = "File metadata", body = FileStat),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn stat_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Json(req): Json<StatRequest>,
) -> Result<Json<FileStat>, (StatusCode, String)> {
    let uuid =
        resolve_for_read(&state, &org, &workspace_id, "authorize stat workspace file").await?;
    let path = normalize_path(&req.path);
    let stat = state
        .file_service
        .stat(uuid, &path)
        .await
        .map_err(service_error)?
        .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;
    Ok(Json(stat))
}

/// POST /v1/workspaces/{workspace_id}/fs/_/grep - Search files
#[utoipa::path(
    post,
    path = "/v1/workspaces/{workspace_id}/fs/_/grep",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
    ),
    request_body = GrepRequest,
    responses(
        (status = 200, description = "Search results", body = ListResponse<GrepResult>),
        (status = 400, description = "Invalid regex pattern"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn grep_files(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Json(req): Json<GrepRequest>,
) -> Result<Json<ListResponse<GrepResult>>, (StatusCode, String)> {
    let uuid = resolve_for_read(
        &state,
        &org,
        &workspace_id,
        "authorize grep workspace files",
    )
    .await?;
    let results = state
        .file_service
        .grep(
            uuid,
            GrepInput {
                pattern: req.pattern,
                path_pattern: req.path_pattern,
            },
        )
        .await
        .map_err(service_error)?;
    Ok(Json(ListResponse::new(results)))
}

/// POST /v1/workspaces/{workspace_id}/fs/_/move - Move/rename a file or directory
#[utoipa::path(
    post,
    path = "/v1/workspaces/{workspace_id}/fs/_/move",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
    ),
    request_body = MoveFileRequest,
    responses(
        (status = 200, description = "Moved", body = SessionFile),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Source not found"),
        (status = 409, description = "Destination exists"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn move_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Json(req): Json<MoveFileRequest>,
) -> Result<Json<SessionFile>, (StatusCode, String)> {
    let uuid =
        resolve_for_write(&state, &org, &workspace_id, "authorize move workspace file").await?;
    let file = state
        .file_service
        .move_file(
            uuid,
            MoveFileInput {
                src_path: req.src_path,
                dst_path: req.dst_path,
            },
        )
        .await
        .map_err(service_error)?
        .ok_or((StatusCode::NOT_FOUND, "Source not found".to_string()))?;
    Ok(Json(file))
}

/// POST /v1/workspaces/{workspace_id}/fs/_/copy - Copy a file
#[utoipa::path(
    post,
    path = "/v1/workspaces/{workspace_id}/fs/_/copy",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
    ),
    request_body = CopyFileRequest,
    responses(
        (status = 201, description = "Copied", body = SessionFile),
        (status = 400, description = "Invalid request or cannot copy directories"),
        (status = 404, description = "Source not found"),
        (status = 409, description = "Destination exists"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn copy_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Json(req): Json<CopyFileRequest>,
) -> Result<(StatusCode, Json<SessionFile>), (StatusCode, String)> {
    let uuid =
        resolve_for_write(&state, &org, &workspace_id, "authorize copy workspace file").await?;
    let file = state
        .file_service
        .copy_file(
            uuid,
            CopyFileInput {
                src_path: req.src_path,
                dst_path: req.dst_path,
            },
        )
        .await
        .map_err(service_error)?
        .ok_or((StatusCode::NOT_FOUND, "Source not found".to_string()))?;
    Ok((StatusCode::CREATED, Json(file)))
}

/// GET /v1/workspaces/{workspace_id}/fs/_/download/{path} - Download raw file bytes
#[utoipa::path(
    get,
    path = "/v1/workspaces/{workspace_id}/fs/_/download/{path}",
    params(
        ("workspace_id" = String, Path, description = "Workspace ID (wsp_<32-hex>)"),
        ("path" = String, Path, description = "File path. Wildcard route: nested paths are literal `/`-separated segments, not a single URL-encoded value."),
    ),
    responses(
        (status = 200, description = "Raw file bytes", content_type = "application/octet-stream"),
        (status = 400, description = "Path points to a directory"),
        (status = 404, description = "File not found"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "workspace-filesystem"
)]
pub async fn download_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((workspace_id, path)): Path<(String, String)>,
) -> Result<Response, (StatusCode, String)> {
    let uuid = resolve_for_read(
        &state,
        &org,
        &workspace_id,
        "authorize download workspace file",
    )
    .await?;
    let path = normalize_path(&path);
    let stat = state
        .file_service
        .stat(uuid, &path)
        .await
        .map_err(service_error)?
        .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;
    if stat.is_directory {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot download a directory".to_string(),
        ));
    }
    let file = state
        .file_service
        .read_file(uuid, &path)
        .await
        .map_err(service_error)?
        .ok_or((StatusCode::NOT_FOUND, "Not found".to_string()))?;
    raw_file_response(file, true)
}
