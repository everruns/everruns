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

use crate::auth::{AuthState, ResolvedOrg};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::events::{
    EventContext, EventRequest, FILE_OP_CREATE, FILE_OP_UPDATE, FileWrittenData,
};
use everruns_core::typed_id::SessionId;
use everruns_core::{FileInfo, FileStat, GrepResult, SessionFile};

use super::common::{ListResponse, impl_auth_state, verify_session_ownership};
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
    pub db: Arc<StorageBackend>,
    pub event_service: Arc<crate::services::EventService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        event_service: Arc<crate::services::EventService>,
        auth: AuthState,
    ) -> Self {
        Self {
            file_service: Arc::new(SessionFileService::new(db.clone())),
            db,
            event_service,
            auth,
        }
    }

    pub fn with_virtual_registry(
        mut self,
        registry: Arc<crate::services::virtual_mount_registry::VirtualMountRegistry>,
    ) -> Self {
        self.file_service =
            Arc::new(SessionFileService::new(self.db.clone()).with_virtual_registry(registry));
        self
    }
}

impl_auth_state!(AppState);

/// Create session files routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Actions (must be before wildcard to take precedence)
        .route("/v1/sessions/{session_id}/fs/_/move", post(move_file))
        .route("/v1/sessions/{session_id}/fs/_/copy", post(copy_file))
        .route("/v1/sessions/{session_id}/fs/_/grep", post(grep_files))
        .route("/v1/sessions/{session_id}/fs/_/stat", post(stat_file))
        // File operations with path
        .route(
            "/v1/sessions/{session_id}/fs",
            get(get_root).post(create_root).delete(delete_root),
        )
        .route(
            "/v1/sessions/{session_id}/fs/{*path}",
            get(get_path)
                .post(create_path)
                .put(update_path)
                .delete(delete_path),
        )
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(state)
}

/// Workspace prefix used by capabilities (file_system, virtual_bash)
const WORKSPACE_PREFIX: &str = "/workspace";

/// Normalize path from URL, stripping /workspace prefix if present.
///
/// The file_system and virtual_bash capabilities present paths to users with
/// a /workspace prefix (e.g., /workspace/demo/a.txt), but store them without
/// the prefix (e.g., /demo/a.txt). This function handles that transformation
/// so the API accepts both formats.
///
/// Examples:
/// - "workspace" -> "/" (workspace root = storage root)
/// - "workspace/demo/a.txt" -> "/demo/a.txt"
/// - "demo/a.txt" -> "/demo/a.txt" (no prefix = pass through)
/// - "" -> "/" (empty = root)
fn normalize_path(path: &str) -> String {
    let path = path.trim_start_matches('/');
    if path.is_empty() {
        return "/".to_string();
    }

    // Add leading slash for consistent handling
    let abs_path = format!("/{}", path);

    // Strip /workspace prefix if present
    if abs_path == WORKSPACE_PREFIX {
        "/".to_string()
    } else if let Some(stripped) = abs_path.strip_prefix(WORKSPACE_PREFIX) {
        if stripped.starts_with('/') {
            stripped.to_string()
        } else {
            // /workspacefoo is not a valid workspace path
            abs_path
        }
    } else {
        abs_path
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
    path = "/v1/sessions/{session_id}/fs",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        ("recursive" = Option<bool>, Query, description = "List recursively")
    ),
    responses(
        (status = 200, description = "Directory listing"),
        (status = 400, description = "Invalid session ID"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn get_root(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<GetQuery>,
) -> Result<Json<GetResponse>, (StatusCode, String)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid session ID: {}", e),
        )
    })?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;
    get_path_impl(state, session_id.uuid(), "/", query).await
}

/// GET /fs/*path - Get file content or directory listing
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/fs/{path}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        ("path" = String, Path, description = "File or directory path"),
        ("recursive" = Option<bool>, Query, description = "List recursively")
    ),
    responses(
        (status = 200, description = "File content or directory listing"),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn get_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, path)): Path<(String, String)>,
    Query(query): Query<GetQuery>,
) -> Result<Json<GetResponse>, (StatusCode, String)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid session ID: {}", e),
        )
    })?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;
    let normalized = normalize_path(&path);
    get_path_impl(state, session_id.uuid(), &normalized, query).await
}

