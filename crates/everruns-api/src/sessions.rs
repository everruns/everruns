// Session CRUD HTTP routes

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_core::Session;
use everruns_storage::Database;

use crate::common::ListResponse;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Request to create a session
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub model_id: Option<Uuid>,
}

/// Request to update a session
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSessionRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

use crate::services::SessionService;

/// App state for sessions routes
#[derive(Clone)]
pub struct AppState {
    pub session_service: Arc<SessionService>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db)),
        }
    }
}

/// Create session routes (nested under agents)
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Session CRUD under agent
        .route(
            "/v1/agents/:agent_id/sessions",
            post(create_session).get(list_sessions),
        )
        .route(
            "/v1/agents/:agent_id/sessions/:session_id",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        .with_state(state)
}

/// POST /v1/agents/{agent_id}/sessions - Create a new session
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/sessions",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Session created successfully", body = Session),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn create_session(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), StatusCode> {
    let session = state
        .session_service
        .create(agent_id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to create session: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok((StatusCode::CREATED, Json(session)))
}

/// GET /v1/agents/{agent_id}/sessions - List sessions in agent
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "List of sessions", body = ListResponse<Session>),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn list_sessions(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
) -> Result<Json<ListResponse<Session>>, StatusCode> {
    let sessions = state.session_service.list(agent_id).await.map_err(|e| {
        tracing::error!("Failed to list sessions: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ListResponse::new(sessions)))
}

/// GET /v1/agents/{agent_id}/sessions/{session_id} - Get session
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/sessions/{session_id}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Session found", body = Session),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Session>, StatusCode> {
    let session = state
        .session_service
        .get(session_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get session: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(session))
}

/// PATCH /v1/agents/{agent_id}/sessions/{session_id} - Update session
#[utoipa::path(
    patch,
    path = "/v1/agents/{agent_id}/sessions/{session_id}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    request_body = UpdateSessionRequest,
    responses(
        (status = 200, description = "Session updated successfully", body = Session),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn update_session(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<Session>, StatusCode> {
    let session = state
        .session_service
        .update(session_id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update session: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(session))
}

/// DELETE /v1/agents/{agent_id}/sessions/{session_id} - Delete session
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}/sessions/{session_id}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    responses(
        (status = 204, description = "Session deleted successfully"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn delete_session(
    State(state): State<AppState>,
    Path((_agent_id, session_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state
        .session_service
        .delete(session_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete session: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}
