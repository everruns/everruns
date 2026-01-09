// Session Files (Virtual Filesystem) HTTP routes
//
// RESTful API design:
// - GET    /fs/*path  - Read file content or list directory
// - POST   /fs/*path  - Create file or directory
// - PUT    /fs/*path  - Update file content
// - DELETE /fs/*path  - Delete file or directory
// - POST   /fs/_/move - Move/rename file
// - POST   /fs/_/copy - Copy file
// - POST   /fs/_/grep - Search files
// - POST   /fs/_/stat - Get file metadata
//
// Note: Paths starting with "_" are reserved for actions and cannot be
// used for file creation or updates.

use crate::storage::StorageBackend;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_core::{FileInfo, FileStat, GrepResult, SessionFile};

use super::common::ListResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::services::session_file::{
    CopyFileInput, CreateDirectoryInput, CreateFileInput, GrepInput, MoveFileInput,
    SessionFileService, UpdateFileInput,
};

/// Request to create a file
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateFileRequest {
    /// File content (text or base64-encoded)
    #[serde(default)]
    pub content: Option<String>,
    /// Content encoding: "text" or "base64"
    #[serde(default)]
    pub encoding: Option<String>,
    /// Whether file is read-only
    #[serde(default)]
    pub is_readonly: Option<bool>,
    /// Whether to create a directory instead of a file
    #[serde(default)]
    pub is_directory: Option<bool>,
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

/// Request to get file stat
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct StatRequest {
    /// Path to the file or directory
    pub path: String,
}

/// Query parameters for GET requests
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GetQuery {
    /// For directories: whether to list recursively
    #[serde(default)]
    pub recursive: bool,
}

/// Query parameters for DELETE requests
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DeleteQuery {
    /// Whether to delete recursively
    #[serde(default)]
    pub recursive: bool,
}

/// Response for delete operation
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DeleteResponse {
    pub deleted: bool,
}

/// Unified response for GET that can be file or directory listing
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(untagged)]
pub enum GetResponse {
    File(SessionFile),
    Listing(ListResponse<FileInfo>),
}

/// App state for session files routes
#[derive(Clone)]
pub struct AppState {
    pub file_service: Arc<SessionFileService>,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            file_service: Arc::new(SessionFileService::new(db)),
        }
    }
}