async fn get_path_impl(
    state: AppState,
    session_id: Uuid,
    path: &str,
    query: GetQuery,
) -> Result<Json<GetResponse>, (StatusCode, String)> {
    // Check if path is a directory or file
    let stat = state
        .file_service
        .stat(session_id, path)
        .await
        .map_err(|e| {
            tracing::error!("Failed to stat: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            )
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
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                )
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
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error".to_string(),
                    )
                })?
                .ok_or((StatusCode::NOT_FOUND, "File not found".to_string()))?;
            Ok(Json(GetResponse::File(file)))
        }
        None => {
            // For root path, return empty listing
            if path == "/" {
                Ok(Json(GetResponse::Listing(ListResponse::new(vec![]))))
            } else {
                Err((StatusCode::NOT_FOUND, "Path not found".to_string()))
            }
        }
    }
}

/// POST /fs - Create at root (not allowed)
pub async fn create_root(_org: ResolvedOrg) -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        "Cannot create at root path, specify a path".to_string(),
    )
}

/// POST /fs/*path - Create file or directory
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/fs/{path}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        ("path" = String, Path, description = "File or directory path")
    ),
    request_body = CreateFileRequest,
    responses(
        (status = 201, description = "Created successfully", body = SessionFile),
        (status = 400, description = "Invalid session ID or request"),
        (status = 409, description = "Already exists"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn create_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, path)): Path<(String, String)>,
    Json(req): Json<CreateFileRequest>,
) -> Result<(StatusCode, Json<SessionFile>), (StatusCode, String)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid session ID: {}", e),
        )
    })?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;
    let typed_session_id = session_id;
    let session_id = session_id.uuid();
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
                let msg = e.to_string();
                if msg.contains("already exists") {
                    (StatusCode::CONFLICT, msg)
                } else if msg.contains("Invalid") || msg.contains("cannot") {
                    (StatusCode::BAD_REQUEST, msg)
                } else {
                    tracing::error!("Failed to create file: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Internal server error".to_string(),
                    )
                }
            })?;

        // Emit file.written event
        let event = EventRequest::new(
            typed_session_id,
            EventContext::empty(),
            FileWrittenData {
                path: file.path.clone(),
                operation: FILE_OP_CREATE.into(),
                size_bytes: file.size_bytes,
                created: true,
            },
        );
        if let Err(e) = state.event_service.emit(event).await {
            tracing::warn!(error = %e, path = %file.path, "Failed to emit file.written event");
        }

        Ok((StatusCode::CREATED, Json(file)))
    }
}

/// PUT /fs/*path - Update file content
#[utoipa::path(
    put,
    path = "/v1/sessions/{session_id}/fs/{path}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        ("path" = String, Path, description = "File path")
    ),
    request_body = UpdateFileRequest,
    responses(
        (status = 200, description = "Updated successfully", body = SessionFile),
        (status = 400, description = "Invalid session ID or cannot modify readonly file or directory"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn update_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, path)): Path<(String, String)>,
    Json(req): Json<UpdateFileRequest>,
) -> Result<Json<SessionFile>, (StatusCode, String)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid session ID: {}", e),
        )
    })?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;
    let typed_session_id = session_id;
    let session_id = session_id.uuid();
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

    // Emit file.written event
    let event = EventRequest::new(
        typed_session_id,
        EventContext::empty(),
        FileWrittenData {
            path: file.path.clone(),
            operation: FILE_OP_UPDATE.into(),
            size_bytes: file.size_bytes,
            created: false,
        },
    );
    if let Err(e) = state.event_service.emit(event).await {
        tracing::warn!(error = %e, path = %file.path, "Failed to emit file.written event");
    }

    Ok(Json(file))
}

/// DELETE /fs - Delete root (not allowed)
pub async fn delete_root(_org: ResolvedOrg) -> (StatusCode, String) {
    (
        StatusCode::BAD_REQUEST,
        "Cannot delete root directory".to_string(),
    )
}

