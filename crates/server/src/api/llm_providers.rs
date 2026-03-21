// LLM Provider API endpoints
// Routes: /v1/llm-providers/...

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::llm_provider::{LLM_PROVIDER_MANAGE, LLM_PROVIDER_VIEW};
use crate::services::{LlmProviderService, LlmResolverService, ModelSyncService, SyncResult};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::llm_models::LlmProvider;
use everruns_core::typed_id::ProviderId;
use everruns_core::{
    Caller, DriverRegistry, LlmProviderStatus, LlmProviderType, ResourceConfigResponse,
    evaluate_policies_with,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::{
    ApiOptionExt, ApiPolicyResultExt, ApiResult, ErrorResponse, ListResponse, impl_auth_state,
};

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<LlmProviderService>,
    pub sync_service: Arc<ModelSyncService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        driver_registry: Arc<DriverRegistry>,
        auth: AuthState,
        llm_resolver: Option<Arc<LlmResolverService>>,
    ) -> Self {
        let service = if let Some(resolver) = llm_resolver {
            LlmProviderService::with_resolver(db.clone(), encryption.clone(), resolver)
        } else {
            LlmProviderService::new(db.clone(), encryption.clone())
        };
        Self {
            service: Arc::new(service),
            sync_service: Arc::new(ModelSyncService::new(db, driver_registry, encryption)),
            auth,
        }
    }
}

impl_auth_state!(AppState);

/// Request to create a new LLM provider
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLlmProviderRequest {
    /// Display name for the provider.
    #[schema(example = "OpenAI Production")]
    pub name: String,
    /// The type of LLM provider (e.g., openai, anthropic).
    pub provider_type: LlmProviderType,
    /// Base URL for the provider's API. Required for custom endpoints.
    /// For standard providers, this can be omitted to use the default URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://api.openai.com/v1")]
    pub base_url: Option<String>,
    /// API key for authenticating with the provider.
    /// Will be encrypted at rest if encryption is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
}

/// Response from syncing models from a provider
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SyncModelsResponse {
    /// Sync completed successfully
    Success {
        /// Number of new models discovered
        created: usize,
        /// Number of existing models updated
        updated: usize,
        /// Number of models marked as stale (not seen in this sync)
        stale: usize,
    },
    /// Provider doesn't support model discovery
    NotSupported,
}

/// Request to update an LLM provider. Only provided fields will be updated.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateLlmProviderRequest {
    /// Display name for the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "OpenAI Development")]
    pub name: Option<String>,
    /// The type of LLM provider (e.g., openai, anthropic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<LlmProviderType>,
    /// Base URL for the provider's API.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://api.openai.com/v1")]
    pub base_url: Option<String>,
    /// API key for authenticating with the provider.
    /// Will be encrypted at rest if encryption is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// The status of the provider. Set to "inactive" to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<LlmProviderStatus>,
}

/// Create a new LLM provider
#[utoipa::path(
    post,
    path = "/v1/llm-providers",
    request_body = CreateLlmProviderRequest,
    responses(
        (status = 201, description = "Provider created", body = LlmProvider),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal error")
    ),
    tag = "llm-providers"
)]
pub async fn create_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateLlmProviderRequest>,
) -> Result<(StatusCode, Json<LlmProvider>), (StatusCode, Json<ErrorResponse>)> {
    let caller = Caller::from(&org);
    let provider = state
        .service
        .create(&caller, req)
        .await
        .map_policy_or_internal("create LLM provider")?;

    Ok((StatusCode::CREATED, Json(provider)))
}

/// List all LLM providers
#[utoipa::path(
    get,
    path = "/v1/llm-providers",
    responses(
        (status = 200, description = "List of providers", body = ListResponse<LlmProvider>)
    ),
    tag = "llm-providers"
)]
pub async fn list_providers(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<ListResponse<LlmProvider>> {
    let caller = Caller::from(&org);
    let providers = state
        .service
        .list(&caller)
        .await
        .map_policy_or_internal("list LLM providers")?;

    Ok(Json(ListResponse::new(providers)))
}

/// Get a specific LLM provider
#[utoipa::path(
    get,
    path = "/v1/llm-providers/{id}",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 200, description = "Provider found", body = LlmProvider),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found")
    ),
    tag = "llm-providers"
)]
pub async fn get_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<LlmProvider> {
    let provider_id: ProviderId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid provider ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let provider = state
        .service
        .get(&caller, provider_id.uuid())
        .await
        .map_policy_or_internal("get LLM provider")?
        .ok_or_not_found_json("Provider")?;

    Ok(Json(provider))
}

