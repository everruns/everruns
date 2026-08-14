// Image upload HTTP routes
// Routes: /v1/images/...
//
// Organization image storage for message attachments.
// Supports PNG, JPEG, GIF, WebP (OpenAI Vision compatible formats).
// Generates thumbnails on upload for efficient display.

use super::common::impl_auth_state;
use crate::auth::{AuthState, ResolvedOrg};
use crate::storage::{StorageBackend, models::CreateImageRow};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    response::Response,
    routing::{get, post},
};
use axum_extra::extract::Multipart;
use chrono::{DateTime, Utc};
use everruns_provider::typed_id::{ImageId, SessionId};
use image::ImageFormat;
use image::ImageReader;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::Arc;
use utoipa::ToSchema;

// ============================================
// Constants
// ============================================

/// Maximum image size in bytes (100 MB)
pub(crate) const MAX_IMAGE_SIZE: usize = 100 * 1024 * 1024;

/// Thumbnail max dimension (width or height)
const THUMBNAIL_MAX_DIM: u32 = 200;
const MAX_IMAGE_PIXELS: u64 = 40_000_000;

/// Allowed MIME types
pub(crate) const ALLOWED_CONTENT_TYPES: &[&str] =
    &["image/png", "image/jpeg", "image/gif", "image/webp"];

// ============================================
// API Types
// ============================================

/// Image metadata (without binary data)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageInfo {
    #[schema(value_type = String, example = "img_01933b5a00007000800000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: ImageId,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    /// Free-form metadata attached to this resource.
    pub metadata: serde_json::Value,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
}

/// Image upload response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageUploadResponse {
    #[schema(value_type = String, example = "img_01933b5a00007000800000000000001")]
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: ImageId,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    /// Timestamp when this resource was created (RFC 3339).
    pub created_at: DateTime<Utc>,
}

/// Query parameters for image list
#[derive(Debug, Deserialize)]
pub struct ListImagesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// Query parameters for image upload
#[derive(Debug, Deserialize)]
pub struct UploadImageQuery {
    /// Optional session ID stored as metadata for tracking (not required for upload)
    pub session_id: Option<SessionId>,
}

// ============================================
// App State and Routes
// ============================================

/// App state for images routes
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

/// Create images routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Upload needs larger body limit (100MB + some overhead for multipart encoding)
        .route(
            "/v1/images",
            post(upload_image)
                .layer(DefaultBodyLimit::max(MAX_IMAGE_SIZE + 1024 * 1024))
                .get(list_images),
        )
        .route("/v1/images/{image_id}", get(get_image).delete(delete_image))
        .route("/v1/images/{image_id}/thumbnail", get(get_thumbnail))
        .with_state(state)
}

// ============================================
// Helper Functions
// ============================================

