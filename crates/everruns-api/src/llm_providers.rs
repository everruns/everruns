// LLM Provider API endpoints

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_contracts::{LlmProvider, LlmProviderStatus, LlmProviderType};
use everruns_storage::{
    models::{CreateLlmProvider, UpdateLlmProvider},
    Database, EncryptionService,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
    pub encryption: Option<Arc<EncryptionService>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLlmProviderRequest {
    pub name: String,
    pub provider_type: LlmProviderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateLlmProviderRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<LlmProviderType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_default: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<LlmProviderStatus>,
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    error: String,
}

fn row_to_provider(row: &everruns_storage::models::LlmProviderRow) -> LlmProvider {
    LlmProvider {
        id: row.id,
        name: row.name.clone(),
        provider_type: row.provider_type.parse().unwrap_or(LlmProviderType::Custom),
        base_url: row.base_url.clone(),
        api_key_set: row.api_key_set,
        is_default: row.is_default,
        status: match row.status.as_str() {
            "active" => LlmProviderStatus::Active,
            _ => LlmProviderStatus::Disabled,
        },
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
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
    // Encrypt API key if provided and encryption is available
    let api_key_encrypted = if let Some(api_key) = &req.api_key {
        if let Some(encryption) = &state.encryption {
            Some(encryption.encrypt_string(api_key).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to encrypt API key: {}", e),
                    }),
                )
            })?)
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Encryption not configured. Cannot store API key.".to_string(),
                }),
            ));
        }
    } else {
        None
    };

    let input = CreateLlmProvider {
        name: req.name,
        provider_type: req.provider_type.to_string(),
        base_url: req.base_url,
        api_key_encrypted,
        is_default: req.is_default,
    };

    let row = state.db.create_llm_provider(input).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    Ok((StatusCode::CREATED, Json(row_to_provider(&row))))
}

/// List all LLM providers
#[utoipa::path(
    get,
    path = "/v1/llm-providers",
    responses(
        (status = 200, description = "List of providers", body = Vec<LlmProvider>)
    ),
    tag = "llm-providers"
)]
pub async fn list_providers(
    State(state): State<AppState>,
) -> Result<Json<Vec<LlmProvider>>, (StatusCode, Json<ErrorResponse>)> {
    let rows = state.db.list_llm_providers().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    Ok(Json(rows.iter().map(row_to_provider).collect()))
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
    let row = state.db.get_llm_provider(id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    match row {
        Some(r) => Ok(Json(row_to_provider(&r))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Provider not found".to_string(),
            }),
        )),
    }
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
    // Encrypt API key if provided
    let api_key_encrypted = if let Some(api_key) = &req.api_key {
        if let Some(encryption) = &state.encryption {
            Some(encryption.encrypt_string(api_key).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: format!("Failed to encrypt API key: {}", e),
                    }),
                )
            })?)
        } else {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Encryption not configured. Cannot store API key.".to_string(),
                }),
            ));
        }
    } else {
        None
    };

    let input = UpdateLlmProvider {
        name: req.name,
        provider_type: req.provider_type.map(|t| t.to_string()),
        base_url: req.base_url,
        api_key_encrypted,
        is_default: req.is_default,
        status: req.status.map(|s| match s {
            LlmProviderStatus::Active => "active".to_string(),
            LlmProviderStatus::Disabled => "disabled".to_string(),
        }),
    };

    let row = state.db.update_llm_provider(id, input).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    match row {
        Some(r) => Ok(Json(row_to_provider(&r))),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Provider not found".to_string(),
            }),
        )),
    }
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
    let deleted = state.db.delete_llm_provider(id).await.map_err(|e| {
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
