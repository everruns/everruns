use super::common::impl_auth_state;
use crate::auth::{AuthState, ResolvedOrg};
use crate::storage::{StorageBackend, models::CreateFileRow};
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::{
    Json, Router,
    routing::{get, post},
};
use axum_extra::extract::Multipart;
use chrono::{DateTime, Utc};
use everruns_provider::typed_id::FileId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// App state for files routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }
}

impl_auth_state!(AppState);

/// Maximum upload size: 32 MiB (matches the Anthropic document limit).
pub const MAX_FILE_SIZE_BYTES: usize = 32 * 1024 * 1024;

/// Allowed content types for model-input files.
pub const ALLOWED_FILE_CONTENT_TYPES: &[&str] = &["application/pdf"];

/// File metadata returned after a successful upload (no binary data).
#[derive(Debug, Serialize, ToSchema)]
pub struct FileUploadResponse {
    #[schema(value_type = String, example = "file_01933b5a00007000800000000000001")]
    pub id: FileId,
    /// Original filename supplied at upload, if known.
    #[schema(example = "report.pdf")]
    pub filename: Option<String>,
    /// MIME type of the stored file (currently always application/pdf).
    #[schema(example = "application/pdf")]
    pub content_type: String,
    /// Size of the stored file in bytes.
    #[schema(example = 1048576)]
    pub size_bytes: i64,
    /// Upload timestamp.
    #[schema(example = "2026-01-04T11:23:00Z")]
    pub created_at: DateTime<Utc>,
}

/// Stored file metadata (no binary data).
#[derive(Debug, Serialize, ToSchema)]
pub struct FileInfo {
    #[schema(value_type = String, example = "file_01933b5a00007000800000000000001")]
    pub id: FileId,
    /// Original filename supplied at upload, if known.
    #[schema(example = "report.pdf")]
    pub filename: Option<String>,
    /// MIME type of the stored file (currently always application/pdf).
    #[schema(example = "application/pdf")]
    pub content_type: String,
    /// Size of the stored file in bytes.
    #[schema(example = 1048576)]
    pub size_bytes: i64,
    /// Caller-supplied metadata captured at upload.
    #[schema(value_type = Object, example = json!({"source": "chat-upload"}))]
    pub metadata: serde_json::Value,
    /// Upload timestamp.
    #[schema(example = "2026-01-04T11:23:00Z")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct UploadFileQuery {
    /// Optional session to attribute the upload to.
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListFilesQuery {
    /// Maximum number of files to return.
    pub limit: Option<i64>,
}

#[utoipa::path(
    post,
    path = "/v1/files",
    params(UploadFileQuery),
    request_body(content = String, content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "File uploaded", body = FileUploadResponse),
        (status = 400, description = "Invalid file"),
        (status = 413, description = "File too large"),
    ),
    tag = "Files"
)]
/// Upload a PDF file for use as model input.
pub async fn upload_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    query: Query<UploadFileQuery>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, Response> {
    let mut filename: Option<String> = None;
    let mut content_type: Option<String> = None;
    let mut data: Vec<u8> = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid multipart body: {e}"),
        )
            .into_response()
    })? {
        let name = field.name().unwrap_or("").to_string();
        if name != "file" {
            continue;
        }
        filename = field.file_name().map(|s| s.to_string());
        content_type = field.content_type().map(|s| s.to_string());
        data = field
            .bytes()
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Failed to read upload: {e}"),
                )
                    .into_response()
            })?
            .to_vec();
    }

    if data.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "Missing 'file' field").into_response());
    }
    if data.len() > MAX_FILE_SIZE_BYTES {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("File exceeds {MAX_FILE_SIZE_BYTES} bytes"),
        )
            .into_response());
    }
    let content_type = content_type.unwrap_or_else(|| "application/octet-stream".to_string());
    if !ALLOWED_FILE_CONTENT_TYPES.contains(&content_type.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Unsupported file type: {content_type}"),
        )
            .into_response());
    }
    // Magic-byte check: PDFs start with %PDF-.
    if !data.starts_with(b"%PDF-") {
        return Err((
            StatusCode::BAD_REQUEST,
            "Uploaded file is not a valid PDF (missing %PDF- header)",
        )
            .into_response());
    }

    let mut metadata = serde_json::json!({});
    if let Some(session_id) = &query.session_id {
        metadata["session_id"] = serde_json::json!(session_id);
    }

    let row = state
        .db
        .create_file(
            org.org_id,
            CreateFileRow {
                org_id: org.org_id,
                filename: filename.clone(),
                content_type: content_type.clone(),
                size_bytes: data.len() as i64,
                data,
                metadata,
            },
        )
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to store file: {e}"),
            )
                .into_response()
        })?;

    Ok((
        StatusCode::CREATED,
        Json(FileUploadResponse {
            id: row.id,
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/v1/files",
    params(ListFilesQuery),
    responses((status = 200, description = "List files", body = Vec<FileInfo>)),
    tag = "Files"
)]
/// List uploaded files, newest first.
pub async fn list_files(
    org: ResolvedOrg,
    State(state): State<AppState>,
    query: Query<ListFilesQuery>,
) -> Result<impl IntoResponse, Response> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let rows = state.db.list_files(org.org_id, limit).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to list files: {e}"),
        )
            .into_response()
    })?;
    Ok(Json(
        rows.into_iter()
            .map(|r| FileInfo {
                id: r.id,
                filename: r.filename,
                content_type: r.content_type,
                size_bytes: r.size_bytes,
                metadata: r.metadata,
                created_at: r.created_at,
            })
            .collect::<Vec<_>>(),
    ))
}

