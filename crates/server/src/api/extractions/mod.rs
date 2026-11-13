//! Org-scoped, explicitly selected BYOK extractions. No deployment credential fallback.
use crate::{
    api::{common::ApiResult, state::ApiState},
    auth::ResolvedOrg,
    domains::{
        common::CommandError,
        models::{LLM_MODEL_MANAGE, ModelService},
    },
    services::provider_resolver::{
        provider_api, provider_request_options, resolve_provider_api_key,
    },
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    routing::post,
};
use everruns_contracts::{
    ServiceKind,
    driver_registry::ProviderConfig,
    extractions::{ExtractionOutcome, ExtractionRequest},
    typed_id::ModelId,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A configured org catalog model and its extraction input. Provider/model selection
/// inside `input` is rejected; the catalog identity is authoritative.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractRequest {
    pub model_id: String,
    pub input: ExtractionRequest,
}

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/extractions", post(extract))
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .with_state(state)
}

#[utoipa::path(post, path = "/v1/extractions", operation_id = "extract_typed_fields",
 summary = "Extract typed fields", request_body = ExtractRequest,
 responses((status = 200, description = "Typed extraction values, evidence and usage", body = ExtractionOutcome),
 (status = 400, description = "Invalid or unsupported extraction request"),
 (status = 403, description = "Model management permission required"),
 (status = 404, description = "Model not found"),
 (status = 413, description = "Request body exceeds 8 MiB"),
 (status = 502, description = "Extraction provider failed"),
 (status = 504, description = "Extraction timed out")), tag = "extractions")]
pub async fn extract(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Json(mut request): Json<ExtractRequest>,
) -> ApiResult<ExtractionOutcome> {
    // THREAT[TM-EXTRACT-001]: tenant model selection never borrows utility credentials.
    let ctx = state.ctx(&org);
    LLM_MODEL_MANAGE
        .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
        .map_err(|_| CommandError::forbidden("Model management permission required"))?;
    if ctx.caller.read_only {
        return Err(CommandError::forbidden("Read-only credentials cannot run extractions").into());
    }
    validate_selection(&request)?;
    request
        .input
        .validate()
        .map_err(|_| CommandError::bad_request("Invalid extraction input"))?;
    let model_id = request
        .model_id
        .parse::<ModelId>()
        .map_err(|_| CommandError::bad_request("Invalid model ID"))?;
    let model = ModelService::new(ctx.db.clone())
        .get_with_provider(&ctx.caller, model_id.uuid())
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::not_found("Model not found"))?;
    if !model.enabled
        || !(model.service == ServiceKind::Extractions
            || model
                .capabilities
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case("extractions")))
    {
        return Err(CommandError::bad_request("Select an enabled extraction model").into());
    }
    let provider = ctx
        .db
        .get_provider(ctx.org_id(), model.provider_id.uuid())
        .await
        .map_err(CommandError::from)?
        .ok_or_else(|| CommandError::not_found("Provider not found"))?;
    if provider.status != "active" {
        return Err(CommandError::bad_request("Provider is not active").into());
    }
    // Host-managed accounts require host-specific quota/accounting admission. Do not
    // turn this standalone request path into an unmetered platform credential proxy.
    if provider.managed {
        return Err(CommandError::bad_request(
            "Extractions require an organization-owned provider account",
        )
        .into());
    }
    let key = resolve_provider_api_key(&ctx.db, ctx.encryption.as_deref(), &provider)
        .map_err(|_| CommandError::bad_request("Provider credentials could not be resolved"))?
        .ok_or_else(|| CommandError::bad_request("Provider credentials are required"))?;
    let mut config = ProviderConfig::for_provider(
        provider.id.to_string(),
        provider.provider_type.parse().expect("infallible DriverId"),
    );
    config.api_key = Some(key);
    config.base_url = provider.base_url;
    config.request_options = provider_request_options(&provider.settings);
    config.api = provider_api(&provider.settings)
        .map_err(|_| CommandError::bad_request("Invalid provider configuration"))?;
    let runtime = ctx.driver_registry.create_provider(&config).map_err(|_| {
        CommandError::bad_request("Provider could not be configured for extractions")
    })?;
    if !runtime.supports_service(ServiceKind::Extractions) {
        return Err(CommandError::bad_request("Provider does not support extractions").into());
    }
    request.input.model = Some(model.model_id);
    // Never echo provider response bodies or source documents into logs/API errors.
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        runtime.extract(request.input),
    )
    .await
    .map_err(|_| {
        (
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            Json(crate::api::common::ErrorResponse::new(
                "Extraction timed out",
            )),
        )
    })?
    .map_err(|_| {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            Json(crate::api::common::ErrorResponse::new(
                "Extraction provider failed",
            )),
        )
    })?;
    Ok(Json(outcome))
}

fn validate_selection(request: &ExtractRequest) -> Result<(), CommandError> {
    if request.input.provider.is_some() || request.input.model.is_some() {
        return Err(CommandError::bad_request(
            "Use model_id to select the configured extraction model",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