/// Strip MIME parameters (`; charset=...`) and lowercase the bare media type
/// so callers tolerate common header variations.
fn normalize_content_type(content_type: &str) -> String {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Validate content type is allowed
pub(crate) fn is_valid_content_type(content_type: &str) -> bool {
    ALLOWED_CONTENT_TYPES.contains(&normalize_content_type(content_type).as_str())
}

/// Validate uploaded bytes are a real image matching declared content type.
pub(crate) fn validate_image_bytes(data: &[u8], content_type: &str) -> bool {
    let expected_format = match format_from_content_type(content_type) {
        Some(format) => format,
        None => return false,
    };

    image::guess_format(data).is_ok_and(|format| format == expected_format)
}

/// Detect image format from content type
fn format_from_content_type(content_type: &str) -> Option<ImageFormat> {
    match normalize_content_type(content_type).as_str() {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/gif" => Some(ImageFormat::Gif),
        "image/webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

/// Generate thumbnail from image data
pub(crate) fn generate_thumbnail(data: &[u8], content_type: &str) -> Option<(Vec<u8>, String)> {
    let format = format_from_content_type(content_type)?;

    // Load image with allocation limits to avoid decompression bombs.
    //
    // - max_alloc binds the decoder's working memory to MAX_IMAGE_PIXELS * 4 bytes
    //   (assumes RGBA8, the worst case for the formats we accept).
    // - max_image_width/height cap any single dimension well below the area
    //   implied by MAX_IMAGE_PIXELS so an extreme aspect ratio (e.g. 1 x 4B)
    //   is rejected at header parse before allocation work happens.
    let max_dim = (MAX_IMAGE_PIXELS as f64).sqrt() as u32;
    let mut reader = ImageReader::with_format(Cursor::new(data), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(max_dim);
    limits.max_image_height = Some(max_dim);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 4);
    reader.limits(limits);
    let img = reader.decode().ok()?;

    // Resize to thumbnail
    let thumbnail = img.thumbnail(THUMBNAIL_MAX_DIM, THUMBNAIL_MAX_DIM);

    // Encode as JPEG for smaller size (thumbnails don't need transparency)
    let mut output = Cursor::new(Vec::new());
    thumbnail.write_to(&mut output, ImageFormat::Jpeg).ok()?;

    Some((output.into_inner(), "image/jpeg".to_string()))
}

// ============================================
// HTTP Handlers
// ============================================

/// POST /v1/images - Upload an image
#[utoipa::path(
    post,
    path = "/v1/images",
    params(
        ("session_id" = Option<String>, Query, description = "Optional: session ID stored as metadata for tracking (not required for upload)")
    ),
    request_body(content = String, description = "Multipart form data with 'file' field", content_type = "multipart/form-data"),
    responses(
        (status = 201, description = "Image uploaded successfully", body = ImageUploadResponse),
        (status = 400, description = "Invalid request (bad format, size exceeded, etc.)"),
        (status = 500, description = "Internal server error")
    ),
    tag = "images"
)]
pub async fn upload_image(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<UploadImageQuery>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<ImageUploadResponse>), (StatusCode, String)> {
    // Extract file from multipart
    let mut file_data: Option<(String, String, Vec<u8>)> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Failed to parse multipart: {}", e),
        )
    })? {
        let name: String = field.name().unwrap_or("").to_string();

        if name == "file" {
            let filename: String = field.file_name().unwrap_or("upload").to_string();

            let content_type: String = field
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_string();

            // Validate content type
            if !is_valid_content_type(&content_type) {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!(
                        "Invalid content type '{}'. Allowed: {}",
                        content_type,
                        ALLOWED_CONTENT_TYPES.join(", ")
                    ),
                ));
            }

            // Read data with size limit
            let data = field.bytes().await.map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Failed to read file: {}", e),
                )
            })?;

            if data.len() > MAX_IMAGE_SIZE {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!(
                        "File too large: {} bytes (max: {} MB)",
                        data.len(),
                        MAX_IMAGE_SIZE / (1024 * 1024)
                    ),
                ));
            }

            file_data = Some((filename, content_type, data.to_vec()));
            break;
        }
    }

    let (filename, content_type, data) = file_data.ok_or((
        StatusCode::BAD_REQUEST,
        "No 'file' field in request".to_string(),
    ))?;

    // Generate thumbnail
    let (thumbnail_data, thumbnail_content_type) = generate_thumbnail(&data, &content_type)
        .map(|(d, t)| (Some(d), Some(t)))
        .unwrap_or((None, None));

    // Build metadata
    let mut metadata = serde_json::json!({});
    if let Some(session_id) = query.session_id {
        metadata["session_id"] = serde_json::json!(session_id.to_string());
    }

    // Store in database
    let size_bytes = data.len() as i64;
    let row = state
        .db
        .create_image(
            org.org_id,
            CreateImageRow {
                org_id: org.org_id,
                filename: filename.clone(),
                content_type: content_type.clone(),
                size_bytes,
                data,
                thumbnail_data,
                thumbnail_content_type,
                metadata,
            },
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to store image: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store image".to_string(),
            )
        })?;

    Ok((
        StatusCode::CREATED,
        Json(ImageUploadResponse {
            id: row.id,
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
        }),
    ))
}

/// GET /v1/images - List images
#[utoipa::path(
    get,
    path = "/v1/images",
    params(
        ("limit" = Option<i64>, Query, description = "Max items to return (default 50)"),
        ("offset" = Option<i64>, Query, description = "Items to skip")
    ),
    responses(
        (status = 200, description = "List of images", body = Vec<ImageInfo>),
        (status = 500, description = "Internal server error")
    ),
    tag = "images"
)]
pub async fn list_images(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListImagesQuery>,
) -> Result<Json<Vec<ImageInfo>>, StatusCode> {
    let rows = state
        .db
        .list_images(org.org_id, query.limit, query.offset)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list images: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let images: Vec<ImageInfo> = rows
        .into_iter()
        .map(|row| ImageInfo {
            id: row.id,
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            metadata: row.metadata,
            created_at: row.created_at,
        })
        .collect();

    Ok(Json(images))
}

/// GET /v1/images/{image_id} - Get image (returns binary data)
#[utoipa::path(
    get,
    path = "/v1/images/{image_id}",
    params(
        ("image_id" = String, Path, description = "Image ID (prefixed, e.g., img_...)")
    ),
    responses(
        (status = 200, description = "Image binary data", content_type = "image/*"),
        (status = 400, description = "Invalid image ID"),
        (status = 404, description = "Image not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "images"
)]
pub async fn get_image(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(image_id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let image_id: ImageId = image_id
        .parse()
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid image ID: {}", e)))?;

    let row = state
        .db
        .get_image(org.org_id, image_id.uuid())
        .await
        .map_err(|e| {
            tracing::error!("Failed to get image: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            )
        })?
        .ok_or((StatusCode::NOT_FOUND, "Image not found".to_string()))?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, row.content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!("inline; filename=\"{}\"", row.filename),
        )
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(row.data))
        .unwrap();

    Ok(response)
}

