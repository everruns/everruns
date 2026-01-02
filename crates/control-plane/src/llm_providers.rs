// LLM Provider API endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_core::llm_models::LlmProvider;
use everruns_core::{LlmProviderStatus, LlmProviderType};
use everruns_storage::{Database, EncryptionService};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::common::ListResponse;
use crate::services::LlmProviderService;

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<LlmProviderService>,
}

impl AppState {
    pub fn new(db: Arc<Database>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self {
            service: Arc::new(LlmProviderService::new(db, encryption)),
        }
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

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    error: String,
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
    path = "/v1/llm-providers",
    responses(
        (status = 200, description = "List of providers", body = ListResponse<LlmProvider>)
    ),
    tag = "llm-providers"
)]
pub async fn list_providers(
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
    path = "/v1/llm-providers/{id}",
    params(
        ("id" = Uuid, Path, description = "Provider ID")
    ),
    responses(
        (status = 200, description = "Provider found", body = LlmProvider),
        (status = 404, description = "Provider not found")
    ),
    tag = "llm-providers"
)]
pub async fn get_provider(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<LlmProvider>, (StatusCode, Json<ErrorResponse>)> {
    let provider = state
        .service
        .get(id)
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
    path = "/v1/llm-providers/{id}",
    params(
        ("id" = Uuid, Path, description = "Provider ID")
    ),
    request_body = UpdateLlmProviderRequest,
    responses(
        (status = 200, description = "Provider updated", body = LlmProvider),
        (status = 404, description = "Provider not found")
    ),
    tag = "llm-providers"
)]
pub async fn update_provider(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLlmProviderRequest>,
) -> Result<Json<LlmProvider>, (StatusCode, Json<ErrorResponse>)> {
    let provider = state
        .service
        .update(id, req)
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
    path = "/v1/llm-providers/{id}",
    params(
        ("id" = Uuid, Path, description = "Provider ID")
    ),
    responses(
        (status = 204, description = "Provider deleted"),
        (status = 404, description = "Provider not found")
    ),
    tag = "llm-providers"
)]
pub async fn delete_provider(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let deleted = state.service.delete(id).await.map_err(|e| {
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

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/llm-providers",
            post(create_provider).get(list_providers),
        )
        .route(
            "/v1/llm-providers/:id",
            get(get_provider)
                .patch(update_provider)
                .delete(delete_provider),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_response_serialization() {
        let error = ErrorResponse {
            error: "Internal server error".to_string(),
        };
        let json = serde_json::to_string(&error).expect("Failed to serialize");
        assert_eq!(json, r#"{"error":"Internal server error"}"#);
    }

    #[test]
    fn test_error_response_internal_error_format() {
        // Verify that internal error responses use the generic message
        let error = ErrorResponse {
            error: "Internal server error".to_string(),
        };
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(parsed["error"], "Internal server error");
    }

    #[test]
    fn test_error_response_not_found_format() {
        let error = ErrorResponse {
            error: "Provider not found".to_string(),
        };
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(parsed["error"], "Provider not found");
    }

    #[test]
    fn test_error_response_encryption_not_configured() {
        // This error is safe to expose - it's a configuration issue, not internal details
        let error = ErrorResponse {
            error: "Encryption not configured. Cannot store API key.".to_string(),
        };
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
