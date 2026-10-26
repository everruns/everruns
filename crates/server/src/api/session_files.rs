// Session Files (Virtual Filesystem) HTTP routes — DEPRECATED.
//
// These `/v1/sessions/{session_id}/fs/*` routes are retained as aliases for
// backwards compatibility. New clients should use the equivalent
// `/v1/workspaces/{workspace_id}/fs/*` endpoints instead. The routes are
// intentionally delisted from the published OpenAPI surface (see
// `crates/server/src/openapi/mod.rs`) so they do not appear in `docs/api/openapi.json`.
//
// RESTful API design:
// - GET    /fs/*path  - Read file content or list directory
// - POST   /fs/*path  - Create file or directory
// - PUT    /fs/*path  - Update file content
// - DELETE /fs/*path  - Delete file or directory
// - POST   /fs/_/move - Move/rename file
// - POST   /fs/_/copy - Copy file
// - POST   /fs/_/grep - Search files
// - POST   /fs/_/search - Search files with context and paging
// - POST   /fs/_/stat - Get file metadata
// - GET    /fs/_/download/*path - Download raw file bytes
//
// Note: Paths starting with "_" are reserved for actions and cannot be
// used for file creation or updates.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, CommandError, CommandErrorKind, Ctx};
pub use crate::domains::session_files::types::{
    CopyFileRequest, CreateFileRequest, DeleteQuery, DeleteResponse, GetQuery, GetResponse,
    GrepRequest, MoveFileRequest, SearchRequest, StatRequest, UpdateFileRequest,
};
use crate::domains::session_files::{
    CopyWorkspaceFile, CreateWorkspaceFile, DeleteWorkspaceFile, GetWorkspaceFile,
    GrepWorkspaceFiles, ListWorkspaceFiles, MoveWorkspaceFile, SearchWorkspaceFiles,
    ByteRangeRequest, RangeNotSatisfiable, StatWorkspaceFile, UpdateWorkspaceFile,
    WorkspaceFileService, queries as q,
};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use everruns_core::Caller;
use everruns_core::{FileStat, GrepResult, GrepSearchResult, SessionFile};

use super::common::{ListResponse, impl_auth_state};
use std::sync::Arc;

/// App state for session files routes
#[derive(Clone)]
pub struct AppState {
    pub file_service: Arc<WorkspaceFileService>,
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
            file_service: Arc::new(WorkspaceFileService::new(db.clone())),
            db,
            event_service,
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

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_session_file_service(self.file_service.clone())
        .with_event_service(self.event_service.clone())
    }
}

impl_auth_state!(AppState);

fn file_error(error: CommandError) -> (StatusCode, String) {
    let status = error.status();
    match error {
        CommandError {
            kind: CommandErrorKind::Internal(inner),
            ..
        } => {
            tracing::error!("Session files command failed: {inner}");
            (status, "Internal server error".to_string())
        }
        other => (status, other.to_string()),
    }
}

pub(crate) fn wants_raw_file(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|accept| {
            accept
                .split(',')
                .filter_map(parse_accept_entry)
                .any(|(entry, quality)| {
                    entry.eq_ignore_ascii_case("application/octet-stream") && quality > 0.0
                })
        })
        .unwrap_or(false)
}

fn parse_accept_entry(entry: &str) -> Option<(&str, f32)> {
    let mut parts = entry.split(';');
    let media_type = parts.next()?.trim();
    if media_type.is_empty() {
        return None;
    }

    let mut quality = 1.0_f32;
    for param in parts {
        let mut key_value = param.splitn(2, '=');
        let key = key_value.next().unwrap_or("").trim();
        let value = key_value.next().unwrap_or("").trim();

        if key.eq_ignore_ascii_case("q") {
            if let Ok(parsed) = value.parse::<f32>() {
                quality = parsed;
            }
            break;
        }
    }

    Some((media_type, quality))
}

fn ascii_filename_fallback(filename: &str) -> String {
    let sanitized: String = filename
        .chars()
        .map(|ch| match ch {
            '"' | '\\' | '/' | ';' => '_',
            ' '..='~' => ch,
            _ => '_',
        })
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.to_string()
    }
}

fn content_disposition_value(filename: &str, attachment: bool) -> String {
    let disposition = if attachment { "attachment" } else { "inline" };
    let fallback = ascii_filename_fallback(filename);
    let encoded = urlencoding::encode(filename);
    format!("{disposition}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

/// The single byte range a request asks for, if any.
fn requested_range(headers: &HeaderMap) -> Option<ByteRangeRequest> {
    headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(ByteRangeRequest::parse)
}

/// Whether an `If-Range` validator (when present) still matches `etag`. A
/// stale or date-based validator means the client must get the whole file.
fn if_range_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_RANGE)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|validator| validator.trim() == etag)
}