/// GET /v1/images/{image_id}/thumbnail - Get image thumbnail
#[utoipa::path(
    get,
    path = "/v1/images/{image_id}/thumbnail",
    params(
        ("image_id" = String, Path, description = "Image ID (prefixed, e.g., img_...)")
    ),
    responses(
        (status = 200, description = "Thumbnail binary data", content_type = "image/jpeg"),
        (status = 400, description = "Invalid image ID"),
        (status = 404, description = "Image not found or no thumbnail"),
        (status = 500, description = "Internal server error")
    ),
    tag = "images"
)]
pub async fn get_thumbnail(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(image_id): Path<String>,
) -> Result<Response, (StatusCode, String)> {
    let image_id: ImageId = image_id
        .parse()
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid image ID: {}", e)))?;

    let row = state
        .db
        .get_image(org.org_id, image_id.uuid())
        .await
        .map_err(|e| {
            tracing::error!("Failed to get image: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            )
        })?
        .ok_or((StatusCode::NOT_FOUND, "Image not found".to_string()))?;

    let (thumbnail_data, thumbnail_content_type) =
        match (row.thumbnail_data, row.thumbnail_content_type) {
            (Some(data), Some(ct)) => (data, ct),
            _ => return Err((StatusCode::NOT_FOUND, "No thumbnail available".to_string())),
        };

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, thumbnail_content_type)
        .header(header::CACHE_CONTROL, "public, max-age=31536000, immutable")
        .body(Body::from(thumbnail_data))
        .unwrap();

    Ok(response)
}

/// DELETE /v1/images/{image_id} - Delete an image
#[utoipa::path(
    delete,
    path = "/v1/images/{image_id}",
    params(
        ("image_id" = String, Path, description = "Image ID (prefixed, e.g., img_...)")
    ),
    responses(
        (status = 204, description = "Image deleted"),
        (status = 400, description = "Invalid image ID"),
        (status = 404, description = "Image not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "images"
)]
pub async fn delete_image(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(image_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let image_id: ImageId = image_id
        .parse()
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid image ID: {}", e)))?;

    let deleted = state
        .db
        .delete_image(org.org_id, image_id.uuid())
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete image: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error".to_string(),
            )
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((StatusCode::NOT_FOUND, "Image not found".to_string()))
    }
}

// ============================================
// Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_valid_content_type() {
        assert!(is_valid_content_type("image/png"));
        assert!(is_valid_content_type("image/jpeg"));
        assert!(is_valid_content_type("image/gif"));
        assert!(is_valid_content_type("image/webp"));
        assert!(!is_valid_content_type("image/svg+xml"));
        assert!(!is_valid_content_type("application/pdf"));
    }

    #[test]
    fn test_upload_query_session_id_parsing() {
        // Typed SessionId with prefix parses correctly
        let query: UploadImageQuery =
            serde_html_form::from_str("session_id=session_01933b5a00007000800000000000001a")
                .expect("typed session ID should parse");
        assert!(query.session_id.is_some());

        // Omitted session_id is fine (it's optional)
        let query: UploadImageQuery =
            serde_html_form::from_str("").expect("empty query should parse");
        assert!(query.session_id.is_none());

        // Raw UUID without prefix is rejected
        let result: Result<UploadImageQuery, _> =
            serde_html_form::from_str("session_id=01933b5a-0000-7000-8000-00000000001a");
        assert!(
            result.is_err(),
            "raw UUID without prefix should be rejected"
        );
    }

    #[test]
    fn test_format_from_content_type() {
        assert!(matches!(
            format_from_content_type("image/png"),
            Some(ImageFormat::Png)
        ));
        assert!(matches!(
            format_from_content_type("image/jpeg"),
            Some(ImageFormat::Jpeg)
        ));
        assert!(matches!(
            format_from_content_type("image/gif"),
            Some(ImageFormat::Gif)
        ));
        assert!(matches!(
            format_from_content_type("image/webp"),
            Some(ImageFormat::WebP)
        ));
        assert!(format_from_content_type("image/svg+xml").is_none());
    }

    #[test]
    fn test_image_info_serializes_prefixed_id() {
        let image_id = ImageId::from_seed(1);
        let info = ImageInfo {
            id: image_id,
            filename: "upload.png".to_string(),
            content_type: "image/png".to_string(),
            size_bytes: 42,
            metadata: json!({"session_id": "session_123"}),
            created_at: Utc::now(),
        };

        let value = serde_json::to_value(info).expect("image info should serialize");

        assert_eq!(value["id"], json!(image_id.to_string()));
    }

    #[test]
    fn test_image_upload_response_serializes_prefixed_id() {
        let image_id = ImageId::from_seed(2);
        let response = ImageUploadResponse {
            id: image_id,
            filename: "upload.png".to_string(),
            content_type: "image/png".to_string(),
            size_bytes: 42,
            created_at: Utc::now(),
        };

        let value = serde_json::to_value(response).expect("upload response should serialize");

        assert_eq!(value["id"], json!(image_id.to_string()));
    }
}
