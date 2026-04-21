use crate::domains::common::CommandError;
use crate::services::LlmProviderService;
use everruns_core::typed_id::ProviderId;
use std::sync::Arc;

pub fn service(ctx: &crate::domains::common::Ctx) -> Arc<LlmProviderService> {
    ctx.llm_provider_service.clone().unwrap_or_else(|| {
        Arc::new(LlmProviderService::new(
            ctx.db.clone(),
            ctx.encryption.clone(),
        ))
    })
}

pub fn parse_provider_id(input: &str) -> Result<uuid::Uuid, CommandError> {
    input
        .parse::<ProviderId>()
        .map(|id| id.uuid())
        .map_err(|e| CommandError::bad_request(format!("Invalid provider ID: {e}")))
}
