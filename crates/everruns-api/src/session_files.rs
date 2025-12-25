// Session Files (Virtual Filesystem) HTTP routes

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post, put},
    Json, Router,
};
use everruns_core::{FileInfo, FileStat, GrepResult, SessionFile};
use everruns_storage::Database;

use crate::common::ListResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::services::SessionFileService;

/// Request to create a file
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateFileRequest {
    /// File path (e.g., "/folder/file.txt")
    pub path: String,
    /// File content (text or base64-encoded)
    #[serde(default)]
    pub content: Option<String>,
    /// Content encoding: "text" or "base64"
    #[serde(default)]
    pub encoding: Option<String>,
    /// Whether file is read-only
    #[serde(default)]
    pub is_readonly: Option<bool>,
}

/// Request to create a directory
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateDirectoryRequest {
    /// Directory path (e.g., "/folder/subfolder")
    pub path: String,
}

/// Request to update a file
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateFileRequest {
    /// New file content
    #[serde(default)]
    pub content: Option<String>,
    /// Content encoding: "text" or "base64"
    #[serde(default)]
    pub encoding: Option<String>,
    /// Whether file is read-only
    #[serde(default)]
    pub is_readonly: Option<bool>,
}

/// Request to move/rename a file
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct MoveFileRequest {
    /// Source path
    pub src_path: String,
    /// Destination path
    pub dst_path: String,
}

/// Request to copy a file
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CopyFileRequest {
    /// Source path
    pub src_path: String,
    /// Destination path
    pub dst_path: String,
}

/// Request to search files
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GrepRequest {
    /// Regex pattern to search for
    pub pattern: String,
    /// Optional path pattern to filter files
    #[serde(default)]
    pub path_pattern: Option<String>,
}

/// Query parameters for listing files
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ListFilesQuery {
    /// Directory path to list (defaults to "/")
    #[serde(default = "default_root")]
    pub path: String,
    /// Whether to list recursively
    #[serde(default)]
    pub recursive: bool,
}

fn default_root() -> String {
    "/".to_string()
}

/// Query parameters for reading files
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ReadFileQuery {
    /// File path to read
    pub path: String,
}

/// Query parameters for stat
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct StatQuery {
    /// File/directory path
    pub path: String,
}

/// Query parameters for deleting files
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DeleteFileQuery {
    /// File/directory path
    pub path: String,
    /// Whether to delete recursively
    #[serde(default)]
    pub recursive: bool,
}

/// Response for delete operation
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeleteResponse {
    pub deleted: bool,
}

/// App state for session files routes
#[derive(Clone)]
pub struct AppState {
    pub file_service: Arc<SessionFileService>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            file_service: Arc::new(SessionFileService::new(db)),
        }
    }
}

/// Create session files routes (nested under sessions)
pub fn routes(state: AppState) -> Router {
    Router::new()
        // File operations
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/files",
            get(list_files).post(create_file),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/files/read",
            get(read_file),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/files/write",
            put(update_file),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/files/stat",
            get(stat_file),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/files/delete",
            delete(delete_file),
        )
        // Directory operations
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/files/mkdir",
            post(create_directory),
        )
        // File management operations
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/files/move",
            post(move_file),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/files/copy",
            post(copy_file),
        )
        // Search
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/files/grep",
            post(grep_files),
        )
        .with_state(state)
}

