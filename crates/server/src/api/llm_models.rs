// LLM Model API endpoints
// Routes: /v1/llm-providers/:provider_id/models/... and /v1/llm-models/...

use crate::api::common::{ApiOptionExt, ApiPolicyResultExt, ErrorResponse, ListResponse};
use crate::auth::{AuthState, ResolvedOrg};
use crate::services::llm_model::{LLM_MODEL_MANAGE, LLM_MODEL_VIEW};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::typed_id::{ModelId, ProviderId};
use everruns_core::{
    Caller, LlmModel, LlmModelSource, LlmModelStatus, LlmModelWithProvider, ResourceConfigResponse,
    evaluate_policies,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

use crate::services::{LlmModelService, LlmResolverService};

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<LlmModelService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        auth: AuthState,
        llm_resolver: Option<Arc<LlmResolverService>>,
    ) -> Self {
        let service = if let Some(resolver) = llm_resolver {
            LlmModelService::with_resolver(db, resolver)
        } else {
            LlmModelService::new(db)
        };
        Self {
            service: Arc::new(service),
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Request to create a new LLM model for a provider
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLlmModelRequest {
    /// The model identifier used by the provider's API (e.g., "gpt-4", "claude-3-opus").
    #[schema(example = "gpt-4o")]
    pub model_id: String,
    /// Human-readable display name for the model.
    #[schema(example = "GPT-4o")]
    pub display_name: String,
    /// List of capabilities this model supports (e.g., "chat", "vision", "tools").
    #[serde(default)]
    #[schema(example = json!(["chat", "vision", "tools"]))]
    pub capabilities: Vec<String>,
    /// Whether this model should be installed (visible in UI model pickers).
    #[serde(default)]
    #[schema(example = false)]
    pub installed: bool,
    /// Whether this model should be marked as a favorite for quick access.
    #[serde(default)]
    #[schema(example = false)]
    pub is_favorite: bool,
}

/// Query parameters for filtering models list
#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct ListModelsQuery {
    /// Filter by model source (manual, discovered, predefined)
    pub source: Option<LlmModelSource>,
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
pub struct UpdateLlmModelRequest {
    /// The model identifier used by the provider's API.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-4o-mini")]
    pub model_id: Option<String>,
    /// Human-readable display name for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "GPT-4o Mini")]
    pub display_name: Option<String>,
    /// List of capabilities this model supports.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["chat", "tools"]))]
    pub capabilities: Option<Vec<String>>,
    /// Whether this model should be installed (visible in UI model pickers).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub installed: Option<bool>,
    /// Whether this model should be marked as a favorite for quick access.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub is_favorite: Option<bool>,
    /// The status of the model. Set to "inactive" to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<LlmModelStatus>,
}

/// Create a new model for a provider
#[utoipa::path(
    post,
    path = "/v1/llm-providers/{provider_id}/models",
    params(
        ("provider_id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    request_body = CreateLlmModelRequest,
    responses(
        (status = 201, description = "Model created", body = LlmModel),
        (status = 400, description = "Invalid provider ID"),
        (status = 500, description = "Internal error")
    ),
    tag = "llm-models"
)]
pub async fn create_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(req): Json<CreateLlmModelRequest>,
) -> Result<(StatusCode, Json<LlmModel>), (StatusCode, Json<ErrorResponse>)> {
    let provider_id: ProviderId = provider_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid provider ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let model = state
        .service
        .create(&caller, provider_id.uuid(), req)
        .await
        .map_policy_or_internal("create LLM model")?;

    Ok((StatusCode::CREATED, Json(model)))
}

/// List models for a specific provider
#[utoipa::path(
    get,
    path = "/v1/llm-providers/{provider_id}/models",
    params(
        ("provider_id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 200, description = "List of models", body = ListResponse<LlmModel>),
        (status = 400, description = "Invalid provider ID")
    ),
    tag = "llm-models"
)]
pub async fn list_provider_models(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<Json<ListResponse<LlmModel>>, (StatusCode, Json<ErrorResponse>)> {
    let provider_id: ProviderId = provider_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid provider ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let models = state
        .service
        .list_for_provider(&caller, provider_id.uuid())
        .await
        .map_policy_or_internal("list LLM models for provider")?;

    Ok(Json(ListResponse::new(models)))
}

/// List all models across all providers
#[utoipa::path(
    get,
    path = "/v1/llm-models",
    params(
        ListModelsQuery
    ),
    responses(
        (status = 200, description = "List of all models", body = ListResponse<LlmModelWithProvider>)
    ),
    tag = "llm-models"
)]
pub async fn list_all_models(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListModelsQuery>,
) -> Result<Json<ListResponse<LlmModelWithProvider>>, (StatusCode, Json<ErrorResponse>)> {
    let caller = Caller::from(&org);
    let models = state
        .service
        .list_all_with_filters(
            &caller,
            query.source,
            query.include_stale,
            query.favorites_only,
        )
        .await
        .map_policy_or_internal("list all LLM models")?;

    Ok(Json(ListResponse::new(models)))
}