#[utoipa::path(
    get,
    path = "/v1/files/{file_id}",
    responses((status = 200, description = "File bytes")),
    tag = "Files"
)]
/// Download a stored file's bytes.
pub async fn get_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    path: Path<String>,
) -> Result<impl IntoResponse, Response> {
    let file_id = parse_file_id(&path.0)?;
    let row = state
        .db
        .get_file(org.org_id, file_id.uuid())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to load file: {e}"),
            )
                .into_response()
        })?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "File not found").into_response())?;
    // Sanitize the filename for the Content-Disposition header: filenames are
    // user-controlled, so strip quotes and control characters (header injection).
    let filename: String = row
        .filename
        .as_deref()
        .unwrap_or("file.pdf")
        .chars()
        .filter(|c| !c.is_control() && *c != '"' && *c != '\\')
        .take(200)
        .collect();
    let filename = if filename.is_empty() {
        "file.pdf".to_string()
    } else {
        filename
    };
    let len = row.data.len();
    let mut response = Response::new(Body::from(row.data));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, row.content_type.parse().unwrap());
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, len.into());
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        format!("inline; filename=\"{filename}\"").parse().unwrap(),
    );
    response
        .headers_mut()
        .insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    Ok(response)
}

#[utoipa::path(
    delete,
    path = "/v1/files/{file_id}",
    responses((status = 200, description = "File deleted")),
    tag = "Files"
)]
/// Delete a stored file.
pub async fn delete_file(
    org: ResolvedOrg,
    State(state): State<AppState>,
    path: Path<String>,
) -> Result<impl IntoResponse, Response> {
    let file_id = parse_file_id(&path.0)?;
    let deleted = state
        .db
        .delete_file(org.org_id, file_id.uuid())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to delete file: {e}"),
            )
                .into_response()
        })?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, "File not found").into_response());
    }
    Ok(Json(serde_json::json!({ "deleted": true })))
}

fn parse_file_id(raw: &str) -> Result<FileId, Response> {
    raw.parse::<FileId>()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid file id").into_response())
}

/// Create files router.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/files", post(upload_file).get(list_files))
        .route("/v1/files/{file_id}", get(get_file).delete(delete_file))
        .layer(DefaultBodyLimit::max(MAX_FILE_SIZE_BYTES + 1024 * 1024))
        .with_state(state)
}
