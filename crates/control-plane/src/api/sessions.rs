// Session CRUD HTTP routes
// Routes are org-scoped: /v1/orgs/:org/agents/:agent_id/sessions/...

use crate::auth::{AuthState, OrgContext, middleware::FromRef};
use crate::services::{EventService, SessionService};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::events::{EventContext, EventRequest, MessageUserData, TurnCancelledData};
use everruns_core::{Message, Session};
use everruns_worker::AgentRunner;

use super::common::{ApiOptionExt, ApiResultExt, PaginatedResponse, Pagination};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};
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

/// Response from cancel turn endpoint
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CancelTurnResponse {
    /// Whether the cancellation was performed or was a no-op
    #[schema(example = "cancelled")]
    pub status: CancelStatus,
    /// Human-readable message
    #[schema(example = "Turn cancelled successfully")]
    pub message: String,
}

/// Status of the cancel operation
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CancelStatus {
    /// Turn was actively cancelled
    Cancelled,
    /// No turn was running, cancel was a no-op
    NoOp,
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

/// Query parameters for listing sessions with pagination.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListSessionsQuery {
    /// Number of items to skip (for pagination).
    #[param(minimum = 0, default = 0)]
    pub offset: Option<u32>,
    /// Maximum number of items to return (for pagination).
    #[param(minimum = 1, maximum = 100, default = 20)]
    pub limit: Option<u32>,
}

/// Default limit for session listing
const DEFAULT_LIMIT: u32 = 20;
/// Maximum allowed limit for session listing
const MAX_LIMIT: u32 = 100;

/// App state for sessions routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub event_service: EventService,
    pub runner: Arc<dyn AgentRunner>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, runner: Arc<dyn AgentRunner>, auth: AuthState) -> Self {
        Self {
            session_service: Arc::new(SessionService::new(db.clone())),
            event_service: EventService::new(db.clone()),
            db,
            runner,
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create session routes (nested under agents, org-scoped)
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Session CRUD under agent (org-scoped)
        .route(
            "/v1/orgs/:org/agents/:agent_id/sessions",
            post(create_session).get(list_sessions),
        )
        .route(
            "/v1/orgs/:org/agents/:agent_id/sessions/:session_id",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        // Cancel turn endpoint
        .route(
            "/v1/orgs/:org/agents/:agent_id/sessions/:session_id/cancel",
            post(cancel_turn),
        )
        .with_state(state)
}

/// POST /v1/orgs/{org}/agents/{agent_id}/sessions - Create a new session
#[utoipa::path(
    post,
    path = "/v1/orgs/{org}/agents/{agent_id}/sessions",
    params(
        ("org" = String, Path, description = "Organization public ID"),
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
    org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, agent_id)): Path<(String, Uuid)>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), StatusCode> {
    let session = state
        .session_service
        .create(org.org_id, agent_id, req)
        .await
        .log_internal_error("create session")?;

    Ok((StatusCode::CREATED, Json(session)))
}

/// GET /v1/orgs/{org}/agents/{agent_id}/sessions - List sessions in agent
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/agents/{agent_id}/sessions",
    params(
        ("org" = String, Path, description = "Organization public ID"),
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ListSessionsQuery
    ),
    responses(
        (status = 200, description = "Paginated list of sessions", body = PaginatedResponse<Session>),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn list_sessions(
    org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, agent_id)): Path<(String, Uuid)>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<PaginatedResponse<Session>>, StatusCode> {
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let pagination = Pagination::new(offset, limit);

    let (sessions, total) = state
        .session_service
        .list(org.org_id, agent_id, pagination)
        .await
        .log_internal_error("list sessions")?;

    Ok(Json(PaginatedResponse::new(sessions, total, offset, limit)))
}

