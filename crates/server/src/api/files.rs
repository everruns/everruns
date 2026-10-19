use crate::api::state::ApiState;
use crate::auth::ResolvedOrg;
use crate::storage::CreateFileRow;
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
use everruns_contracts::typed_id::FileId;
use everruns_core::host::attachment_zip as zip_attachments;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Maximum upload size for files transferred inline over gRPC.
///
/// Base64 expands the payload by 4/3. Keeping the raw payload at 11 MiB leaves
/// more than 1 MiB of the 16 MiB gRPC message for protobuf metadata.
pub const MAX_FILE_SIZE_BYTES: usize = 11 * 1024 * 1024;

/// Maximum number of distinct files resolved for one prompt.
pub const MAX_FILES_PER_RESOLUTION: usize = 8;

/// Allowed content types for model-input files.
pub const ALLOWED_FILE_CONTENT_TYPES: &[&str] = &["application/pdf", "application/zip"];

/// File metadata returned after a successful upload (no binary data).
#[derive(Debug, Serialize, ToSchema)]
pub struct FileUploadResponse {
    #[schema(value_type = String, example = "file_01933b5a00007000800000000000001")]
    pub id: FileId,
    /// Original filename supplied at upload, if known.
    #[schema(example = "report.pdf")]
    pub filename: Option<String>,
    /// MIME type of the stored file: application/pdf or application/zip.
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
    /// MIME type of the stored file: application/pdf or application/zip.
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
/// Upload a PDF or zip file as a message attachment.
pub async fn upload_file(
    org: ResolvedOrg,
    State(state): State<ApiState>,
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
    // Windows browsers label zips with a legacy type, and some clients send
    // no type at all; the zip check below still verifies the bytes.
    let content_type = match content_type.as_str() {
        "application/x-zip-compressed" | "application/x-zip" => "application/zip".to_string(),
        "application/octet-stream" if zip_attachments::looks_like_zip(&data) => {
            "application/zip".to_string()
        }
        _ => content_type,
    };
    if !ALLOWED_FILE_CONTENT_TYPES.contains(&content_type.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Unsupported file type: {content_type}"),
        )
            .into_response());
    }
    if content_type == "application/zip" {
        // A zip is unpacked into the session's files when a message sends it,
        // so it must pass the unpack limits now: a stored zip always unpacks.
        if !zip_attachments::looks_like_zip(&data) {
            return Err(
                (StatusCode::BAD_REQUEST, "Uploaded file is not a valid zip").into_response(),
            );
        }
        if let Err(reason) = zip_attachments::unpack_zip(&data) {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Zip cannot be unpacked: {reason}"),
            )
                .into_response());
        }
    } else if !data.starts_with(b"%PDF-") {
        // Magic-byte check: PDFs start with %PDF-.
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
    State(state): State<ApiState>,
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
    State(state): State<ApiState>,
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
    // THREAT[TM-WEB-021]: stored MIME metadata is for agents, not permission to
    // execute on the authenticated origin. Attachment disposition handles
    // navigation; an inert type plus nosniff also blocks script/style loads.
    let len = row.data.len();
    let mut response = Response::new(Body::from(row.data));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/octet-stream"),
    );
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, len.into());
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        format!("attachment; filename=\"{filename}\"")
            .parse()
            .unwrap(),
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
    State(state): State<ApiState>,
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
pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/files", post(upload_file).get(list_files))
        .route("/v1/files/{file_id}", get(get_file).delete(delete_file))
        .layer(DefaultBodyLimit::max(MAX_FILE_SIZE_BYTES + 1024 * 1024))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::auth::{AuthState, config::AuthConfig};
    use crate::domains::common::all_feature_flags_for_test;
    use crate::domains::messages::intake::store_received_attachment;
    use crate::storage::StorageBackend;
    use everruns_contracts::runtime::message::ContentPart;
    use everruns_core::{OrgRole, channel_runtime::ReceivedAttachment};

    fn org(org_id: i64) -> ResolvedOrg {
        ResolvedOrg {
            org_id,
            public_id: format!("org_{org_id}"),
            name: "Example".to_string(),
            user_id: None,
            role: OrgRole::Member,
            is_platform_user: false,
            feature_flags: all_feature_flags_for_test(),
            read_only: false,
        }
    }

    fn state(db: Arc<StorageBackend>) -> ApiState {
        ApiState::for_test(
            db.clone(),
            None,
            AuthState::builtin(AuthConfig::default(), db),
        )
    }

    #[tokio::test]
    async fn stored_files_are_downloads_with_inert_http_media_type() {
        let db = Arc::new(StorageBackend::test_database());
        let state = state(db.clone());
        // MIME labels only; every fixture contains the same inert plain bytes.
        for media_type in [
            "text/html",
            "application/javascript",
            "text/javascript",
            "image/svg+xml",
            "text/xml",
            "text/css",
            "text/csv",
            "application/octet-stream",
            "application/pdf",
            "application/javascript; charset=utf-8",
            "invalid media type",
        ] {
            let bytes = b"ordinary attachment";
            let part = store_received_attachment(
                &db,
                1,
                ReceivedAttachment {
                    name: "example.txt".to_string(),
                    media_type: media_type.to_string(),
                    data: bytes.to_vec(),
                },
            )
            .await
            .unwrap();
            let ContentPart::File(file) = part else {
                panic!("expected a file");
            };
            let row = db.get_file(1, file.file_id.uuid()).await.unwrap().unwrap();
            assert_eq!(row.content_type, media_type);
            assert_eq!(row.data, bytes);

            let request = axum::http::Request::builder()
                .uri(format!("/v1/files/{}", file.file_id))
                .body(Body::empty())
                .unwrap();
            let response = routes(state.clone()).oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "application/octet-stream",
                "{media_type}"
            );
            assert_eq!(
                response.headers()[header::CONTENT_DISPOSITION],
                "attachment; filename=\"example.txt\""
            );
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            assert_eq!(
                response.headers()[header::CONTENT_LENGTH],
                bytes.len().to_string()
            );
            assert_eq!(
                axum::body::to_bytes(response.into_body(), 64)
                    .await
                    .unwrap(),
                bytes.as_slice()
            );
            let denied = get_file(org(2), State(state.clone()), Path(file.file_id.to_string()))
                .await
                .err()
                .expect("another org cannot read the attachment");
            assert_eq!(denied.status(), StatusCode::NOT_FOUND);
        }
    }

    #[tokio::test]
    async fn download_route_requires_authentication() {
        let db = Arc::new(StorageBackend::test_database());
        let auth = AuthState::builtin(
            AuthConfig {
                mode: crate::auth::config::AuthMode::Full,
                ..AuthConfig::default()
            },
            db.clone(),
        );
        let state = ApiState::for_test(db, None, auth);
        let request = axum::http::Request::builder()
            .uri(format!("/v1/files/{}", FileId::new()))
            .body(Body::empty())
            .unwrap();
        let response = routes(state).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn download_filename_remains_a_single_bounded_header_value() {
        let db = Arc::new(StorageBackend::test_database());
        let state = state(db.clone());
        let long = "a".repeat(205);
        let bounded = "a".repeat(200);
        for (filename, expected) in [
            (Some("report.pdf"), "report.pdf"),
            (Some("a\"b\\c\r\nd.txt"), "abcd.txt"),
            (Some("\"\\\r\n"), "file.pdf"),
            (None, "file.pdf"),
            (Some(long.as_str()), bounded.as_str()),
        ] {
            let row = db
                .create_file(
                    1,
                    CreateFileRow {
                        org_id: 1,
                        filename: filename.map(str::to_string),
                        content_type: "application/zip".to_string(),
                        size_bytes: 4,
                        data: b"data".to_vec(),
                        metadata: serde_json::json!({}),
                    },
                )
                .await
                .unwrap();
            let response = get_file(org(1), State(state.clone()), Path(row.id.to_string()))
                .await
                .unwrap()
                .into_response();
            assert_eq!(
                response.headers()[header::CONTENT_DISPOSITION],
                format!("attachment; filename=\"{expected}\"")
            );
            assert_eq!(
                response
                    .headers()
                    .get_all(header::CONTENT_DISPOSITION)
                    .iter()
                    .count(),
                1
            );
        }
    }
}