/// Stream a file as raw bytes, honoring `Range` / `If-Range`.
///
/// Answers 200 with the whole file, 206 with one range, or 416 when the range
/// lies outside the file. The body streams from storage, so large recordings
/// are never held in memory, and video players can seek.
pub(crate) async fn stream_file_response(
    service: &WorkspaceFileService,
    key: uuid::Uuid,
    path: &str,
    headers: &HeaderMap,
    attachment: bool,
) -> Result<Response, (StatusCode, String)> {
    let open = |range| async move {
        match service.open_file(key, path, range).await {
            Ok(Some(file)) => Ok(Ok(file)),
            Ok(None) => Err((StatusCode::NOT_FOUND, "File not found".to_string())),
            Err(error) => match error.downcast_ref::<RangeNotSatisfiable>() {
                Some(unsatisfiable) => Ok(Err(unsatisfiable.size)),
                None => {
                    tracing::error!(%path, %error, "Failed to open session file");
                    Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to read file".to_string(),
                    ))
                }
            },
        }
    };

    let range = requested_range(headers);
    let mut opened = open(range).await?;
    if let Ok(file) = &opened
        && range.is_some()
        && !if_range_matches(headers, &file.etag)
    {
        opened = open(None).await?;
    }
    let file = match opened {
        Ok(file) => file,
        Err(size) => {
            return Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                .header(header::ACCEPT_RANGES, "bytes")
                .body(Body::empty())
                .map_err(build_error);
        }
    };

    let mut response = Response::builder()
        .header(header::CONTENT_TYPE, &file.content_type)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, &file.etag)
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition_value(&file.name, attachment),
        );
    response = match &file.range {
        Some(range) => response
            .status(StatusCode::PARTIAL_CONTENT)
            .header(
                header::CONTENT_RANGE,
                format!("bytes {}-{}/{}", range.start, range.end - 1, file.size),
            )
            .header(header::CONTENT_LENGTH, range.end - range.start),
        None => response
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, file.size),
    };
    response
        .body(Body::from_stream(file.body))
        .map_err(build_error)
}

fn build_error(error: axum::http::Error) -> (StatusCode, String) {
    tracing::error!(%error, "Failed to build raw file response");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to build file response".to_string(),
    )
}

/// CSP for the sandboxed HTML preview response (TM-WEB-010).
///
/// The `sandbox` directive WITHOUT `allow-same-origin` forces an opaque origin,
/// so the previewed document — even though served same-origin and fetched with
/// the user's auth cookies — cannot read everruns cookies, storage, or the
/// parent DOM. Omitting `allow-top-navigation`/`allow-forms`/`allow-popups`
/// blocks redirects, form posts, and popups; `allow-scripts` lets the page run
/// JavaScript. Network fetches are denied by omitting remote URL schemes from
/// every fetch directive: untrusted preview content may execute, but it cannot
/// exfiltrate the rendered document to attacker-controlled endpoints. Inline
/// scripts/styles and local `data:`/`blob:` resources remain available for
/// self-contained reports; `object-src 'none'` blocks plugins and `base-uri
/// 'none'` blocks `<base>` hijacking.
///
/// Crucially this is delivered as a real network response: unlike a `srcdoc`/
/// `data:` iframe — which inherits the app's strict `script-src 'self'` and thus
/// cannot run inline scripts — a network response carries its own CSP, which is
/// what lets the preview execute JavaScript at all.
const SANDBOXED_HTML_PREVIEW_CSP: &str = "sandbox allow-scripts; default-src 'none'; \
script-src 'unsafe-inline' 'unsafe-eval' data: blob:; \
style-src 'unsafe-inline' data: blob:; img-src data: blob:; \
font-src data: blob:; media-src data: blob:; connect-src 'none'; \
object-src 'none'; base-uri 'none'";

/// True when `path` names an HTML document eligible for sandboxed preview.
pub(crate) fn is_html_preview_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".html") || lower.ends_with(".htm")
}

/// Serve a file's bytes as a sandboxed `text/html` document (TM-WEB-010).
///
/// Sets `X-Frame-Options: SAMEORIGIN` so the file-viewer iframe can embed it —
/// the global response layer would otherwise apply `DENY` (`if_not_present`) and
/// block even same-origin framing. The CSP `sandbox` directive is the real
/// isolation boundary; `X-Frame-Options` only governs who may frame it.
pub(crate) fn sandboxed_html_response(file: SessionFile) -> Result<Response, (StatusCode, String)> {
    let content = file.content.as_ref().ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "File content missing".to_string(),
    ))?;
    let bytes = SessionFile::decode_content(content, &file.encoding).map_err(|error| {
        tracing::error!(path = %file.path, %error, "Failed to decode HTML preview content");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to decode file content".to_string(),
        )
    })?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CONTENT_SECURITY_POLICY, SANDBOXED_HTML_PREVIEW_CSP)
        .header(header::X_FRAME_OPTIONS, "SAMEORIGIN")
        .body(Body::from(bytes))
        .map_err(|error| {
            tracing::error!(path = %file.path, %error, "Failed to build HTML preview response");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to build file response".to_string(),
            )
        })
}

