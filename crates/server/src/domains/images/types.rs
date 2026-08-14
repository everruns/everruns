// Images domain types — list DTOs reuse API structs; MCP get returns stored bytes.

use chrono::{DateTime, Utc};
use everruns_provider::typed_id::ImageId;
use serde::Serialize;

pub use crate::api::images::ImageInfo;
pub type ListImagesResponse = crate::api::common::ListResponse<ImageInfo>;

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
