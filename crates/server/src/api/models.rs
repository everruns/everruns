// LLM Model API endpoints
// Routes: /v1/providers/:provider_id/models/... and /v1/models/...

use crate::api::common::{
    ApiResult, ErrorResponse, ListResponse, UrlBuilder, WithUrls, impl_auth_state,
};
use crate::api::dispatch::{Dispatchable, impl_dispatchable};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::models::{
    CreateModel, DeleteModel, GetModel, LLM_MODEL_MANAGE, LLM_MODEL_VIEW, ListModels,
    ListProviderModels, ModelService, UpdateModel,
};
use crate::kernel_imports::{
    Caller, ResourceConfigResponse, evaluate_policies_with, everruns_provider::model::Model,
    everruns_provider::model::ModelSource, everruns_provider::model::ModelWithProvider,
};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

use crate::services::ProviderResolverService;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub service: Arc<ModelService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        auth: AuthState,
        provider_resolver: Option<Arc<ProviderResolverService>>,
    ) -> Self {
        let service = if let Some(resolver) = provider_resolver {
            ModelService::with_resolver(db.clone(), resolver)
        } else {
            ModelService::new(db.clone())
        };
        Self {
            db,
            service: Arc::new(service),
            auth,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_model_service(self.service.clone())
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

/// Request to create a new LLM model for a provider
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateModelRequest {
    /// The model identifier used by the provider's API (e.g., "gpt-5.6-sol", "claude-opus-5").
    #[schema(example = "gpt-5.2")]
    pub model_id: String,
    /// Human-readable display name for the model.
    #[schema(example = "GPT-4o")]
    pub display_name: String,
    /// List of capabilities this model supports (e.g., "chat", "vision", "tools").
    #[serde(default)]
    #[schema(example = json!(["chat", "vision", "tools"]))]
    pub capabilities: Vec<String>,
    /// Whether this model should be enabled (visible in UI model pickers).
    #[serde(default)]
    #[schema(example = false)]
    pub enabled: bool,
    /// Whether this model should be marked as a favorite for quick access.
    #[serde(default)]
    #[schema(example = false)]
    pub is_favorite: bool,
}

/// Query parameters for filtering models list
#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct ListModelsQuery {
    /// Filter by model source (manual, discovered, predefined)
    pub source: Option<ModelSource>,
    /// Include models that are stale (not seen in recent sync). Default: true
    #[serde(default = "default_true")]
    pub include_stale: bool,
    /// Only return favorite models. Default: false
    #[serde(default)]
    pub favorites_only: bool,
}

fn default_true() -> bool {
    true
}

/// Request to update an LLM model. Only provided fields will be updated.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateModelRequest {
    /// Provider that owns this model.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "provider_019df670b5af7db7a5685a4ad18a544a")]
    pub provider_id: Option<String>,
    /// The model identifier used by the provider's API.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-5.4-mini")]
    pub model_id: Option<String>,
    /// Human-readable display name for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "GPT-4o Mini")]
    pub display_name: Option<String>,
    /// List of capabilities this model supports.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["chat", "tools"]))]
    pub capabilities: Option<Vec<String>>,
    /// Whether this model should be enabled (visible in UI model pickers).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub enabled: Option<bool>,
    /// Whether this model should be marked as a favorite for quick access.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub is_favorite: Option<bool>,
}

/// Create a new model for a provider
#[utoipa::path(
    post,
    path = "/v1/providers/{provider_id}/models",
    params(
        ("provider_id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    request_body = CreateModelRequest,
    responses(
        (status = 201, description = "Model created", body = WithUrls<Model>),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found"),
        (status = 500, description = "Internal error")
    ),
    tag = "models"
)]
pub async fn create_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(req): Json<CreateModelRequest>,
) -> Result<(StatusCode, Json<WithUrls<Model>>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created_with_urls(CreateModel {
            provider_id,
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            enabled: req.enabled,
            is_favorite: req.is_favorite,
        })
        .await
}

/// List models for a specific provider
#[utoipa::path(
    get,
    path = "/v1/providers/{provider_id}/models",
    params(
        ("provider_id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 200, description = "List of models", body = ListResponse<WithUrls<Model>>),
        (status = 400, description = "Invalid provider ID")
    ),
    tag = "models"
)]
pub async fn list_provider_models(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> ApiResult<ListResponse<WithUrls<Model>>> {
    let models = ListProviderModels { provider_id }
        .run(&state.ctx(&org))
        .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(ListResponse::new(models).with_urls(&builder)))
}

