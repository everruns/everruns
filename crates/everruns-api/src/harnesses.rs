// Harness CRUD HTTP routes (M2)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_contracts::{CreateHarnessRequest, Harness, ListResponse, UpdateHarnessRequest};
use everruns_storage::{
    models::{CreateHarness, UpdateHarness},
    Database,
};
use std::sync::Arc;
use uuid::Uuid;

use crate::services::HarnessService;

/// App state for harnesses routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<HarnessService>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            service: Arc::new(HarnessService::new(db)),
        }
    }
}

/// Create harness routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/harnesses", post(create_harness).get(list_harnesses))
        .route(
            "/v1/harnesses/{harness_id}",
            get(get_harness)
                .patch(update_harness)
                .delete(delete_harness),
        )
        .route("/v1/harnesses/slug/{slug}", get(get_harness_by_slug))
        .with_state(state)
}

/// POST /v1/harnesses - Create a new harness
#[utoipa::path(
    post,
    path = "/v1/harnesses",
    request_body = CreateHarnessRequest,
    responses(
        (status = 201, description = "Harness created successfully", body = Harness),
        (status = 500, description = "Internal server error")
    ),
    tag = "harnesses"
)]
pub async fn create_harness(
    State(state): State<AppState>,
    Json(req): Json<CreateHarnessRequest>,
) -> Result<(StatusCode, Json<Harness>), StatusCode> {
    let input = CreateHarness {
        slug: req.slug,
        display_name: req.display_name,
        description: req.description,
        system_prompt: req.system_prompt,
        default_model_id: req.default_model_id,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        tags: req.tags,
    };

    let harness = state.service.create(input).await.map_err(|e| {
        tracing::error!("Failed to create harness: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok((StatusCode::CREATED, Json(harness)))
}

/// GET /v1/harnesses - List all active harnesses
#[utoipa::path(
    get,
    path = "/v1/harnesses",
    responses(
        (status = 200, description = "List of harnesses", body = ListResponse<Harness>),
        (status = 500, description = "Internal server error")
    ),
    tag = "harnesses"
)]
pub async fn list_harnesses(
    State(state): State<AppState>,
) -> Result<Json<ListResponse<Harness>>, StatusCode> {
    let harnesses = state.service.list().await.map_err(|e| {
        tracing::error!("Failed to list harnesses: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ListResponse::new(harnesses)))
}

/// GET /v1/harnesses/{harness_id} - Get harness by ID
#[utoipa::path(
    get,
    path = "/v1/harnesses/{harness_id}",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID")
    ),
    responses(
        (status = 200, description = "Harness found", body = Harness),
        (status = 404, description = "Harness not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "harnesses"
)]
pub async fn get_harness(
    State(state): State<AppState>,
    Path(harness_id): Path<Uuid>,
) -> Result<Json<Harness>, StatusCode> {
    let harness = state
        .service
        .get(harness_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get harness: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(harness))
}

/// GET /v1/harnesses/slug/{slug} - Get harness by slug
#[utoipa::path(
    get,
    path = "/v1/harnesses/slug/{slug}",
    params(
        ("slug" = String, Path, description = "Harness slug")
    ),
    responses(
        (status = 200, description = "Harness found", body = Harness),
        (status = 404, description = "Harness not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "harnesses"
)]
pub async fn get_harness_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Harness>, StatusCode> {
    let harness = state
        .service
        .get_by_slug(&slug)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get harness by slug: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(harness))
}

/// PATCH /v1/harnesses/{harness_id} - Update harness
#[utoipa::path(
    patch,
    path = "/v1/harnesses/{harness_id}",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID")
    ),
    request_body = UpdateHarnessRequest,
    responses(
        (status = 200, description = "Harness updated successfully", body = Harness),
        (status = 404, description = "Harness not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "harnesses"
)]
pub async fn update_harness(
    State(state): State<AppState>,
    Path(harness_id): Path<Uuid>,
    Json(req): Json<UpdateHarnessRequest>,
) -> Result<Json<Harness>, StatusCode> {
    let input = UpdateHarness {
        slug: req.slug,
        display_name: req.display_name,
        description: req.description,
        system_prompt: req.system_prompt,
        default_model_id: req.default_model_id,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        tags: req.tags,
        status: req.status.map(|s| s.to_string()),
    };

    let harness = state
        .service
        .update(harness_id, input)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update harness: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(harness))
}

/// DELETE /v1/harnesses/{harness_id} - Archive harness
#[utoipa::path(
    delete,
    path = "/v1/harnesses/{harness_id}",
    params(
        ("harness_id" = Uuid, Path, description = "Harness ID")
    ),
    responses(
        (status = 204, description = "Harness archived successfully"),
        (status = 404, description = "Harness not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "harnesses"
)]
pub async fn delete_harness(
    State(state): State<AppState>,
    Path(harness_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state.service.delete(harness_id).await.map_err(|e| {
        tracing::error!("Failed to delete harness: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
