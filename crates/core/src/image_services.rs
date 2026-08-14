//! Neutral contracts for image artifacts and model image resolution.

use crate::error::Result;
use crate::typed_id::ImageId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Metadata for a stored image artifact.
#[derive(Debug, Clone)]
pub struct StoredImageInfo {
    pub id: ImageId,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Stored image artifact with binary data.
#[derive(Debug, Clone)]
pub struct StoredImage {
    pub info: StoredImageInfo,
    pub data: Vec<u8>,
}

/// Input for creating a stored image artifact.
#[derive(Debug, Clone)]
pub struct CreateStoredImage {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
    pub metadata: serde_json::Value,
}

#[async_trait]
pub trait ImageArtifactStore: Send + Sync {
    /// Persist an image artifact and return its durable metadata.
    async fn create_image(&self, input: CreateStoredImage) -> Result<StoredImageInfo>;

    /// Load a stored image artifact including bytes.
    async fn get_image(&self, image_id: ImageId) -> Result<Option<StoredImage>>;

    /// Load stored image metadata without binary data.
    async fn get_image_info(&self, image_id: ImageId) -> Result<Option<StoredImageInfo>>;
}

/// Resolved image data for LLM consumption
///
/// This struct contains the actual image data in a format suitable for
/// sending to LLM providers. Both OpenAI and Anthropic accept base64-encoded
/// images with media type information.
#[derive(Debug, Clone)]
pub struct ResolvedImage {
    /// Base64-encoded image data (without data URL prefix)
    pub base64: String,
    /// MIME type (e.g., "image/png", "image/jpeg")
    pub media_type: String,
}

impl ResolvedImage {
    /// Create a new resolved image
    pub fn new(base64: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self {
            base64: base64.into(),
            media_type: media_type.into(),
        }
    }

    /// Convert to a data URL suitable for OpenAI Vision API
    ///
    /// Format: `data:{media_type};base64,{base64_data}`
    pub fn to_data_url(&self) -> String {
        format!("data:{};base64,{}", self.media_type, self.base64)
    }
}

/// Trait for resolving image_file content parts to actual image data
///
/// When building LLM messages, `image_file` content parts contain only
/// a reference (UUID) to an uploaded image. This trait allows resolving
/// those references to actual image data.
///
/// # Provider-specific formatting
///
/// The resolved image data is then converted to provider-specific formats:
///
/// **OpenAI Vision:**
/// ```json
/// {
///   "type": "image_url",
///   "image_url": { "url": "data:image/png;base64,..." }
/// }
/// ```
///
/// **Anthropic Vision:**
/// ```json
/// {
///   "type": "image",
///   "source": { "type": "base64", "media_type": "image/png", "data": "..." }
/// }
/// ```
///
/// # Implementation notes
///
/// Implementations should:
/// - Fetch image data from storage (database, S3, etc.)
/// - Return base64-encoded data with media type
/// - Handle missing images gracefully (return None)
#[async_trait]
pub trait ImageResolver: Send + Sync {
    /// Resolve an image_file reference to actual image data
    ///
    /// Returns `None` if the image is not found.
    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>>;
}

// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolved_image_new() {
        let image = ResolvedImage::new("SGVsbG8=", "image/png");
        assert_eq!(image.base64, "SGVsbG8=");
        assert_eq!(image.media_type, "image/png");
    }

    #[test]
    fn test_resolved_image_to_data_url() {
        let image = ResolvedImage::new("SGVsbG8=", "image/png");
        let data_url = image.to_data_url();
        assert_eq!(data_url, "data:image/png;base64,SGVsbG8=");
    }

    #[test]
    fn test_resolved_image_jpeg() {
        let image = ResolvedImage::new("base64data", "image/jpeg");
        let data_url = image.to_data_url();
        assert!(data_url.starts_with("data:image/jpeg;base64,"));
    }
}
