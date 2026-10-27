//! Safe destination history: receiver credentials are never projected or restored.
use super::{DestinationConfig, commands::TRACING_MANAGE};
use crate::domains::common::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GetTracingDestinationMetadata {
    pub id: Uuid,
}
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TracingDestinationMetadata {
    pub id: Uuid,
    #[serde(flatten)]
    pub config: DestinationConfig,
}
#[command(name="get_tracing_destination_metadata",http=plain,params(("id"=Uuid,Path,description="Tracing resource ID")),category="tracing",description="Read safe destination metadata. Never returns receiver authentication.",method="GET",path="/v1/tracing/destination-metadata/{id}",policy=TRACING_MANAGE,cli=CliRoute::new(&["tracing","destinations"],"get").with_examples(&[CliExample::new("Inspect a destination without credentials","everruns tracing destinations get --id 00000000-0000-0000-0000-000000000001")]))]
impl Command for GetTracingDestinationMetadata {
    type Output = TracingDestinationMetadata;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let row = ctx
            .db
            .tracing_destination(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Destination"))?;
        let config: DestinationConfig =
            serde_json::from_value(row.config).map_err(|e| CommandError::internal(e.into()))?;
        Ok(TracingDestinationMetadata {
            id: self.id,
            config,
        })
    }
}
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct UpdateTracingDestinationMetadata {
    pub id: Uuid,
    pub name: Option<String>,
    pub enabled: Option<bool>,
}
#[command(name="update_tracing_destination_metadata",http=plain,summary="Update tracing destination metadata",params(("id"=Uuid,Path,description="Tracing resource ID")),category="tracing",description="Update safe destination metadata while preserving its receiver and current authentication.",method="PATCH",path="/v1/tracing/destination-metadata/{id}",policy=TRACING_MANAGE,cli=CliRoute::new(&["tracing","destinations"],"update").with_examples(&[CliExample::new("Pause external exports","everruns tracing destinations update --id 00000000-0000-0000-0000-000000000001 --enabled false --reason 'Pause exports'")]))]
impl Command for UpdateTracingDestinationMetadata {
    type Output = TracingDestinationMetadata;
    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let row = ctx
            .db
            .tracing_destination(ctx.org_id(), self.id)
            .await?
            .ok_or_else(|| CommandError::not_found("Destination"))?;
        let mut config: DestinationConfig =
            serde_json::from_value(row.config).map_err(|e| CommandError::internal(e.into()))?;
        if let Some(name) = self.name {
            config.name = name;
        }
        if let Some(value) = self.enabled {
            config.enabled = value;
        }
        config.validate().map_err(CommandError::bad_request)?;
        if !ctx
            .db
            .update_tracing_destination(ctx.org_id(), self.id, row.revision, &config, None)
            .await?
        {
            return Err(CommandError::conflict("Destination changed"));
        }
        let _ = super::health::refresh(&ctx.db, ctx.org_id()).await;
        Ok(TracingDestinationMetadata {
            id: self.id,
            config,
        })
    }
}