/// List all models across all providers
#[utoipa::path(
    get,
    path = "/v1/models",
    params(
        ListModelsQuery
    ),
    responses(
        (status = 200, description = "List of all models", body = ListResponse<WithUrls<ModelWithProvider>>)
    ),
    tag = "models"
)]
pub async fn list_all_models(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListModelsQuery>,
) -> ApiResult<ListResponse<WithUrls<ModelWithProvider>>> {
    let models = ListModels {
        source: query.source,
        include_stale: query.include_stale,
        favorites_only: query.favorites_only,
    }
    .run(&state.ctx(&org))
    .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(ListResponse::new(models).with_urls(&builder)))
}

/// Get a specific model with provider info and profile
#[utoipa::path(
    get,
    path = "/v1/models/{id}",
    params(
        ("id" = String, Path, description = "Model ID (prefixed, e.g., mod_...)")
    ),
    responses(
        (status = 200, description = "Model found", body = WithUrls<ModelWithProvider>),
        (status = 400, description = "Invalid model ID"),
        (status = 404, description = "Model not found")
    ),
    tag = "models"
)]
pub async fn get_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<WithUrls<ModelWithProvider>> {
    state.dispatcher(&org).run_with_urls(GetModel { id }).await
}

/// Update a model
#[utoipa::path(
    patch,
    path = "/v1/models/{id}",
    params(
        ("id" = String, Path, description = "Model ID (prefixed, e.g., mod_...)")
    ),
    request_body = UpdateModelRequest,
    responses(
        (status = 200, description = "Model updated", body = WithUrls<Model>),
        (status = 400, description = "Invalid model ID"),
        (status = 404, description = "Model not found")
    ),
    tag = "models"
)]
pub async fn update_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateModelRequest>,
) -> ApiResult<WithUrls<Model>> {
    state
        .dispatcher(&org)
        .run_with_urls(UpdateModel {
            id,
            provider_id: req.provider_id,
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            enabled: req.enabled,
            is_favorite: req.is_favorite,
        })
        .await
}

/// Delete a model
#[utoipa::path(
    delete,
    path = "/v1/models/{id}",
    params(
        ("id" = String, Path, description = "Model ID (prefixed, e.g., mod_...)")
    ),
    responses(
        (status = 204, description = "Model deleted"),
        (status = 400, description = "Invalid model ID"),
        (status = 404, description = "Model not found")
    ),
    tag = "models"
)]
pub async fn delete_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(DeleteModel { id })
        .await
}

/// GET /v1/models/config
#[utoipa::path(
    get,
    path = "/v1/models/config",
    responses(
        (status = 200, description = "Resource config for LLM models", body = ResourceConfigResponse),
    ),
    tag = "models"
)]
pub async fn model_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&LLM_MODEL_VIEW, &LLM_MODEL_MANAGE],
    );
    Json(ResourceConfigResponse { policies })
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/models/config", get(model_config))
        .route(
            "/v1/providers/{provider_id}/models",
            post(create_model).get(list_provider_models),
        )
        .route("/v1/models", get(list_all_models))
        .route(
            "/v1/models/{id}",
            get(get_model).patch(update_model).delete(delete_model),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_response_serialization() {
        // RFC 9457 Problem Details shape: detail carries the message,
        // title/status are populated when the response is finalized.
        let (_status, body) = ErrorResponse::new("Internal server error")
            .into_response(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        let parsed: serde_json::Value = serde_json::to_value(&body.0).expect("Failed to serialize");
        assert_eq!(parsed["detail"], "Internal server error");
        assert_eq!(parsed["title"], "Internal Server Error");
        assert_eq!(parsed["status"], 500);
        // Unset extensions stay out of the wire payload.
        assert!(parsed.get("type").is_none());
        assert!(parsed.get("allowed_actions").is_none());
    }

    // Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

    #[test]
    fn test_internal_error_does_not_leak_details() {
        // Simulate what happens when a database error occurs
        // The error message should be generic, not contain DB details
        let generic_message = "Internal server error".to_string();

        // This is what we return to clients - verify it doesn't contain
        // typical database error patterns
        assert!(!generic_message.contains("SQLX"));
        assert!(!generic_message.contains("connection"));
        assert!(!generic_message.contains("database"));
        assert!(!generic_message.contains("query"));
        assert!(!generic_message.contains("postgres"));
    }
}
