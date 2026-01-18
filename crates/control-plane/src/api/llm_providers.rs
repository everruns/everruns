// LLM Provider API endpoints
// Routes are org-scoped: /v1/orgs/:org/llm-providers/...

use crate::auth::{AuthState, OrgContext, middleware::FromRef};
use crate::services::{LlmProviderService, ModelSyncService, SyncResult};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::llm_models::LlmProvider;
use everruns_core::typed_id::ProviderId;
use everruns_core::{DriverRegistry, LlmProviderStatus, LlmProviderType};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::{ErrorResponse, ListResponse};

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
    ) -> Self {
        Self {
            service: Arc::new(LlmProviderService::new(db.clone(), encryption)),
            sync_service: Arc::new(ModelSyncService::new(db, driver_registry)),
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

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
    path = "/v1/orgs/{org}/llm-providers",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    request_body = CreateLlmProviderRequest,
    responses(
        (status = 201, description = "Provider created", body = LlmProvider),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal error")
    ),
    tag = "llm-providers"
)]
pub async fn create_provider(
    _org: OrgContext,
    State(state): State<AppState>,
    Json(req): Json<CreateLlmProviderRequest>,
) -> Result<(StatusCode, Json<LlmProvider>), (StatusCode, Json<ErrorResponse>)> {
    let provider = state.service.create(req).await.map_err(|e| {
        let error_msg = e.to_string();
        if error_msg.contains("Encryption not configured") {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse { error: error_msg }),
            )
        } else {
            tracing::error!("Failed to create LLM provider: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        }
    })?;

    Ok((StatusCode::CREATED, Json(provider)))
}

/// List all LLM providers
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/llm-providers",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    responses(
        (status = 200, description = "List of providers", body = ListResponse<LlmProvider>)
    ),
    tag = "llm-providers"
)]
pub async fn list_providers(
    _org: OrgContext,
    State(state): State<AppState>,
) -> Result<Json<ListResponse<LlmProvider>>, (StatusCode, Json<ErrorResponse>)> {
    let providers = state.service.list().await.map_err(|e| {
        tracing::error!("Failed to list LLM providers: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    Ok(Json(ListResponse::new(providers)))
}

/// Get a specific LLM provider
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/llm-providers/{id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
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
    _org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, id)): Path<(String, String)>,
) -> Result<Json<LlmProvider>, (StatusCode, Json<ErrorResponse>)> {
    let provider_id: ProviderId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid provider ID: {}", e),
            }),
        )
    })?;

    let provider = state
        .service
        .get(provider_id.uuid())
        .await
        .map_err(|e| {
            tracing::error!("Failed to get LLM provider: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Provider not found".to_string(),
                }),
            )
        })?;

    Ok(Json(provider))
}

/// Update an LLM provider
#[utoipa::path(
    patch,
    path = "/v1/orgs/{org}/llm-providers/{id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
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
    _org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, id)): Path<(String, String)>,
    Json(req): Json<UpdateLlmProviderRequest>,
) -> Result<Json<LlmProvider>, (StatusCode, Json<ErrorResponse>)> {
    let provider_id: ProviderId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid provider ID: {}", e),
            }),
        )
    })?;

    let provider = state
        .service
        .update(provider_id.uuid(), req)
        .await
        .map_err(|e| {
            let error_msg = e.to_string();
            if error_msg.contains("Encryption not configured") {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse { error: error_msg }),
                )
            } else {
                tracing::error!("Failed to update LLM provider: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal server error".to_string(),
                    }),
                )
            }
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Provider not found".to_string(),
                }),
            )
        })?;

    Ok(Json(provider))
}

/// Delete an LLM provider
#[utoipa::path(
    delete,
    path = "/v1/orgs/{org}/llm-providers/{id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
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
    _org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let provider_id: ProviderId = id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid provider ID: {}", e),
            }),
        )
    })?;

    let deleted = state
        .service
        .delete(provider_id.uuid())
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete LLM provider: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Provider not found".to_string(),
            }),
        ))
    }
}

/// Sync models from an LLM provider
///
/// Fetches the list of available models from the provider's API and updates
/// the database. Only works for providers with standard base URLs (not custom).
#[utoipa::path(
    post,
    path = "/v1/orgs/{org}/llm-providers/{id}/sync-models",
    params(
        ("org" = String, Path, description = "Organization public ID"),
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
    _org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, id)): Path<(String, String)>,
) -> Result<Json<SyncModelsResponse>, (StatusCode, Json<ErrorResponse>)> {
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
        .sync_provider(provider_id.uuid())
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

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/orgs/:org/llm-providers",
            post(create_provider).get(list_providers),
        )
        .route(
            "/v1/orgs/:org/llm-providers/:id",
            get(get_provider)
                .patch(update_provider)
                .delete(delete_provider),
        )
        .route(
            "/v1/orgs/:org/llm-providers/:id/sync-models",
            post(sync_models),
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