/// Create session files routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Actions (must be before wildcard to take precedence)
        .route("/v1/sessions/{session_id}/fs/_/move", post(move_file))
        .route("/v1/sessions/{session_id}/fs/_/copy", post(copy_file))
        .route("/v1/sessions/{session_id}/fs/_/grep", post(grep_files))
        .route("/v1/sessions/{session_id}/fs/_/search", post(search_files))
        .route("/v1/sessions/{session_id}/fs/_/stat", post(stat_file))
        .route(
            "/v1/sessions/{session_id}/fs/_/download/{*path}",
            get(download_path),
        )
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
    let response = ListWorkspaceFiles {
        session_id,
        recursive: query.recursive,
    }
    .run(&state.ctx(&org))
    .await
    .map_err(file_error)?;
    Ok(Json(response))
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
    headers: HeaderMap,
    Query(query): Query<GetQuery>,
) -> Result<Response, (StatusCode, String)> {
    if wants_raw_file(&headers) {
        return stream_session_file(&state, &org, session_id, path, &headers, false).await;
    }

    let response = GetWorkspaceFile {
        session_id,
        path,
        recursive: query.recursive,
    }
    .run(&state.ctx(&org))
    .await
    .map_err(file_error)?;

    Ok(Json(response).into_response())
}

/// Authorize a raw read of a session file, then stream it.
async fn stream_session_file(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: String,
    path: String,
    headers: &HeaderMap,
    attachment: bool,
) -> Result<Response, (StatusCode, String)> {
    let ctx = state.ctx(org);
    // The stat command carries the read policy, the session check and the
    // private-memory check; the stream below only runs after it passed.
    let stat = StatWorkspaceFile {
        session_id: session_id.clone(),
        req: StatRequest { path: path.clone() },
    }
    .run(&ctx)
    .await
    .map_err(file_error)?;
    if stat.is_directory {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot download a directory".to_string(),
        ));
    }
    let session_id = q::parse_session_id(&session_id).map_err(file_error)?;
    let access = q::verify_session(&ctx, session_id)
        .await
        .map_err(file_error)?;
    let path = q::normalize_path(&path);
    stream_file_response(
        &q::service(&ctx),
        access.read_key(session_id, &path),
        &path,
        headers,
        attachment,
    )
    .await
}

/// GET /fs/_/download/*path - Download raw file bytes
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/fs/_/download/{path}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., sess_...)"),
        ("path" = String, Path, description = "File path")
    ),
    responses(
        (status = 200, description = "Raw file bytes", content_type = "application/octet-stream"),
        (status = 206, description = "The byte range named by the `Range` header", content_type = "application/octet-stream"),
        (status = 400, description = "Invalid session ID or path points to a directory"),
        (status = 416, description = "The `Range` header lies outside the file"),
        (status = 404, description = "File not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "filesystem"
)]
pub async fn download_path(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    stream_session_file(&state, &org, session_id, path, &headers, true).await
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
    let file = CreateWorkspaceFile {
        session_id,
        path,
        req,
    }
    .run(&state.ctx(&org))
    .await
    .map_err(file_error)?;
    Ok((StatusCode::CREATED, Json(file)))
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
    let file = UpdateWorkspaceFile {
        session_id,
        path,
        req,
    }
    .run(&state.ctx(&org))
    .await
    .map_err(file_error)?;
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
    let response = DeleteWorkspaceFile {
        session_id,
        path,
        recursive: query.recursive,
    }
    .run(&state.ctx(&org))
    .await
    .map_err(file_error)?;
    Ok(Json(response))
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
    let file = MoveWorkspaceFile { session_id, req }
        .run(&state.ctx(&org))
        .await
        .map_err(file_error)?;
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
    let file = CopyWorkspaceFile { session_id, req }
        .run(&state.ctx(&org))
        .await
        .map_err(file_error)?;
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
    let results = GrepWorkspaceFiles { session_id, req }
        .run(&state.ctx(&org))
        .await
        .map_err(file_error)?;
    Ok(Json(ListResponse::new(results)))
}

