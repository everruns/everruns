// LLM Model API endpoints
// Routes: /v1/providers/:provider_id/models/... and /v1/models/...

use crate::api::common::{
    ApiResult, ErrorResponse, ListResponse, UrlBuilder, WithUrls, impl_auth_state,
};
use crate::api::dispatch::{Dispatchable, impl_dispatchable};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
pub use crate::domains::models::types::{CreateModelRequest, ListModelsQuery, UpdateModelRequest};
use crate::domains::models::{
    CreateModel, DeleteModel, GetDefaultModel, GetModel, LLM_MODEL_MANAGE, LLM_MODEL_VIEW,
    ListModels, ListProviderModels, ModelService, UpdateModel,
};
use crate::kernel_imports::{
    Caller, ResourceConfigResponse, contracts::model::Model, contracts::model::ModelWithProvider,
    evaluate_policies_with,
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

/// Read-only model behavior, independent from the credentials of a serving account.
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct ModelProfileResponse {
    /// Stable profile identity, including a vendor or custom-account namespace.
    pub key: String,
    /// Model service described by the profile.
    pub service: everruns_contracts::ServiceKind,
    /// Model developer, when known; this may differ from the serving provider.
    pub vendor: Option<everruns_contracts::model::ModelVendor>,
    /// Profile origin: curated, discovered, predefined or manual.
    pub source: String,
    /// Capabilities, decision semantics, limits and pricing for this model.
    pub profile: everruns_contracts::model::ModelProfile,
}

/// Restrict profiles to one service or an authorized provider catalog.
#[derive(Default, Deserialize, IntoParams)]
pub struct ProfileQuery {
    /// Return only profiles for this service.
    pub service: Option<everruns_contracts::ServiceKind>,
    /// Prefixed provider account ID used to restrict available profiles.
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

#[utoipa::path(get, path = "/v1/model-profiles", description = "List curated and account-scoped profiles visible to the caller, optionally filtered by service or provider.", params(ProfileQuery), responses((status = 200, description = "Authorized model profiles", body = ListResponse<ModelProfileResponse>)), tag = "models")]
pub async fn list_model_profiles(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ProfileQuery>,
) -> ApiResult<ListResponse<ModelProfileResponse>> {
    let Json(profiles) = profiles_for_caller(&org, &state, query).await?;
    Ok(Json(ListResponse::new(profiles)))
}

#[utoipa::path(get, path = "/v1/model-profiles/{key}", description = "Read a stable model profile visible to the caller. Profiles describe behavior independently from provider authentication.", params(("key" = String, Path)), responses((status = 200, description = "Model profile", body = ModelProfileResponse), (status = 404, description = "Unknown profile")), tag = "models")]
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

#[utoipa::path(get, path = "/v1/models/decision-default", description = "Get the organization decision default. An unavailable selection remains visible for repair; no selection returns null.", responses((status = 200, description = "Selected decision model", body = Option<ModelWithProvider>)), tag = "models")]
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
#[utoipa::path(put, path = "/v1/models/decision-default", description = "Select an enabled, healthy model supporting calibrated noul, choice and score decisions. Omit model_id or set it to null to clear the default.", request_body = crate::domains::models::commands::SetDefaultDecisionModel, responses((status = 200, description = "Selected decision model", body = Option<ModelWithProvider>)), tag = "models")]
pub async fn set_default_decision_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<crate::domains::models::commands::SetDefaultDecisionModel>,
) -> ApiResult<Option<ModelWithProvider>> {
    Ok(Json(req.run(&state.ctx(&org)).await?))
}
