use super::queries as q;
use super::types::{ListImagesResponse, StoredImageResponse};
use crate::domains::common::*;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListImages {
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub offset: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u32_lenient")]
    pub limit: Option<u32>,
}

impl Command for ListImages {
    type Output = ListImagesResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_images",
            category: "images",
            description: "List uploaded images. Supports pagination (limit/offset).",
            method: "GET",
            path: "/v1/images",
        }
    }

    async fn execute(self, ctx: &Ctx) -> Result<ListImagesResponse, CommandError> {
        let offset = self.offset.unwrap_or(0);
        let limit = self.limit.unwrap_or(50).min(100);
        let rows = ctx
            .db
            .list_images(ctx.org_id(), i64::from(limit), i64::from(offset))
            .await
            .map_err(classify_anyhow)?;

        Ok(crate::api::common::ListResponse::new(
            rows.into_iter().map(q::row_to_image_info).collect(),
        ))
    }
}

inventory::submit! { CommandDescriptor::of::<ListImages>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetImage {
    pub id: String,
}

impl Command for GetImage {
    type Output = StoredImageResponse;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_image",
            category: "images",
            description: "Get image data by ID.",
            method: "GET",
            path: "/v1/images/{id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("id")
    }

    async fn execute(self, ctx: &Ctx) -> Result<StoredImageResponse, CommandError> {
        let image_id = q::parse_image_id(&self.id)?;
        let row = ctx
            .db
            .get_image(ctx.org_id(), image_id.uuid())
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Image"))?;
        Ok(q::row_to_stored_image(row))
    }
}

inventory::submit! { CommandDescriptor::of::<GetImage>() }