/// GET /v1/agents/{agent_id}/sessions/{session_id}/files - List files
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("path" = Option<String>, Query, description = "Directory path to list"),
        ("recursive" = Option<bool>, Query, description = "List recursively")
    ),
    responses(
        (status = 200, description = "List of files", body = ListResponse<FileInfo>),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn list_files(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<ListFilesQuery>,
) -> Result<Json<ListResponse<FileInfo>>, StatusCode> {
    let files = if query.recursive {
        state.file_service.list_all(session_id).await
    } else {
        state
            .file_service
            .list_directory(session_id, &query.path)
            .await
    }
    .map_err(|e| {
        tracing::error!("Failed to list files: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ListResponse::new(files)))
}

/// POST /v1/agents/{agent_id}/sessions/{session_id}/files - Create file
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = CreateFileRequest,
    responses(
        (status = 201, description = "File created successfully", body = SessionFile),
        (status = 400, description = "Invalid request"),
        (status = 409, description = "File already exists"),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn create_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<CreateFileRequest>,
) -> Result<(StatusCode, Json<SessionFile>), (StatusCode, String)> {
    let file = state
        .file_service
        .create_file(session_id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create file: {}", e);
            let msg = e.to_string();
            if msg.contains("already exists") {
                (StatusCode::CONFLICT, msg)
            } else if msg.contains("Invalid") || msg.contains("cannot") {
                (StatusCode::BAD_REQUEST, msg)
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?;

    Ok((StatusCode::CREATED, Json(file)))
}

/// GET /v1/agents/{agent_id}/sessions/{session_id}/files/read - Read file
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files/read",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("path" = String, Query, description = "File path to read")
    ),
    responses(
        (status = 200, description = "File content", body = SessionFile),
        (status = 404, description = "File not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn read_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<ReadFileQuery>,
) -> Result<Json<SessionFile>, StatusCode> {
    let file = state
        .file_service
        .read_file(session_id, &query.path)
        .await
        .map_err(|e| {
            tracing::error!("Failed to read file: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(file))
}

/// PUT /v1/agents/{agent_id}/sessions/{session_id}/files/write - Update file
#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files/write",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("path" = String, Query, description = "File path to update")
    ),
    request_body = UpdateFileRequest,
    responses(
        (status = 200, description = "File updated successfully", body = SessionFile),
        (status = 400, description = "Cannot modify readonly file"),
        (status = 404, description = "File not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn update_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<ReadFileQuery>,
    Json(req): Json<UpdateFileRequest>,
) -> Result<Json<SessionFile>, (StatusCode, String)> {
    let file = state
        .file_service
        .update_file(session_id, &query.path, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update file: {}", e);
            let msg = e.to_string();
            if msg.contains("readonly") {
                (StatusCode::BAD_REQUEST, msg)
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?
        .ok_or((StatusCode::NOT_FOUND, "File not found".to_string()))?;

    Ok(Json(file))
}

/// GET /v1/agents/{agent_id}/sessions/{session_id}/files/stat - Get file stat
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files/stat",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("path" = String, Query, description = "File/directory path")
    ),
    responses(
        (status = 200, description = "File stat", body = FileStat),
        (status = 404, description = "File not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn stat_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<StatQuery>,
) -> Result<Json<FileStat>, StatusCode> {
    let stat = state
        .file_service
        .stat(session_id, &query.path)
        .await
        .map_err(|e| {
            tracing::error!("Failed to stat file: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(stat))
}

/// DELETE /v1/agents/{agent_id}/sessions/{session_id}/files/delete - Delete file
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files/delete",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("path" = String, Query, description = "File/directory path"),
        ("recursive" = Option<bool>, Query, description = "Delete recursively")
    ),
    responses(
        (status = 200, description = "Delete result", body = DeleteResponse),
        (status = 400, description = "Directory not empty"),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn delete_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<DeleteFileQuery>,
) -> Result<Json<DeleteResponse>, (StatusCode, String)> {
    let deleted = state
        .file_service
        .delete(session_id, &query.path, query.recursive)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete file: {}", e);
            let msg = e.to_string();
            if msg.contains("not empty") || msg.contains("Cannot delete root") {
                (StatusCode::BAD_REQUEST, msg)
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?;

    Ok(Json(DeleteResponse { deleted }))
}

/// POST /v1/agents/{agent_id}/sessions/{session_id}/files/mkdir - Create directory
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files/mkdir",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = CreateDirectoryRequest,
    responses(
        (status = 201, description = "Directory created successfully", body = FileInfo),
        (status = 400, description = "Invalid path or file exists at path"),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn create_directory(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<CreateDirectoryRequest>,
) -> Result<(StatusCode, Json<FileInfo>), (StatusCode, String)> {
    let dir = state
        .file_service
        .create_directory(session_id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create directory: {}", e);
            let msg = e.to_string();
            if msg.contains("file exists") || msg.contains("Invalid") {
                (StatusCode::BAD_REQUEST, msg)
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?;

    Ok((StatusCode::CREATED, Json(dir)))
}

/// POST /v1/agents/{agent_id}/sessions/{session_id}/files/move - Move/rename file
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files/move",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = MoveFileRequest,
    responses(
        (status = 200, description = "File moved successfully", body = SessionFile),
        (status = 400, description = "Invalid path"),
        (status = 404, description = "Source not found"),
        (status = 409, description = "Destination exists"),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn move_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<MoveFileRequest>,
) -> Result<Json<SessionFile>, (StatusCode, String)> {
    let file = state
        .file_service
        .move_file(session_id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to move file: {}", e);
            let msg = e.to_string();
            if msg.contains("not found") {
                (StatusCode::NOT_FOUND, msg)
            } else if msg.contains("already exists") {
                (StatusCode::CONFLICT, msg)
            } else if msg.contains("Invalid") {
                (StatusCode::BAD_REQUEST, msg)
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?
        .ok_or((StatusCode::NOT_FOUND, "Source not found".to_string()))?;

    Ok(Json(file))
}

/// POST /v1/agents/{agent_id}/sessions/{session_id}/files/copy - Copy file
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files/copy",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = CopyFileRequest,
    responses(
        (status = 201, description = "File copied successfully", body = SessionFile),
        (status = 400, description = "Cannot copy directories"),
        (status = 404, description = "Source not found"),
        (status = 409, description = "Destination exists"),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn copy_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<CopyFileRequest>,
) -> Result<(StatusCode, Json<SessionFile>), (StatusCode, String)> {
    let file = state
        .file_service
        .copy_file(session_id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to copy file: {}", e);
            let msg = e.to_string();
            if msg.contains("not found") {
                (StatusCode::NOT_FOUND, msg)
            } else if msg.contains("already exists") {
                (StatusCode::CONFLICT, msg)
            } else if msg.contains("Cannot copy") || msg.contains("Invalid") {
                (StatusCode::BAD_REQUEST, msg)
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?
        .ok_or((StatusCode::NOT_FOUND, "Source not found".to_string()))?;

    Ok((StatusCode::CREATED, Json(file)))
}

/// POST /v1/agents/{agent_id}/sessions/{session_id}/files/grep - Search files
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/files/grep",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = GrepRequest,
    responses(
        (status = 200, description = "Search results", body = ListResponse<GrepResult>),
        (status = 400, description = "Invalid regex pattern"),
        (status = 500, description = "Internal server error")
    ),
    tag = "files"
)]
pub async fn grep_files(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<GrepRequest>,
) -> Result<Json<ListResponse<GrepResult>>, (StatusCode, String)> {
    let results = state
        .file_service
        .grep(session_id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to grep files: {}", e);
            let msg = e.to_string();
            if msg.contains("regex") || msg.contains("pattern") {
                (StatusCode::BAD_REQUEST, format!("Invalid regex: {}", msg))
            } else {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
            }
        })?;

    Ok(Json(ListResponse::new(results)))
}
