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
use serde_json::json;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Request to create a session
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// Human-readable title for the session.
    #[serde(default)]
    #[schema(example = "Debug login issue")]
    pub title: Option<String>,
    /// Tags for organizing and filtering sessions.
    #[serde(default)]
    #[schema(example = json!(["debugging", "urgent"]))]
    pub tags: Vec<String>,
    /// The ID of the LLM model to use for this session.
    /// Overrides the agent's default model if specified.
    #[serde(default)]
    pub model_id: Option<Uuid>,
}

/// Request to update a session. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSessionRequest {
    /// Human-readable title for the session.
    #[serde(default)]
    #[schema(example = "Updated session title")]
    pub title: Option<String>,
    /// Tags for organizing and filtering sessions.
    #[serde(default)]
    #[schema(example = json!(["resolved"]))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_session_request_minimal() {
        // Test with minimal fields (all optional)
        let json = r#"{}"#;
        let req: CreateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, None);
        assert!(req.tags.is_empty());
        assert_eq!(req.model_id, None);
    }

    #[test]
    fn test_create_session_request_with_title() {
        let json = r#"{"title": "Test Session"}"#;
        let req: CreateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, Some("Test Session".to_string()));
        assert!(req.tags.is_empty());
        assert_eq!(req.model_id, None);
    }

    #[test]
    fn test_create_session_request_with_model_id() {
        let model_uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let json = format!(r#"{{"model_id": "{}"}}"#, model_uuid);
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.model_id, Some(model_uuid));
    }

    #[test]
    fn test_create_session_request_full() {
        let model_uuid = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap();
        let json = format!(
            r#"{{"title": "Full Session", "tags": ["tag1", "tag2"], "model_id": "{}"}}"#,
            model_uuid
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.title, Some("Full Session".to_string()));
        assert_eq!(req.tags, vec!["tag1", "tag2"]);
        assert_eq!(req.model_id, Some(model_uuid));
    }

    #[test]
    fn test_update_session_request_minimal() {
        let json = r#"{}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, None);
        assert_eq!(req.tags, None);
    }

    #[test]
    fn test_update_session_request_with_title() {
        let json = r#"{"title": "Updated Title"}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, Some("Updated Title".to_string()));
        assert_eq!(req.tags, None);
    }

    #[test]
    fn test_update_session_request_with_tags() {
        let json = r#"{"tags": ["new-tag"]}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, None);
        assert_eq!(req.tags, Some(vec!["new-tag".to_string()]));
    }
}
