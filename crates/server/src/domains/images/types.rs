// Images domain types — list DTOs; MCP get returns stored bytes.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::ImageId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub type ListImagesResponse = crate::common_dto::ListResponse<ImageInfo>;

#[derive(Debug, Clone, Serialize)]
pub struct StoredImageResponse {
    pub id: ImageId,
    pub filename: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub data: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_data: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail_content_type: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

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
