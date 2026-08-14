use super::types::{ImageInfo, StoredImageResponse};
use crate::domains::common::CommandError;
use crate::storage::{ImageInfoRow, ImageRow};
use everruns_provider::typed_id::ImageId;

pub fn parse_image_id(input: &str) -> Result<ImageId, CommandError> {
    input
        .parse()
        .map_err(|e| CommandError::bad_request(format!("Invalid image ID: {e}")))
}

pub fn row_to_image_info(row: ImageInfoRow) -> ImageInfo {
    ImageInfo {
        id: row.id,
        filename: row.filename,
        content_type: row.content_type,
        size_bytes: row.size_bytes,
        metadata: row.metadata,
        created_at: row.created_at,
    }
}

pub fn row_to_stored_image(row: ImageRow) -> StoredImageResponse {
    StoredImageResponse {
        id: row.id,
        filename: row.filename,
        content_type: row.content_type,
        size_bytes: row.size_bytes,
        data: row.data,
        thumbnail_data: row.thumbnail_data,
        thumbnail_content_type: row.thumbnail_content_type,
        metadata: row.metadata,
        created_at: row.created_at,
    }
}