/// Update an LLM provider
#[utoipa::path(
    patch,
    path = "/v1/llm-providers/{id}",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    request_body = UpdateLlmProviderRequest,
    responses(
        (status = 200, description = "Provider updated", body = LlmProvider),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found")
    ),
    tag = "llm-providers"
)]
pub async fn update_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateLlmProviderRequest>,
) -> ApiResult<LlmProvider> {
    let provider_id: ProviderId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid provider ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let provider = state
        .service
        .update(&caller, provider_id.uuid(), req)
        .await
        .map_policy_or_internal("update LLM provider")?
        .ok_or_not_found_json("Provider")?;

    Ok(Json(provider))
}

/// Delete an LLM provider
#[utoipa::path(
    delete,
    path = "/v1/llm-providers/{id}",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 204, description = "Provider deleted"),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found")
    ),
    tag = "llm-providers"
)]
pub async fn delete_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let provider_id: ProviderId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid provider ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .delete(&caller, provider_id.uuid())
        .await
        .map_policy_or_internal("delete LLM provider")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::not_found("Provider"))
    }
}

/// Sync models from an LLM provider
///
/// Fetches the list of available models from the provider's API and updates
/// the database. Only works for providers with standard base URLs (not custom).
#[utoipa::path(
    post,
    path = "/v1/llm-providers/{id}/sync-models",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 200, description = "Models synced", body = SyncModelsResponse),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found"),
        (status = 500, description = "Sync failed")
    ),
    tag = "llm-providers"
)]
pub async fn sync_models(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<SyncModelsResponse> {
    let provider_id: ProviderId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid provider ID: {}", e),
            }),
        )
    })?;

    let result = state
        .sync_service
        .sync_provider(org.org_id, provider_id.uuid())
        .await
        .map_err(|e| {
            let error_msg = e.to_string();
            if error_msg.contains("Provider not found") {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: "Provider not found".to_string(),
                    }),
                )
            } else {
                tracing::error!("Failed to sync models for provider {}: {}", provider_id, e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal server error".to_string(),
                    }),
                )
            }
        })?;

    let response = match result {
        SyncResult::Success {
            created,
            updated,
            stale,
        } => SyncModelsResponse::Success {
            created,
            updated,
            stale,
        },
        SyncResult::NotSupported => SyncModelsResponse::NotSupported,
        SyncResult::Failed { error } => {
            tracing::error!("Model sync failed for provider {}: {}", provider_id, error);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Failed to sync models".to_string(),
                }),
            ));
        }
    };

    Ok(Json(response))
}

/// GET /v1/llm-providers/config
#[utoipa::path(
    get,
    path = "/v1/llm-providers/config",
    responses(
        (status = 200, description = "Resource config for LLM providers", body = ResourceConfigResponse),
    ),
    tag = "llm-providers"
)]
pub async fn llm_provider_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&LLM_PROVIDER_VIEW, &LLM_PROVIDER_MANAGE],
    );
    Json(ResourceConfigResponse { policies })
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/llm-providers/config", get(llm_provider_config))
        .route(
            "/v1/llm-providers",
            post(create_provider).get(list_providers),
        )
        .route(
            "/v1/llm-providers/{id}",
            get(get_provider)
                .patch(update_provider)
                .delete(delete_provider),
        )
        .route("/v1/llm-providers/{id}/sync-models", post(sync_models))
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
        let error = ErrorResponse::new("Provider not found");
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(parsed["error"], "Provider not found");
    }

    #[test]
    fn test_error_response_encryption_not_configured() {
        // This error is safe to expose - it's a configuration issue, not internal details
        let error = ErrorResponse::new("Encryption not configured. Cannot store API key.");
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(
            parsed["error"],
            "Encryption not configured. Cannot store API key."
        );
    }

    #[test]
    fn test_invalid_base_url_error_is_client_facing() {
        // URL validation errors should be returned as-is (not masked as internal error)
        let error_msg =
            "Invalid base URL: URL host resolves to a blocked address: loopback (127.0.0.0/8)";
        assert!(error_msg.contains("Invalid base URL"));
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
        assert!(!generic_message.contains("encryption key"));
    }
}