/// Create session files routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Actions (must be before wildcard to take precedence)
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/fs/_/move",
            post(move_file),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/fs/_/copy",
            post(copy_file),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/fs/_/grep",
            post(grep_files),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/fs/_/stat",
            post(stat_file),
        )
        // File operations with path
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/fs",
            get(get_root).post(create_root).delete(delete_root),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id/fs/*path",
            get(get_path)
                .post(create_path)
                .put(update_path)
                .delete(delete_path),
        )
        .with_state(state)
}

// Helper to normalize path from URL
fn normalize_path(path: &str) -> String {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", path)
    }
}

// Check if path is reserved (starts with _ which is used for actions)
fn is_reserved_path(path: &str) -> bool {
    let path = path.trim_start_matches('/');
    path.starts_with('_') || path.split('/').any(|segment| segment.starts_with('_'))
}

/// GET /fs - Get root directory listing
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/fs",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("recursive" = Option<bool>, Query, description = "List recursively")
    ),
    responses(
        (status = 200, description = "Directory listing"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn get_root(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Query(query): Query<GetQuery>,
) -> Result<Json<GetResponse>, StatusCode> {
    get_path_impl(state, session_id, "/", query).await
}

/// GET /fs/*path - Get file content or directory listing
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/fs/{path}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("path" = String, Path, description = "File or directory path"),
        ("recursive" = Option<bool>, Query, description = "List recursively")
    ),
    responses(
        (status = 200, description = "File content or directory listing"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn get_path(
    State(state): State<AppState>,
    Path((_agent_id, session_id, path)): Path<(Uuid, Uuid, String)>,
    Query(query): Query<GetQuery>,
) -> Result<Json<GetResponse>, StatusCode> {
    let normalized = normalize_path(&path);
    get_path_impl(state, session_id, &normalized, query).await
}

async fn get_path_impl(
    state: AppState,
    session_id: Uuid,
    path: &str,
    query: GetQuery,
) -> Result<Json<GetResponse>, StatusCode> {
    // Check if path is a directory or file
    let stat = state
        .file_service
        .stat(session_id, path)
        .await
        .map_err(|e| {
            tracing::error!("Failed to stat: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    match stat {
        Some(s) if s.is_directory => {
            // List directory
            let files = if query.recursive {
                state.file_service.list_all(session_id).await
            } else {
                state.file_service.list_directory(session_id, path).await
            }
            .map_err(|e| {
                tracing::error!("Failed to list: {}", e);
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            Ok(Json(GetResponse::Listing(ListResponse::new(files))))
        }
        Some(_) => {
            // Read file
            let file = state
                .file_service
                .read_file(session_id, path)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to read file: {}", e);
                    StatusCode::INTERNAL_SERVER_ERROR
                })?
                .ok_or(StatusCode::NOT_FOUND)?;
            Ok(Json(GetResponse::File(file)))
        }
        None => {
            // For root path, return empty listing
            if path == "/" {
                Ok(Json(GetResponse::Listing(ListResponse::new(vec![]))))
            } else {
                Err(StatusCode::NOT_FOUND)
            }
        }
    }
}

/// POST /fs - Create at root (not allowed)
pub async fn create_root() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        "Cannot create at root path, specify a path".to_string(),
    )
}

/// POST /fs/*path - Create file or directory
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/fs/{path}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("path" = String, Path, description = "File or directory path")
    ),
    request_body = CreateFileRequest,
    responses(
        (status = 201, description = "Created successfully", body = SessionFile),
        (status = 400, description = "Invalid request"),
        (status = 409, description = "Already exists"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn create_path(
    State(state): State<AppState>,
    Path((_agent_id, session_id, path)): Path<(Uuid, Uuid, String)>,
    Json(req): Json<CreateFileRequest>,
) -> Result<(StatusCode, Json<SessionFile>), (StatusCode, String)> {
    let normalized = normalize_path(&path);

    // Paths starting with _ are reserved for actions
    if is_reserved_path(&normalized) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Paths starting with '_' are reserved for system actions".to_string(),
        ));
    }

    if req.is_directory.unwrap_or(false) {
        // Create directory
        let dir = state
            .file_service
            .create_directory(session_id, CreateDirectoryInput { path: normalized })
            .await
            .map_err(|e| {
                tracing::error!("Failed to create directory: {}", e);
                let msg = e.to_string();
                if msg.contains("file exists") || msg.contains("Invalid") {
                    (StatusCode::BAD_REQUEST, msg)
                } else if msg.contains("already exists") {
                    (StatusCode::CONFLICT, msg)
                } else {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error".to_string(),
                    )
                }
            })?;
        // Convert FileInfo to SessionFile for consistent response
        Ok((
            StatusCode::CREATED,
            Json(SessionFile {
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
            }),
        ))
    } else {
        // Create file
        let file = state
            .file_service
            .create_file(
                session_id,
                CreateFileInput {
                    path: normalized,
                    content: req.content,
                    encoding: req.encoding,
                    is_readonly: req.is_readonly,
                },
            )
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
}

/// PUT /fs/*path - Update file content
#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/fs/{path}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("path" = String, Path, description = "File path")
    ),
    request_body = UpdateFileRequest,
    responses(
        (status = 200, description = "Updated successfully", body = SessionFile),
        (status = 400, description = "Cannot modify readonly file or directory"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn update_path(
    State(state): State<AppState>,
    Path((_agent_id, session_id, path)): Path<(Uuid, Uuid, String)>,
    Json(req): Json<UpdateFileRequest>,
) -> Result<Json<SessionFile>, (StatusCode, String)> {
    let normalized = normalize_path(&path);

    // Paths starting with _ are reserved for actions
    if is_reserved_path(&normalized) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Paths starting with '_' are reserved for system actions".to_string(),
        ));
    }

    let input = UpdateFileInput {
        content: req.content,
        encoding: req.encoding,
        is_readonly: req.is_readonly,
    };

    let file = state
        .file_service
        .update_file(session_id, &normalized, input)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update file: {}", e);
            let msg = e.to_string();
            if msg.contains("readonly") || msg.contains("directory") {
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

/// DELETE /fs - Delete root (not allowed)
pub async fn delete_root() -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        "Cannot delete root directory".to_string(),
    )
}

/// DELETE /fs/*path - Delete file or directory
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/fs/{path}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID"),
        ("path" = String, Path, description = "File or directory path"),
        ("recursive" = Option<bool>, Query, description = "Delete recursively")
    ),
    responses(
        (status = 200, description = "Deleted", body = DeleteResponse),
        (status = 400, description = "Directory not empty"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn delete_path(
    State(state): State<AppState>,
    Path((_agent_id, session_id, path)): Path<(Uuid, Uuid, String)>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<DeleteResponse>, (StatusCode, String)> {
    let normalized = normalize_path(&path);

    let deleted = state
        .file_service
        .delete(session_id, &normalized, query.recursive)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete: {}", e);
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

/// POST /fs/_/move - Move/rename file
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/fs/_/move",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = MoveFileRequest,
    responses(
        (status = 200, description = "Moved successfully", body = SessionFile),
        (status = 400, description = "Invalid path"),
        (status = 404, description = "Source not found"),
        (status = 409, description = "Destination exists"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn move_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<MoveFileRequest>,
) -> Result<Json<SessionFile>, (StatusCode, String)> {
    let input = MoveFileInput {
        src_path: req.src_path,
        dst_path: req.dst_path,
    };

    let file = state
        .file_service
        .move_file(session_id, input)
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

/// POST /fs/_/copy - Copy file
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/fs/_/copy",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = CopyFileRequest,
    responses(
        (status = 201, description = "Copied successfully", body = SessionFile),
        (status = 400, description = "Cannot copy directories"),
        (status = 404, description = "Source not found"),
        (status = 409, description = "Destination exists"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn copy_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<CopyFileRequest>,
) -> Result<(StatusCode, Json<SessionFile>), (StatusCode, String)> {
    let input = CopyFileInput {
        src_path: req.src_path,
        dst_path: req.dst_path,
    };

    let file = state
        .file_service
        .copy_file(session_id, input)
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

/// POST /fs/_/grep - Search files
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/fs/_/grep",
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
    tag = "filesystem"
)]
pub async fn grep_files(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<GrepRequest>,
) -> Result<Json<ListResponse<GrepResult>>, (StatusCode, String)> {
    let input = GrepInput {
        pattern: req.pattern,
        path_pattern: req.path_pattern,
    };

    let results = state
        .file_service
        .grep(session_id, input)
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

/// POST /fs/_/stat - Get file or directory stat
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions/{session_id}/fs/_/stat",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = StatRequest,
    responses(
        (status = 200, description = "Stat info", body = FileStat),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn stat_file(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<StatRequest>,
) -> Result<Json<FileStat>, (StatusCode, String)> {
    let normalized = normalize_path(&req.path);

    let stat = state
        .file_service
        .stat(session_id, &normalized)
        .await
        .map_err(|e| {
            tracing::error!("Failed to stat: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            )
        })?
        .ok_or((StatusCode::NOT_FOUND, "Path not found".to_string()))?;

    Ok(Json(stat))
}