/// DELETE /fs/*path - Delete file or directory
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/fs/{path}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        ("path" = String, Path, description = "File or directory path"),
        ("recursive" = Option<bool>, Query, description = "Delete recursively")
    ),
    responses(
        (status = 200, description = "Deleted", body = DeleteResponse),
        (status = 400, description = "Invalid session ID or directory not empty"),
        (status = 403, description = "Cannot delete readonly file or directory containing readonly files"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn delete_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, path)): Path<(String, String)>,
    Query(query): Query<DeleteQuery>,
) -> Result<Json<DeleteResponse>, (StatusCode, String)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid session ID: {}", e),
        )
    })?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;
    let session_id = session_id.uuid();
    let normalized = normalize_path(&path);

    let deleted = state
        .file_service
        .delete(session_id, &normalized, query.recursive)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete: {}", e);
            let msg = e.to_string();
            if msg.contains("readonly") {
                (StatusCode::FORBIDDEN, msg)
            } else if msg.contains("not empty") || msg.contains("Cannot delete root") {
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
    path = "/v1/sessions/{session_id}/fs/_/move",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)")
    ),
    request_body = MoveFileRequest,
    responses(
        (status = 200, description = "Moved successfully", body = SessionFile),
        (status = 400, description = "Invalid session ID or path"),
        (status = 404, description = "Source not found"),
        (status = 409, description = "Destination exists"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn move_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<MoveFileRequest>,
) -> Result<Json<SessionFile>, (StatusCode, String)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid session ID: {}", e),
        )
    })?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;
    let session_id = session_id.uuid();
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
    path = "/v1/sessions/{session_id}/fs/_/copy",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)")
    ),
    request_body = CopyFileRequest,
    responses(
        (status = 201, description = "Copied successfully", body = SessionFile),
        (status = 400, description = "Invalid session ID or cannot copy directories"),
        (status = 404, description = "Source not found"),
        (status = 409, description = "Destination exists"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn copy_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<CopyFileRequest>,
) -> Result<(StatusCode, Json<SessionFile>), (StatusCode, String)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid session ID: {}", e),
        )
    })?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;
    let session_id = session_id.uuid();
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
    path = "/v1/sessions/{session_id}/fs/_/grep",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)")
    ),
    request_body = GrepRequest,
    responses(
        (status = 200, description = "Search results", body = ListResponse<GrepResult>),
        (status = 400, description = "Invalid session ID or regex pattern"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn grep_files(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<GrepRequest>,
) -> Result<Json<ListResponse<GrepResult>>, (StatusCode, String)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid session ID: {}", e),
        )
    })?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;
    let session_id = session_id.uuid();
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
    path = "/v1/sessions/{session_id}/fs/_/stat",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)")
    ),
    request_body = StatRequest,
    responses(
        (status = 200, description = "Stat info", body = FileStat),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn stat_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<StatRequest>,
) -> Result<Json<FileStat>, (StatusCode, String)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid session ID: {}", e),
        )
    })?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;
    let session_id = session_id.uuid();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_empty() {
        assert_eq!(normalize_path(""), "/");
    }

    #[test]
    fn test_normalize_path_root() {
        assert_eq!(normalize_path("/"), "/");
    }

    #[test]
    fn test_normalize_path_simple() {
        assert_eq!(normalize_path("demo"), "/demo");
        assert_eq!(normalize_path("/demo"), "/demo");
    }

    #[test]
    fn test_normalize_path_nested() {
        assert_eq!(normalize_path("demo/a.txt"), "/demo/a.txt");
        assert_eq!(normalize_path("/demo/a.txt"), "/demo/a.txt");
    }

    #[test]
    fn test_normalize_path_workspace_root() {
        // /workspace maps to /
        assert_eq!(normalize_path("workspace"), "/");
        assert_eq!(normalize_path("/workspace"), "/");
    }

    #[test]
    fn test_normalize_path_workspace_file() {
        // /workspace/demo/a.txt maps to /demo/a.txt
        assert_eq!(normalize_path("workspace/demo/a.txt"), "/demo/a.txt");
        assert_eq!(normalize_path("/workspace/demo/a.txt"), "/demo/a.txt");
    }

    #[test]
    fn test_normalize_path_workspace_dir() {
        // /workspace/demo maps to /demo
        assert_eq!(normalize_path("workspace/demo"), "/demo");
        assert_eq!(normalize_path("/workspace/demo"), "/demo");
    }

    #[test]
    fn test_normalize_path_workspacefoo_not_stripped() {
        // /workspacefoo is NOT a workspace path (no slash after workspace)
        assert_eq!(normalize_path("workspacefoo"), "/workspacefoo");
        assert_eq!(normalize_path("/workspacefoo"), "/workspacefoo");
    }
}
