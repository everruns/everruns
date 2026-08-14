use crate::domains::common::CommandError;
use crate::domains::models::ModelService;
use everruns_provider::typed_id::{ModelId, ProviderId};
use std::sync::Arc;

pub fn service(ctx: &crate::domains::common::Ctx) -> Arc<ModelService> {
    ctx.model_service
        .clone()
        .unwrap_or_else(|| Arc::new(ModelService::new(ctx.db.clone())))
}

pub fn parse_model_id(input: &str) -> Result<uuid::Uuid, CommandError> {
    input
        .parse::<ModelId>()
        .map(|id| id.uuid())
        .map_err(|e| CommandError::bad_request(format!("Invalid model ID: {e}")))
}

pub fn parse_provider_id(input: &str) -> Result<uuid::Uuid, CommandError> {
    input
        .parse::<ProviderId>()
        .map(|id| id.uuid())
        .map_err(|e| CommandError::bad_request(format!("Invalid provider ID: {e}")))
}