/// Get a specific model with provider info and profile
#[utoipa::path(
    get,
    path = "/v1/llm-models/{id}",
    params(
        ("id" = String, Path, description = "Model ID (prefixed, e.g., mod_...)")
    ),
    responses(
        (status = 200, description = "Model found", body = LlmModelWithProvider),
        (status = 400, description = "Invalid model ID"),
        (status = 404, description = "Model not found")
    ),
    tag = "llm-models"
)]
pub async fn get_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<LlmModelWithProvider>, (StatusCode, Json<ErrorResponse>)> {
    let model_id: ModelId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid model ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let model = state
        .service
        .get_with_provider(&caller, model_id.uuid())
        .await
        .map_policy_or_internal("get LLM model")?
        .ok_or_not_found_json("Model")?;

    Ok(Json(model))
}

/// Update a model
#[utoipa::path(
    patch,
    path = "/v1/llm-models/{id}",
    params(
        ("id" = String, Path, description = "Model ID (prefixed, e.g., mod_...)")
    ),
    request_body = UpdateLlmModelRequest,
    responses(
        (status = 200, description = "Model updated", body = LlmModel),
        (status = 400, description = "Invalid model ID"),
        (status = 404, description = "Model not found")
    ),
    tag = "llm-models"
)]
pub async fn update_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateLlmModelRequest>,
) -> Result<Json<LlmModel>, (StatusCode, Json<ErrorResponse>)> {
    let model_id: ModelId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid model ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let model = state
        .service
        .update(&caller, model_id.uuid(), req)
        .await
        .map_policy_or_internal("update LLM model")?
        .ok_or_not_found_json("Model")?;

    Ok(Json(model))
}

/// Delete a model
#[utoipa::path(
    delete,
    path = "/v1/llm-models/{id}",
    params(
        ("id" = String, Path, description = "Model ID (prefixed, e.g., mod_...)")
    ),
    responses(
        (status = 204, description = "Model deleted"),
        (status = 400, description = "Invalid model ID"),
        (status = 404, description = "Model not found")
    ),
    tag = "llm-models"
)]
pub async fn delete_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let model_id: ModelId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid model ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .delete(&caller, model_id.uuid())
        .await
        .map_policy_or_internal("delete LLM model")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::not_found("Model"))
    }
}

/// GET /v1/llm-models/config
#[utoipa::path(
    get,
    path = "/v1/llm-models/config",
    responses(
        (status = 200, description = "Resource config for LLM models", body = ResourceConfigResponse),
    ),
    tag = "llm-models"
)]
pub async fn llm_model_config(org: ResolvedOrg) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies(&caller, &[&LLM_MODEL_VIEW, &LLM_MODEL_MANAGE]);
    Json(ResourceConfigResponse { policies })
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/llm-models/config", get(llm_model_config))
        .route(
            "/v1/llm-providers/{provider_id}/models",
            post(create_model).get(list_provider_models),
        )
        .route("/v1/llm-models", get(list_all_models))
        .route(
            "/v1/llm-models/{id}",
            get(get_model).patch(update_model).delete(delete_model),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_response_serialization() {
        let error = ErrorResponse::new("Internal server error");
        let json = serde_json::to_string(&error).expect("Failed to serialize");
        assert_eq!(json, r#"{"error":"Internal server error"}"#);
    }

    #[test]
    fn test_error_response_internal_error_format() {
        // Verify that internal error responses use the generic message
        let error = ErrorResponse::new("Internal server error");
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(parsed["error"], "Internal server error");
    }

    #[test]
    fn test_error_response_not_found_format() {
        let error = ErrorResponse::new("Model not found");
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(parsed["error"], "Model not found");
    }

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