/// GET /v1/orgs/{org}/agents/{agent_id}/sessions/{session_id} - Get session
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
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
    org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, _agent_id, session_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Json<Session>, StatusCode> {
    let session = state
        .session_service
        .get(org.org_id, session_id)
        .await
        .log_internal_error("get session")?
        .ok_or_not_found()?;

    Ok(Json(session))
}

/// PATCH /v1/orgs/{org}/agents/{agent_id}/sessions/{session_id} - Update session
#[utoipa::path(
    patch,
    path = "/v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
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
    org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, _agent_id, session_id)): Path<(String, Uuid, Uuid)>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<Session>, StatusCode> {
    let session = state
        .session_service
        .update(org.org_id, session_id, req)
        .await
        .log_internal_error("update session")?
        .ok_or_not_found()?;

    Ok(Json(session))
}

/// DELETE /v1/orgs/{org}/agents/{agent_id}/sessions/{session_id} - Delete session
#[utoipa::path(
    delete,
    path = "/v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
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
    org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, _agent_id, session_id)): Path<(String, Uuid, Uuid)>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state
        .session_service
        .delete(org.org_id, session_id)
        .await
        .log_internal_error("delete session")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// POST /v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}/cancel - Cancel current turn
///
/// Cancels the currently running turn in the session. If no turn is running,
/// this is a no-op and returns success (idempotent). When a turn is active:
/// 1. Cancel the underlying workflow execution
/// 2. Emit a turn.cancelled event
/// 3. Insert an agent message indicating the turn was cancelled
/// 4. Set the session status back to idle
#[utoipa::path(
    post,
    path = "/v1/orgs/{org}/agents/{agent_id}/sessions/{session_id}/cancel",
    params(
        ("org" = String, Path, description = "Organization public ID"),
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("session_id" = Uuid, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Turn cancelled successfully (or no-op if idle)", body = CancelTurnResponse),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn cancel_turn(
    org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, _agent_id, session_id)): Path<(String, Uuid, Uuid)>,
) -> Result<Json<CancelTurnResponse>, StatusCode> {
    // Verify session exists
    let session = state
        .session_service
        .get(org.org_id, session_id)
        .await
        .log_internal_error("get session for cancel")?
        .ok_or_not_found()?;

    // If session is not active, cancel is a no-op (idempotent)
    if session.status != everruns_core::SessionStatus::Active {
        return Ok(Json(CancelTurnResponse {
            status: CancelStatus::NoOp,
            message: "No turn currently running".to_string(),
        }));
    }

    // Cancel the workflow
    if let Err(e) = state.runner.cancel_run(session_id).await {
        tracing::error!(session_id = %session_id, error = %e, "Failed to cancel workflow");
        // Continue anyway - workflow may have already completed
    }

    // Generate IDs for the turn context
    // Use session_id as turn_id since workflow_id = session_id in durable runner
    let turn_id = session_id;
    let input_message_id = Uuid::now_v7(); // Placeholder since we don't have the original

    // Emit turn.cancelled event
    let cancelled_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        TurnCancelledData {
            turn_id,
            reason: Some("User requested cancellation".to_string()),
            usage: None, // Usage not available at cancellation time
        },
    );
    if let Err(e) = state.event_service.emit(cancelled_event).await {
        tracing::warn!(session_id = %session_id, error = %e, "Failed to emit turn.cancelled event");
    }

    // Insert user message indicating cancellation request
    let user_cancel_message = Message::user("User requested to cancel the work.");
    let user_message_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        MessageUserData::new(user_cancel_message),
    );
    if let Err(e) = state.event_service.emit(user_message_event).await {
        tracing::warn!(session_id = %session_id, error = %e, "Failed to emit user cancellation message");
    }

    // Note: Agent message "Work was cancelled by user." and session.idled event
    // are emitted by the worker when it detects the cancellation and stops.
    // This ensures the agent message appears AFTER any in-flight events.

    Ok(Json(CancelTurnResponse {
        status: CancelStatus::Cancelled,
        message: "Turn cancelled successfully".to_string(),
    }))
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
