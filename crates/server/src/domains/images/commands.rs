use super::queries as q;
use super::types::{ListImagesResponse, StoredImageResponse};
use crate::domains::common::*;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListImages {
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    /// Zero-based offset into the result set.
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    /// Maximum number of items returned in this page.
    pub limit: Option<u32>,
}

#[command(
    name = "list_images",
    category = "images",
    description = "List uploaded images. Supports pagination (limit/offset).",
    method = "GET",
    path = "/v1/images"
)]
impl Command for ListImages {
    type Output = ListImagesResponse;

    async fn execute(self, ctx: &Ctx) -> Result<ListImagesResponse, CommandError> {
        let offset = self.offset.unwrap_or(0);
        let limit = self.limit.unwrap_or(50).min(100);
        let rows = ctx
            .db
            .list_images(ctx.org_id(), i64::from(limit), i64::from(offset))
            .await?;

        Ok(crate::api::common::ListResponse::new(
            rows.into_iter().map(q::row_to_image_info).collect(),
        ))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetImage {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: String,
}

#[command(
    name = "get_image",
    category = "images",
    description = "Get image data by ID.",
    method = "GET",
    path = "/v1/images/{id}",
    positional = "id"
)]
impl Command for GetImage {
    type Output = StoredImageResponse;

    async fn execute(self, ctx: &Ctx) -> Result<StoredImageResponse, CommandError> {
        let image_id = q::parse_image_id(&self.id)?;
        let row = ctx
            .db
            .get_image(ctx.org_id(), image_id.uuid())
            .await?
            .ok_or_else(|| CommandError::not_found("Image"))?;
        Ok(q::row_to_stored_image(row))
    }
}