/// POST /fs/_/search - Search files with context and paging
///
/// Deliberately carries no utoipa path annotation. This whole
/// `/v1/sessions/{session_id}/fs/*` family is delisted from the published spec
/// so the annotation would describe an
/// operation `ApiDoc` never registers — which
/// `openapi_coverage_test::every_utoipa_handler_is_registered_in_apidoc`
/// rejects. (The guard scans for that attribute as plain text, so naming it
/// literally here would re-trip it on this very comment.) The sibling handlers
/// here keep theirs only because
/// `api::workspace_files` happens to expose functions of the same name, and
/// the guard matches on the function name; `search_files` has no such twin.
///
/// The canonical home for this capability is the workspace surface. That is
/// not a rename: `workspace_files::grep_files` pre-excludes `/memory/user`
/// through `GrepInput::excluded_path_prefix` because it cannot establish
/// session ownership, and `GrepOptions` carries no equivalent — so a workspace
/// search would need that threaded through the service first, or its match
/// counts would betray private files it correctly refuses to return.
pub async fn search_files(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<SearchRequest>,
) -> Result<Json<GrepSearchResult>, (StatusCode, String)> {
    let result = SearchWorkspaceFiles { session_id, req }
        .run(&state.ctx(&org))
        .await
        .map_err(file_error)?;
    Ok(Json(result))
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
    let stat = StatWorkspaceFile { session_id, req }
        .run(&state.ctx(&org))
        .await
        .map_err(file_error)?;
    Ok(Json(stat))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_html_preview_path() {
        assert!(is_html_preview_path("/index.html"));
        assert!(is_html_preview_path("/a/b/page.HTM"));
        assert!(is_html_preview_path("/Report.HTML"));
        assert!(!is_html_preview_path("/notes.md"));
        assert!(!is_html_preview_path("/script.js"));
        assert!(!is_html_preview_path("/htmlfile")); // no extension
        assert!(!is_html_preview_path("/a.html.txt"));
    }

    #[test]
    fn test_sandboxed_html_preview_csp_directives() {
        // The CSP must sandbox into an opaque origin with scripts allowed, and
        // must NOT grant same-origin/top-navigation/forms/popups (TM-WEB-010).
        let csp = SANDBOXED_HTML_PREVIEW_CSP;
        assert!(csp.contains("sandbox allow-scripts"));
        assert!(!csp.contains("allow-same-origin"));
        assert!(!csp.contains("allow-top-navigation"));
        assert!(!csp.contains("allow-forms"));
        assert!(!csp.contains("allow-popups"));
        assert!(csp.contains("object-src 'none'"));
        assert!(csp.contains("base-uri 'none'"));
        assert!(csp.contains("connect-src 'none'"));
        assert!(csp.contains("script-src 'unsafe-inline' 'unsafe-eval' data: blob:"));
        assert!(csp.contains("img-src data: blob:"));
        assert!(!csp.contains("https:"));
        assert!(!csp.contains("http:"));
    }

    #[test]
    fn test_sandboxed_html_response_headers() {
        let now = chrono::Utc::now();
        let file = SessionFile {
            id: uuid::Uuid::nil(),
            session_id: uuid::Uuid::nil(),
            path: "/index.html".to_string(),
            name: "index.html".to_string(),
            content: Some("<h1>hi</h1>".to_string()),
            encoding: "text".to_string(),
            is_directory: false,
            is_readonly: false,
            size_bytes: 11,
            created_at: now,
            updated_at: now,
        };
        let resp = sandboxed_html_response(file).expect("response builds");
        let headers = resp.headers();
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        // SAMEORIGIN is required so the global X-Frame-Options: DENY does not
        // block the file-viewer iframe from embedding this preview.
        assert_eq!(headers.get(header::X_FRAME_OPTIONS).unwrap(), "SAMEORIGIN");
        let csp = headers
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("sandbox allow-scripts"));
        assert!(!csp.contains("allow-same-origin"));
    }

    #[test]
    fn test_wants_raw_file_honors_q_zero() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            "application/octet-stream;q=0, application/json"
                .parse()
                .unwrap(),
        );
        assert!(!wants_raw_file(&headers));
    }

    #[test]
    fn test_wants_raw_file_accepts_positive_quality() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::ACCEPT,
            "application/json, application/octet-stream;q=0.5"
                .parse()
                .unwrap(),
        );
        assert!(wants_raw_file(&headers));
    }

    #[test]
    fn test_content_disposition_value_uses_ascii_fallback_and_filename_star() {
        let value = content_disposition_value("report \"Δ\".pdf", true);
        assert!(value.starts_with("attachment; "));
        assert!(value.contains("filename=\"report ___.pdf\""));
        assert!(value.contains("filename*=UTF-8''report%20%22%CE%94%22.pdf"));
    }
}
