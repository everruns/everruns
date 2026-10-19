use super::*;
use crate::kernel_imports::image_services::{CreateStoredImage, StoredImage, StoredImageInfo};
use everruns_contracts::typed_id::ImageId;

pub(super) struct DirectImageArtifactStore {
    pub(super) db: Arc<StorageBackend>,
    pub(super) org_id: i64,
}

#[async_trait]
impl ImageArtifactStore for DirectImageArtifactStore {
    async fn create_image(&self, input: CreateStoredImage) -> Result<StoredImageInfo> {
        let (thumbnail_data, thumbnail_content_type) =
            crate::api::images::generate_thumbnail(&input.data, &input.content_type)
                .map(|(data, content_type)| (Some(data), Some(content_type)))
                .unwrap_or((None, None));

        let row = self
            .db
            .create_image(
                self.org_id,
                crate::storage::CreateImageRow {
                    org_id: self.org_id,
                    filename: input.filename,
                    content_type: input.content_type,
                    size_bytes: input.data.len() as i64,
                    data: input.data,
                    thumbnail_data,
                    thumbnail_content_type,
                    metadata: input.metadata,
                },
            )
            .await
            .map_err(|e| store_error(format!("Failed to create image artifact: {e}")))?;

        Ok(StoredImageInfo {
            id: row.id,
            filename: row.filename,
            content_type: row.content_type,
            size_bytes: row.size_bytes,
            metadata: row.metadata,
            created_at: row.created_at,
        })
    }

    async fn get_image(&self, image_id: ImageId) -> Result<Option<StoredImage>> {
        let row = self
            .db
            .get_image(self.org_id, image_id.uuid())
            .await
            .map_err(|e| store_error(format!("Failed to get image artifact: {e}")))?;

        Ok(row.map(|row| StoredImage {
            info: StoredImageInfo {
                id: row.id,
                filename: row.filename,
                content_type: row.content_type,
                size_bytes: row.size_bytes,
                metadata: row.metadata,
                created_at: row.created_at,
            },
            data: row.data,
        }))
    }
}
