// Image upload HTTP routes
//
// Global image storage for message attachments.
// Supports PNG, JPEG, GIF, WebP (OpenAI Vision compatible formats).
// Generates thumbnails on upload for efficient display.

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
use image::ImageFormat;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================
// Constants
// ============================================

/// Maximum image size in bytes (100 MB)
const MAX_IMAGE_SIZE: usize = 100 * 1024 * 1024;

/// Thumbnail max dimension (width or height)
const THUMBNAIL_MAX_DIM: u32 = 200;

/// Allowed MIME types
const ALLOWED_CONTENT_TYPES: &[&str] = &["image/png", "image/jpeg", "image/gif", "image/webp"];

// ============================================
// API Types
// ============================================

/// Image metadata (without binary data)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageInfo {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Image upload response
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ImageUploadResponse {
    pub id: Uuid,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
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
    /// Optional session ID for metadata tracking
    pub session_id: Option<Uuid>,
}

// ============================================
// App State and Routes
// ============================================

/// App state for images routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }
}

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
        .route("/v1/images/:image_id", get(get_image).delete(delete_image))
        .route("/v1/images/:image_id/thumbnail", get(get_thumbnail))
        .with_state(state)
}

// ============================================
// Helper Functions
// ============================================

/// Validate content type is allowed
fn is_valid_content_type(content_type: &str) -> bool {
    ALLOWED_CONTENT_TYPES.contains(&content_type)
}

/// Detect image format from content type
fn format_from_content_type(content_type: &str) -> Option<ImageFormat> {
    match content_type {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/gif" => Some(ImageFormat::Gif),
        "image/webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

/// Generate thumbnail from image data
fn generate_thumbnail(data: &[u8], content_type: &str) -> Option<(Vec<u8>, String)> {
    let format = format_from_content_type(content_type)?;

    // Load image
    let img = image::load_from_memory_with_format(data, format).ok()?;

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
        ("session_id" = Option<Uuid>, Query, description = "Optional session ID for tracking")
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
        .create_image(CreateImageRow {
            filename: filename.clone(),
            content_type: content_type.clone(),
            size_bytes,
            data,
            thumbnail_data,
            thumbnail_content_type,
            metadata,
        })
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
    State(state): State<AppState>,
    Query(query): Query<ListImagesQuery>,
) -> Result<Json<Vec<ImageInfo>>, StatusCode> {
    let rows = state
        .db
        .list_images(query.limit, query.offset)
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
        ("image_id" = Uuid, Path, description = "Image ID")
    ),
    responses(
        (status = 200, description = "Image binary data", content_type = "image/*"),
        (status = 404, description = "Image not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "images"
)]
pub async fn get_image(
    State(state): State<AppState>,
    Path(image_id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    let row = state
        .db
        .get_image(image_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get image: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

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
        ("image_id" = Uuid, Path, description = "Image ID")
    ),
    responses(
        (status = 200, description = "Thumbnail binary data", content_type = "image/jpeg"),
        (status = 404, description = "Image not found or no thumbnail"),
        (status = 500, description = "Internal server error")
    ),
    tag = "images"
)]
pub async fn get_thumbnail(
    State(state): State<AppState>,
    Path(image_id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    let row = state
        .db
        .get_image(image_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get image: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let (thumbnail_data, thumbnail_content_type) =
        match (row.thumbnail_data, row.thumbnail_content_type) {
            (Some(data), Some(ct)) => (data, ct),
            _ => return Err(StatusCode::NOT_FOUND),
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
        ("image_id" = Uuid, Path, description = "Image ID")
    ),
    responses(
        (status = 204, description = "Image deleted"),
        (status = 404, description = "Image not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "images"
)]
pub async fn delete_image(
    State(state): State<AppState>,
    Path(image_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state.db.delete_image(image_id).await.map_err(|e| {
        tracing::error!("Failed to delete image: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

// ============================================
// Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

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
}
