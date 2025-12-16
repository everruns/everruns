// LLM Model API endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
    Json, Router,
};
use everruns_contracts::{LlmModel, LlmModelStatus, LlmModelWithProvider, LlmProviderType};
use everruns_storage::{
    models::{CreateLlmModel, UpdateLlmModel},
    Database,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
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

fn row_to_model(row: &everruns_storage::models::LlmModelRow) -> LlmModel {
    let capabilities: Vec<String> =
        serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
    LlmModel {
        id: row.id,
        provider_id: row.provider_id,
        model_id: row.model_id.clone(),
        display_name: row.display_name.clone(),
        capabilities,
        context_window: row.context_window,
        is_default: row.is_default,
        status: match row.status.as_str() {
            "active" => LlmModelStatus::Active,
            _ => LlmModelStatus::Disabled,
        },
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn row_to_model_with_provider(
    row: &everruns_storage::models::LlmModelWithProviderRow,
) -> LlmModelWithProvider {
    let capabilities: Vec<String> =
        serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
    LlmModelWithProvider {
        id: row.id,
        provider_id: row.provider_id,
        model_id: row.model_id.clone(),
        display_name: row.display_name.clone(),
        capabilities,
        context_window: row.context_window,
        is_default: row.is_default,
        status: match row.status.as_str() {
            "active" => LlmModelStatus::Active,
            _ => LlmModelStatus::Disabled,
        },
        created_at: row.created_at,
        updated_at: row.updated_at,
        provider_name: row.provider_name.clone(),
        provider_type: row.provider_type.parse().unwrap_or(LlmProviderType::Custom),
    }
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
    let input = CreateLlmModel {
        provider_id,
        model_id: req.model_id,
        display_name: req.display_name,
        capabilities: req.capabilities,
        context_window: req.context_window,
        is_default: req.is_default,
    };

    let row = state.db.create_llm_model(input).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    Ok((StatusCode::CREATED, Json(row_to_model(&row))))
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
    let rows = state
        .db
        .list_llm_models_for_provider(provider_id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;

    Ok(Json(rows.iter().map(row_to_model).collect()))
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
    let rows = state.db.list_all_llm_models().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    Ok(Json(rows.iter().map(row_to_model_with_provider).collect()))
}

/// Get a specific model
#[utoipa::path(
    get,
    path = "/v1/llm-models/{id}",
    params(
        ("id" = Uuid, Path, description = "Model ID")
    ),
    responses(
        (status = 200, description = "Model found", body = LlmModel),
        (status = 404, description = "Model not found")
    ),
    tag = "llm-models"
)]
pub async fn get_model(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<LlmModel>, (StatusCode, Json<ErrorResponse>)> {
    let row = state.db.get_llm_model(id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    match row {
        Some(r) => Ok(Json(row_to_model(&r))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Model not found".to_string(),
            }),
        )),
    }
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
    let input = UpdateLlmModel {
        model_id: req.model_id,
        display_name: req.display_name,
        capabilities: req.capabilities,
        context_window: req.context_window,
        is_default: req.is_default,
        status: req.status.map(|s| match s {
            LlmModelStatus::Active => "active".to_string(),
            LlmModelStatus::Disabled => "disabled".to_string(),
        }),
    };

    let row = state.db.update_llm_model(id, input).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    match row {
        Some(r) => Ok(Json(row_to_model(&r))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Model not found".to_string(),
            }),
        )),
    }
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
    let deleted = state.db.delete_llm_model(id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
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
        .route("/v1/llm-providers/{provider_id}/models", post(create_model))
        .route(
            "/v1/llm-providers/{provider_id}/models",
            get(list_provider_models),
        )
        .route("/v1/llm-models", get(list_all_models))
        .route("/v1/llm-models/{id}", get(get_model))
        .route("/v1/llm-models/{id}", patch(update_model))
        .route("/v1/llm-models/{id}", delete(delete_model))
        .with_state(state)
}
