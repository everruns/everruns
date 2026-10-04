// LLM Model API endpoints
// Routes: /v1/providers/:provider_id/models/... and /v1/models/...

use crate::api::common::{
    ApiResult, ErrorResponse, ListResponse, UrlBuilder, WithUrls, impl_auth_state,
};
use crate::api::dispatch::{Dispatchable, impl_dispatchable};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::models::{
    CreateModel, DeleteModel, GetDefaultModel, GetModel, LLM_MODEL_MANAGE, LLM_MODEL_VIEW,
    ListModels, ListProviderModels, ModelService, UpdateModel,
};
use crate::kernel_imports::{
    Caller, ResourceConfigResponse, contracts::model::Model, contracts::model::ModelSource,
    contracts::model::ModelWithProvider, evaluate_policies_with,
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
    #[serde(default)]
    pub service: Option<everruns_contracts::ServiceKind>,
    #[serde(default)]
    pub profile_key: Option<String>,
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
    /// Filter by typed model service.
    pub service: Option<everruns_contracts::ServiceKind>,
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
    pub service: Option<everruns_contracts::ServiceKind>,
    pub profile_key: Option<String>,
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
            service: req.service,
            profile_key: req.profile_key,
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
    Query(query): Query<ListModelsQuery>,
) -> ApiResult<ListResponse<WithUrls<Model>>> {
    let models = ListProviderModels { provider_id }
        .run(&state.ctx(&org))
        .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    let models: Vec<_> = models
        .into_iter()
        .filter(|model| query.service.is_none_or(|service| model.service == service))
        .collect();
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
        service: query.service,
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
            service: req.service,
            profile_key: req.profile_key,
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

/// Resolve the default for empty drafts without persisting a session.
#[utoipa::path(get, path = "/v1/models/default", responses(
    (status = 200, description = "Effective organization default, or null when unavailable", body = Option<ModelWithProvider>),
    (status = 403, description = "Forbidden", body = ErrorResponse)
), tag = "models")]
pub async fn get_default_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<Option<ModelWithProvider>> {
    Ok(Json(GetDefaultModel {}.run(&state.ctx(&org)).await?))
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/model-profiles", get(list_model_profiles))
        .route("/v1/model-profiles/{*key}", get(get_model_profile))
        .route("/v1/models/config", get(model_config))
        .route("/v1/models/default", get(get_default_model))
        .route(
            "/v1/models/decision-default",
            get(get_default_decision_model).put(set_default_decision_model),
        )
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

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct ModelProfileResponse {
    pub key: String,
    pub service: everruns_contracts::ServiceKind,
    pub vendor: Option<everruns_contracts::model::ModelVendor>,
    pub source: String,
    pub profile: everruns_contracts::model::ModelProfile,
}

#[derive(Default, Deserialize, IntoParams)]
pub struct ProfileQuery {
    pub service: Option<everruns_contracts::ServiceKind>,
    pub provider_id: Option<String>,
}

async fn profiles_for_caller(
    org: &ResolvedOrg,
    state: &AppState,
    query: ProfileQuery,
) -> ApiResult<Vec<ModelProfileResponse>> {
    // THREAT[TM-TENANT-001]: discovered profiles come only from the caller's authorized model catalog.
    let models = ListModels {
        service: None,
        source: None,
        include_stale: true,
        favorites_only: false,
    }
    .run(&state.ctx(org))
    .await?;
    let selected_provider = query.provider_id.clone();
    let entries = if let Some(id) = query.provider_id {
        let provider = state
            .db
            .get_provider(
                org.org_id,
                id.parse::<everruns_contracts::typed_id::ProviderId>()
                    .map_err(|_| {
                        ErrorResponse::new("Invalid provider ID")
                            .into_response(StatusCode::BAD_REQUEST)
                    })?
                    .uuid(),
            )
            .await
            .map_err(|_| {
                ErrorResponse::new("Failed to read provider")
                    .into_response(StatusCode::INTERNAL_SERVER_ERROR)
            })?;
        let provider = provider.ok_or_else(|| {
            ErrorResponse::new("Provider not found").into_response(StatusCode::NOT_FOUND)
        })?;
        if !crate::services::chatgpt::visible(&provider.settings, &Caller::from(org)) {
            return Err(
                ErrorResponse::new("Provider not found").into_response(StatusCode::NOT_FOUND)
            );
        }
        everruns_contracts::model_profile_data::profile_entries_for_provider(
            &provider.provider_type,
        )
        .into_iter()
        .filter(|entry| {
            state
                .service
                .validate_model_service(&provider.provider_type, entry.service)
                .is_ok()
        })
        .collect()
    } else {
        everruns_contracts::model_profile_data::all_profile_entries()
    };
    let mut profiles: Vec<_> = entries
        .into_iter()
        .map(|entry| ModelProfileResponse {
            key: entry.key,
            service: entry.service,
            vendor: Some(entry.vendor),
            source: "curated".into(),
            profile: entry.profile,
        })
        .collect();
    for model in models {
        if selected_provider
            .as_ref()
            .is_some_and(|id| id != &model.provider_id.to_string())
        {
            continue;
        }
        if !model.profile_key.is_empty()
            && !profiles
                .iter()
                .any(|profile| profile.key == model.profile_key)
            && let Some(profile) = model.profile
        {
            profiles.push(ModelProfileResponse {
                key: model.profile_key,
                service: model.service,
                vendor: model.model_vendor,
                source: model.source.to_string(),
                profile,
            });
        }
    }
    profiles.retain(|profile| {
        query
            .service
            .is_none_or(|service| profile.service == service)
    });
    profiles.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(Json(profiles))
}

#[utoipa::path(get, path = "/v1/model-profiles", params(ProfileQuery), responses((status = 200, description = "Authorized model profiles", body = ListResponse<ModelProfileResponse>)), tag = "models")]
pub async fn list_model_profiles(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ProfileQuery>,
) -> ApiResult<ListResponse<ModelProfileResponse>> {
    let Json(profiles) = profiles_for_caller(&org, &state, query).await?;
    Ok(Json(ListResponse::new(profiles)))
}

#[utoipa::path(get, path = "/v1/model-profiles/{key}", params(("key" = String, Path)), responses((status = 200, description = "Model profile", body = ModelProfileResponse), (status = 404, description = "Unknown profile")), tag = "models")]
pub async fn get_model_profile(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> ApiResult<ModelProfileResponse> {
    let Json(profiles) = profiles_for_caller(&org, &state, ProfileQuery::default()).await?;
    match profiles.into_iter().find(|profile| profile.key == key) {
        Some(profile) => Ok(Json(profile)),
        None => {
            Err(ErrorResponse::new("Model profile not found").into_response(StatusCode::NOT_FOUND))
        }
    }
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
}

#[utoipa::path(get, path = "/v1/models/decision-default", responses((status = 200, description = "Selected decision model", body = Option<ModelWithProvider>)), tag = "models")]
pub async fn get_default_decision_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<Option<ModelWithProvider>> {
    Ok(Json(
        crate::domains::models::commands::GetDefaultDecisionModel {}
            .run(&state.ctx(&org))
            .await?,
    ))
}
#[utoipa::path(put, path = "/v1/models/decision-default", request_body = crate::domains::models::commands::SetDefaultDecisionModel, responses((status = 200, description = "Selected decision model", body = Option<ModelWithProvider>)), tag = "models")]
pub async fn set_default_decision_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<crate::domains::models::commands::SetDefaultDecisionModel>,
) -> ApiResult<Option<ModelWithProvider>> {
    Ok(Json(req.run(&state.ctx(&org)).await?))
}
