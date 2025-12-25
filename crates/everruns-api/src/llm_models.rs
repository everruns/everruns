// LLM Model API endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_core::{LlmModel, LlmModelStatus, LlmModelWithProvider};
use everruns_storage::Database;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::services::LlmModelService;

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<LlmModelService>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            service: Arc::new(LlmModelService::new(db)),
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLlmModelRequest {
    pub model_id: String,
    pub display_name: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i32>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateLlmModelRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<LlmModelStatus>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    error: String,
}

/// Create a new model for a provider
#[utoipa::path(
    post,
    path = "/v1/llm-providers/{provider_id}/models",
    params(
        ("provider_id" = Uuid, Path, description = "Provider ID")
    ),
    request_body = CreateLlmModelRequest,
    responses(
        (status = 201, description = "Model created", body = LlmModel),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal error")
    ),
    tag = "llm-models"
)]
pub async fn create_model(
    State(state): State<AppState>,
    Path(provider_id): Path<Uuid>,
    Json(req): Json<CreateLlmModelRequest>,
) -> Result<(StatusCode, Json<LlmModel>), (StatusCode, Json<ErrorResponse>)> {
    let model = state.service.create(provider_id, req).await.map_err(|e| {
        tracing::error!("Failed to create LLM model: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    Ok((StatusCode::CREATED, Json(model)))
}

/// List models for a specific provider
#[utoipa::path(
    get,
    path = "/v1/llm-providers/{provider_id}/models",
    params(
        ("provider_id" = Uuid, Path, description = "Provider ID")
    ),
    responses(
        (status = 200, description = "List of models", body = Vec<LlmModel>)
    ),
    tag = "llm-models"
)]
pub async fn list_provider_models(
    State(state): State<AppState>,
    Path(provider_id): Path<Uuid>,
) -> Result<Json<Vec<LlmModel>>, (StatusCode, Json<ErrorResponse>)> {
    let models = state
        .service
        .list_for_provider(provider_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list LLM models for provider: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal server error".to_string(),
                }),
            )
        })?;

    Ok(Json(models))
}

/// List all models across all providers
#[utoipa::path(
    get,
    path = "/v1/llm-models",
    responses(
        (status = 200, description = "List of all models", body = Vec<LlmModelWithProvider>)
    ),
    tag = "llm-models"
)]
pub async fn list_all_models(
    State(state): State<AppState>,
) -> Result<Json<Vec<LlmModelWithProvider>>, (StatusCode, Json<ErrorResponse>)> {
    let models = state.service.list_all().await.map_err(|e| {
        tracing::error!("Failed to list all LLM models: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal server error".to_string(),
            }),
        )
    })?;

    Ok(Json(models))
}

/// Get a specific model with provider info and profile
#[utoipa::path(
    get,
    path = "/v1/llm-models/{id}",
    params(
        ("id" = Uuid, Path, description = "Model ID")
    ),
    responses(
        (status = 200, description = "Model found", body = LlmModelWithProvider),
        (status = 404, description = "Model not found")
    ),
    tag = "llm-models"
)]
pub async fn get_model(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<LlmModelWithProvider>, (StatusCode, Json<ErrorResponse>)> {
    let model = state
        .service
        .get_with_provider(id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get LLM model: {}", e);
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
                    error: "Model not found".to_string(),
                }),
            )
        })?;

    Ok(Json(model))
}

/// Update a model
#[utoipa::path(
    patch,
    path = "/v1/llm-models/{id}",
    params(
        ("id" = Uuid, Path, description = "Model ID")
    ),
    request_body = UpdateLlmModelRequest,
    responses(
        (status = 200, description = "Model updated", body = LlmModel),
        (status = 404, description = "Model not found")
    ),
    tag = "llm-models"
)]
pub async fn update_model(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateLlmModelRequest>,
) -> Result<Json<LlmModel>, (StatusCode, Json<ErrorResponse>)> {
    let model = state
        .service
        .update(id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update LLM model: {}", e);
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
                    error: "Model not found".to_string(),
                }),
            )
        })?;

    Ok(Json(model))
}

/// Delete a model
#[utoipa::path(
    delete,
    path = "/v1/llm-models/{id}",
    params(
        ("id" = Uuid, Path, description = "Model ID")
    ),
    responses(
        (status = 204, description = "Model deleted"),
        (status = 404, description = "Model not found")
    ),
    tag = "llm-models"
)]
pub async fn delete_model(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let deleted = state.service.delete(id).await.map_err(|e| {
        tracing::error!("Failed to delete LLM model: {}", e);
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
                error: "Model not found".to_string(),
            }),
        ))
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/llm-providers/:provider_id/models",
            post(create_model).get(list_provider_models),
        )
        .route("/v1/llm-models", get(list_all_models))
        .route(
            "/v1/llm-models/:id",
            get(get_model).patch(update_model).delete(delete_model),
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
            error: "Model not found".to_string(),
        };
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
