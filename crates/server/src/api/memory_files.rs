// Memory virtual filesystem HTTP routes.
//
// Mounted on /v1/memories/{memory_id}/fs and mirrors the request shape used
// by the session filesystem so reusable client code is straightforward to
// share.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::memory::files::{
    GrepMatchInfo, MemoryFileRead, MemoryFileService, MemoryFsError, NewFileInput,
};
use crate::domains::memory::{MEMORY_MANAGE, MEMORY_VIEW};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use everruns_core::{Caller, Policy};
use mime_guess::from_path;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::{ErrorResponse, ListResponse, impl_auth_state};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }

    fn service(&self) -> MemoryFileService {
        MemoryFileService::new(self.db.clone())
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/memories/{memory_id}/fs",
            get(list_root).post(create_at_root),
        )
        .route("/v1/memories/{memory_id}/fs/_/stat", post(stat_action))
        .route("/v1/memories/{memory_id}/fs/_/grep", post(grep_action))
        .route(
            "/v1/memories/{memory_id}/fs/_/download/{*path}",
            get(download_action),
        )
        .route(
            "/v1/memories/{memory_id}/fs/{*path}",
            get(get_file)
                .post(create_file)
                .put(update_file)
                .delete(delete_file),
        )
        .with_state(state)
}

// ============================================
// Wire types
// ============================================

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryFileInfo {
    pub path: String,
    pub is_directory: bool,
    pub size_bytes: i64,
    pub content_hash: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemoryFile {
    pub path: String,
    /// Text or base64-encoded content; check `encoding`.
    pub content: String,
    /// "text" or "base64".
    pub encoding: String,
    pub size_bytes: i64,
    pub content_hash: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateMemoryFileRequest {
    /// File content (text or base64). Default empty.
    #[serde(default)]
    pub content: Option<String>,
    /// "text" or "base64". Defaults to "text".
    #[serde(default)]
    pub encoding: Option<String>,
    /// If true, create a directory instead of a file.
    #[serde(default)]
    pub is_directory: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateMemoryFileRequest {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub encoding: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct GrepRequest {
    /// Regex pattern to search for.
    pub pattern: String,
    /// Optional path glob (`**/*.rs`, `docs/*.md`).
    #[serde(default)]
    pub path_pattern: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
#[schema(as = MemoryGrepResult)]
pub struct GrepResultEntry {
    pub path: String,
    pub size_bytes: i64,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct StatRequest {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DeleteQuery {
    #[serde(default)]
    pub recursive: bool,
}

// ============================================
// Mapping helpers
// ============================================

fn info_to_dto(row: crate::storage::models::MemoryFileInfoRow) -> MemoryFileInfo {
    MemoryFileInfo {
        path: row.path,
        is_directory: row.is_directory,
        size_bytes: row.size_bytes,
        content_hash: row.content_hash,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn read_to_dto(read: MemoryFileRead) -> MemoryFile {
    let (content, encoding) = match std::str::from_utf8(&read.content) {
        Ok(text) if !read.content.iter().take(8192).any(|b| *b == 0) => {
            (text.to_string(), "text".to_string())
        }
        _ => (base64_encode(&read.content), "base64".to_string()),
    };
    MemoryFile {
        path: read.path,
        content,
        encoding,
        size_bytes: read.size_bytes,
        content_hash: read.content_hash,
        created_at: read.created_at,
        updated_at: read.updated_at,
    }
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_request_content(
    content: Option<String>,
    encoding: Option<String>,
) -> Result<Vec<u8>, (StatusCode, Json<ErrorResponse>)> {
    let bytes = content.unwrap_or_default();
    match encoding.as_deref().unwrap_or("text") {
        "text" => Ok(bytes.into_bytes()),
        "base64" => {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(bytes.as_bytes())
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        Json(ErrorResponse::new(format!("invalid base64 content: {e}"))),
                    )
                })
        }
        other => Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(format!(
                "unsupported encoding '{other}'"
            ))),
        )),
    }
}

fn authorize_memory_file_access(
    state: &AppState,
    org: &ResolvedOrg,
    policy: &Policy,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if !org.feature_flags.memory {
        return Err(ErrorResponse::feature_not_enabled("memory"));
    }
    let caller = Caller::from(org);
    policy
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(|e| ErrorResponse::new(e.message).into_response(StatusCode::FORBIDDEN))
}

fn err_to_response(e: MemoryFsError) -> (StatusCode, Json<ErrorResponse>) {
    let (code, msg) = match &e {
        MemoryFsError::MemoryNotFound | MemoryFsError::NotFound => {
            (StatusCode::NOT_FOUND, e.to_string())
        }
        MemoryFsError::Readonly => (StatusCode::FORBIDDEN, e.to_string()),
        MemoryFsError::InvalidPath(_) | MemoryFsError::InvalidPattern(_) => {
            (StatusCode::BAD_REQUEST, e.to_string())
        }
        MemoryFsError::Conflict(_) => (StatusCode::CONFLICT, e.to_string()),
        MemoryFsError::DirectoryNotEmpty => (StatusCode::CONFLICT, e.to_string()),
        MemoryFsError::IsDirectory | MemoryFsError::NotDirectory => {
            (StatusCode::BAD_REQUEST, e.to_string())
        }
        MemoryFsError::Other(inner) => {
            // Internal errors (e.g. SQL failures) may carry sensitive context.
            // Log server-side and surface a generic 500 to the caller.
            tracing::error!(error = %inner, "memory_files: internal error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            )
        }
    };
    (code, Json(ErrorResponse::new(msg)))
}

// ============================================
// Handlers
// ============================================

#[utoipa::path(
    description = "List immediate children of the Memory root.",
    get,
    path = "/v1/memories/{memory_id}/fs",
    params(("memory_id" = String, Path, description = "Memory ID")),
    responses(
        (status = 200, description = "Directory listing", body = ListResponse<MemoryFileInfo>),
        (status = 404, description = "Memory not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn list_root(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(memory_id): Path<String>,
) -> Result<Json<ListResponse<MemoryFileInfo>>, (StatusCode, Json<ErrorResponse>)> {
    authorize_memory_file_access(&state, &org, &MEMORY_VIEW)?;
    let svc = state.service();
    let mem = svc
        .resolve_memory(org.org_id, &memory_id)
        .await
        .map_err(err_to_response)?;
    let rows = svc
        .list_directory(mem.id, "/")
        .await
        .map_err(err_to_response)?;
    Ok(Json(ListResponse::new(
        rows.into_iter().map(info_to_dto).collect(),
    )))
}

#[utoipa::path(
    description = "Read a file or list a directory inside a Memory.",
    get,
    path = "/v1/memories/{memory_id}/fs/{path}",
    params(
        ("memory_id" = String, Path, description = "Memory ID"),
        ("path" = String, Path, description = "File or directory path"),
    ),
    responses(
        (status = 200, description = "File content or directory listing"),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn get_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((memory_id, path)): Path<(String, String)>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    authorize_memory_file_access(&state, &org, &MEMORY_VIEW)?;
    let svc = state.service();
    let mem = svc
        .resolve_memory(org.org_id, &memory_id)
        .await
        .map_err(err_to_response)?;
    // Normalize trailing slashes so `/foo/` and `/foo` resolve identically
    // before the directory pre-check.
    let normalized = {
        let with_slash = ensure_leading_slash(&path);
        if with_slash.len() > 1 && with_slash.ends_with('/') {
            with_slash.trim_end_matches('/').to_string()
        } else {
            with_slash
        }
    };
    if let Some(info) = state
        .db
        .get_memory_file_info(mem.id, &normalized)
        .await
        .map_err(internal)?
        && info.is_directory
    {
        let rows = svc
            .list_directory(mem.id, &normalized)
            .await
            .map_err(err_to_response)?;
        return Ok(Json(ListResponse::new(
            rows.into_iter().map(info_to_dto).collect(),
        ))
        .into_response());
    }
    let read = svc
        .read_file(mem.id, &normalized)
        .await
        .map_err(err_to_response)?;
    Ok(Json(read_to_dto(read)).into_response())
}

// `POST /v1/memories/{id}/fs` is intentionally unsupported and not
// documented in OpenAPI — clients must include a path. The handler exists
// so the router method returns 400 rather than 405.
pub async fn create_at_root(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(_memory_id): Path<String>,
    Json(_req): Json<CreateMemoryFileRequest>,
) -> Result<(StatusCode, Json<MemoryFileInfo>), (StatusCode, Json<ErrorResponse>)> {
    authorize_memory_file_access(&state, &org, &MEMORY_MANAGE)?;
    Err((
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse::new(
            "path required; POST to /v1/memories/{id}/fs/{path}",
        )),
    ))
}

#[utoipa::path(
    description = "Create a file or directory inside a Memory.",
    post,
    path = "/v1/memories/{memory_id}/fs/{path}",
    params(
        ("memory_id" = String, Path, description = "Memory ID"),
        ("path" = String, Path, description = "Target path"),
    ),
    request_body = CreateMemoryFileRequest,
    responses(
        (status = 201, description = "Created", body = MemoryFileInfo),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 403, description = "Source-backed Memory is read-only", body = ErrorResponse),
        (status = 409, description = "Path already exists", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn create_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((memory_id, path)): Path<(String, String)>,
    Json(req): Json<CreateMemoryFileRequest>,
) -> Result<(StatusCode, Json<MemoryFileInfo>), (StatusCode, Json<ErrorResponse>)> {
    authorize_memory_file_access(&state, &org, &MEMORY_MANAGE)?;
    let svc = state.service();
    let mem = svc
        .resolve_memory(org.org_id, &memory_id)
        .await
        .map_err(err_to_response)?;
    let is_directory = req.is_directory.unwrap_or(false);
    // Pass raw bytes through to the service; binary content must not be lossily
    // converted to UTF-8 here.
    let content = if is_directory {
        None
    } else {
        Some(decode_request_content(req.content, req.encoding)?)
    };
    let row = svc
        .create_file(
            &mem,
            NewFileInput {
                path: ensure_leading_slash(&path),
                content,
                is_directory,
            },
        )
        .await
        .map_err(err_to_response)?;
    let dto = MemoryFileInfo {
        path: row.path,
        is_directory: row.is_directory,
        size_bytes: row.size_bytes,
        content_hash: row.content_hash,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };
    Ok((StatusCode::CREATED, Json(dto)))
}

#[utoipa::path(
    description = "Update a file's content. Directories cannot be updated.",
    put,
    path = "/v1/memories/{memory_id}/fs/{path}",
    params(
        ("memory_id" = String, Path, description = "Memory ID"),
        ("path" = String, Path, description = "Target path"),
    ),
    request_body = UpdateMemoryFileRequest,
    responses(
        (status = 200, description = "Updated", body = MemoryFile),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 403, description = "Source-backed Memory is read-only", body = ErrorResponse),
        (status = 404, description = "File not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn update_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((memory_id, path)): Path<(String, String)>,
    Json(req): Json<UpdateMemoryFileRequest>,
) -> Result<Json<MemoryFile>, (StatusCode, Json<ErrorResponse>)> {
    authorize_memory_file_access(&state, &org, &MEMORY_MANAGE)?;
    let svc = state.service();
    let mem = svc
        .resolve_memory(org.org_id, &memory_id)
        .await
        .map_err(err_to_response)?;
    let bytes = decode_request_content(req.content, req.encoding)?;
    let row = svc
        .update_file(&mem, &ensure_leading_slash(&path), bytes.clone())
        .await
        .map_err(err_to_response)?;
    let read = MemoryFileRead {
        path: row.path,
        content: bytes,
        size_bytes: row.size_bytes,
        content_hash: row.content_hash,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };
    Ok(Json(read_to_dto(read)))
}

#[utoipa::path(
    description = "Delete a file or directory. Pass `recursive=true` to delete non-empty directories.",
    delete,
    path = "/v1/memories/{memory_id}/fs/{path}",
    params(
        ("memory_id" = String, Path, description = "Memory ID"),
        ("path" = String, Path, description = "Target path"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "Source-backed Memory is read-only", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 409, description = "Directory not empty", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn delete_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((memory_id, path)): Path<(String, String)>,
    Query(query): Query<DeleteQuery>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    authorize_memory_file_access(&state, &org, &MEMORY_MANAGE)?;
    let svc = state.service();
    let mem = svc
        .resolve_memory(org.org_id, &memory_id)
        .await
        .map_err(err_to_response)?;
    svc.delete(&mem, &ensure_leading_slash(&path), query.recursive)
        .await
        .map_err(err_to_response)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    description = "Stat a file or directory inside a Memory.",
    post,
    path = "/v1/memories/{memory_id}/fs/_/stat",
    params(("memory_id" = String, Path, description = "Memory ID")),
    request_body = StatRequest,
    responses(
        (status = 200, description = "File metadata", body = MemoryFileInfo),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn stat_action(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(memory_id): Path<String>,
    Json(req): Json<StatRequest>,
) -> Result<Json<MemoryFileInfo>, (StatusCode, Json<ErrorResponse>)> {
    authorize_memory_file_access(&state, &org, &MEMORY_VIEW)?;
    let svc = state.service();
    let mem = svc
        .resolve_memory(org.org_id, &memory_id)
        .await
        .map_err(err_to_response)?;
    let info = svc.stat(mem.id, &req.path).await.map_err(err_to_response)?;
    Ok(Json(info_to_dto(info)))
}

#[utoipa::path(
    description = "Search Memory files by regex.",
    post,
    path = "/v1/memories/{memory_id}/fs/_/grep",
    params(("memory_id" = String, Path, description = "Memory ID")),
    request_body = GrepRequest,
    responses(
        (status = 200, description = "Matched files", body = ListResponse<GrepResultEntry>),
        (status = 400, description = "Invalid pattern", body = ErrorResponse),
        (status = 404, description = "Memory not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn grep_action(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(memory_id): Path<String>,
    Json(req): Json<GrepRequest>,
) -> Result<Json<ListResponse<GrepResultEntry>>, (StatusCode, Json<ErrorResponse>)> {
    authorize_memory_file_access(&state, &org, &MEMORY_VIEW)?;
    let svc = state.service();
    let mem = svc
        .resolve_memory(org.org_id, &memory_id)
        .await
        .map_err(err_to_response)?;
    let rows: Vec<GrepMatchInfo> = svc
        .grep(mem.id, &req.pattern, req.path_pattern.as_deref())
        .await
        .map_err(err_to_response)?;
    Ok(Json(ListResponse::new(
        rows.into_iter()
            .map(|m| GrepResultEntry {
                path: m.path,
                size_bytes: m.size_bytes,
            })
            .collect(),
    )))
}

#[utoipa::path(
    description = "Download raw bytes of a Memory file.",
    get,
    path = "/v1/memories/{memory_id}/fs/_/download/{path}",
    params(
        ("memory_id" = String, Path, description = "Memory ID"),
        ("path" = String, Path, description = "File path"),
    ),
    responses(
        (status = 200, description = "Raw file bytes"),
        (status = 400, description = "Path is a directory", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn download_action(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((memory_id, path)): Path<(String, String)>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    authorize_memory_file_access(&state, &org, &MEMORY_VIEW)?;
    let svc = state.service();
    let mem = svc
        .resolve_memory(org.org_id, &memory_id)
        .await
        .map_err(err_to_response)?;
    let normalized = ensure_leading_slash(&path);
    let read = svc
        .read_file(mem.id, &normalized)
        .await
        .map_err(err_to_response)?;
    let mut headers = HeaderMap::new();
    if let Some(mime) = from_path(&normalized).first()
        && let Ok(value) = header::HeaderValue::from_str(mime.essence_str())
    {
        headers.insert(header::CONTENT_TYPE, value);
    }
    let filename = normalized.rsplit('/').next().unwrap_or("file").to_string();
    if let Ok(value) = header::HeaderValue::from_str(&format!(
        "attachment; filename=\"{}\"",
        filename.replace('"', "")
    )) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    Ok((headers, read.content).into_response())
}

fn ensure_leading_slash(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %e, "memory_files: internal error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new("internal server error")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthConfig;
    use everruns_core::{DefaultPermissionResolver, OrgRole, Permission, PermissionResolver};

    fn org_with_role(role: OrgRole) -> ResolvedOrg {
        ResolvedOrg {
            org_id: 1,
            public_id: "org_test".to_string(),
            name: "Test".to_string(),
            user_id: None,
            role,
            is_platform_user: false,
            feature_flags: crate::domains::common::all_feature_flags_for_test(),
        }
    }

    fn app_state() -> AppState {
        let db = Arc::new(StorageBackend::in_memory());
        let auth = AuthState::builtin(AuthConfig::default(), db.clone());
        AppState::new(db, auth)
    }

    struct DenyAllResolver;

    impl PermissionResolver for DenyAllResolver {
        fn has_permission(&self, _caller: &Caller, _permission: &Permission) -> bool {
            false
        }

        fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
            Vec::new()
        }
    }

    #[tokio::test]
    async fn create_file_requires_memory_manage_policy() {
        let result = create_file(
            org_with_role(OrgRole::Member),
            State(app_state()),
            Path(("mem_missing".to_string(), "notes.txt".to_string())),
            Json(CreateMemoryFileRequest {
                content: Some("notes".to_string()),
                encoding: None,
                is_directory: None,
            }),
        )
        .await;

        let Err((status, _)) = result else {
            panic!("member should be denied before memory file creation");
        };
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn get_file_uses_configured_permission_resolver() {
        let mut state = app_state();
        state.auth.permission_resolver = Arc::new(DenyAllResolver);

        let result = get_file(
            org_with_role(OrgRole::Owner),
            State(state),
            Path(("mem_missing".to_string(), "notes.txt".to_string())),
        )
        .await;

        let Err((status, _)) = result else {
            panic!("custom-denied caller should be denied before memory file read");
        };
        assert_eq!(status, StatusCode::FORBIDDEN);

        let default = DefaultPermissionResolver;
        assert!(
            MEMORY_VIEW
                .evaluate_with(&default, &Caller::from(&org_with_role(OrgRole::Owner)))
                .is_ok()
        );
    }
}
