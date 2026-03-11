// Session CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::{EventService, SessionService};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::capability_types::AgentCapabilityConfig;
use everruns_core::events::{EventContext, EventRequest, InputMessageData, TurnCancelledData};
use everruns_core::typed_id::{AgentId, HarnessId, MessageId, ModelId, SessionId, TurnId};
use everruns_core::{Message, Session};
use everruns_worker::AgentRunner;

use super::common::{ApiOptionExt, ApiResultExt, ErrorResponse, PaginatedResponse, Pagination};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Request to create a session
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// ID of the harness for this session (format: harness_{32-hex}).
    #[schema(value_type = String, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: HarnessId,
    /// ID of the agent to work in this session (optional, format: agent_{32-hex}).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
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
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub model_id: Option<ModelId>,
    /// Session-level capabilities (additive to agent capabilities).
    /// Applied after agent capabilities when building RuntimeAgent.
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Client-side tools for this session (additive to agent tools).
    /// These tools are sent to the LLM but executed by the client.
    #[serde(default)]
    pub tools: Vec<everruns_core::ToolDefinition>,
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
    /// Filter sessions by agent ID.
    #[param(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// Search by title (case-insensitive substring match).
    pub search: Option<String>,
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

/// Response for session statistics endpoint
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionStatsResponse {
    /// Total number of sessions across all statuses
    pub total: u32,
    /// Sessions with a turn currently running
    pub active: u32,
    /// Sessions waiting for next input
    pub idle: u32,
    /// Sessions just created, no turn executed yet
    pub started: u32,
    /// Sessions waiting for client-side tool results
    pub waiting_for_tool_results: u32,
}

/// Create session routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Global chat session (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/chat", post(get_or_create_chat_session))
        // Session stats (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/stats", get(get_session_stats))
        // Session CRUD
        .route("/v1/sessions", post(create_session).get(list_sessions))
        .route(
            "/v1/sessions/{session_id}",
            get(get_session)
                .patch(update_session)
                .delete(delete_session),
        )
        // Pin/unpin
        .route(
            "/v1/sessions/{session_id}/pin",
            axum::routing::put(pin_session).delete(unpin_session),
        )
        // Cancel turn endpoint
        .route("/v1/sessions/{session_id}/cancel", post(cancel_turn))
        .with_state(state)
}

/// POST /v1/sessions - Create a new session
#[utoipa::path(
    post,
    path = "/v1/sessions",
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Session created successfully", body = Session),
        (status = 400, description = "Invalid agent ID"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn create_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), (StatusCode, Json<ErrorResponse>)> {
    // Resolve agent public_id to internal UUID for FK storage (if agent specified)
    let (agent_internal_id, agent_public_id) = if let Some(ref agent_id) = req.agent_id {
        let agent_row = state
            .db
            .get_agent_by_public_id(org.org_id, &agent_id.to_string())
            .await
            .log_internal_error_json("resolve agent")?
            .ok_or_not_found_json("Agent")?;

        let public_id: AgentId = agent_row
            .public_id
            .parse()
            .unwrap_or_else(|_| AgentId::from_uuid(agent_row.id.uuid()));

        (Some(agent_row.id.uuid()), Some(public_id))
    } else {
        (None, None)
    };

    let session = state
        .session_service
        .create(
            org.org_id,
            &org.public_id,
            req.harness_id.uuid(),
            agent_internal_id,
            agent_public_id,
            req,
        )
        .await
        .log_internal_error_json("create session")?;

    Ok((StatusCode::CREATED, Json(session)))
}

/// POST /v1/sessions/chat - Get or create global chat session
///
/// Returns the user's singleton global chat session. Creates one if it doesn't exist.
/// Uses the Platform Chat harness and tags for per-user singleton management.
#[utoipa::path(
    post,
    path = "/v1/sessions/chat",
    responses(
        (status = 200, description = "Chat session returned", body = Session),
        (status = 401, description = "Authentication required"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_or_create_chat_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<Session>, (StatusCode, Json<ErrorResponse>)> {
    // Use authenticated user_id, or fall back to anonymous user (auth=none mode)
    let user_id = org.user_id.unwrap_or(everruns_core::ANONYMOUS_USER_ID);

    let session = state
        .session_service
        .get_or_create_chat_session(
            org.org_id,
            &org.public_id,
            user_id,
            crate::seed::CHAT_HARNESS_ID,
        )
        .await
        .log_internal_error_json("get or create chat session")?;

    Ok(Json(session))
}

/// GET /v1/sessions - List sessions in organization
#[utoipa::path(
    get,
    path = "/v1/sessions",
    params(ListSessionsQuery),
    responses(
        (status = 200, description = "Paginated list of sessions", body = PaginatedResponse<Session>),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn list_sessions(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListSessionsQuery>,
) -> Result<Json<PaginatedResponse<Session>>, (StatusCode, Json<ErrorResponse>)> {
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let pagination = Pagination::new(offset, limit);

    // Resolve agent public_id to internal UUID if filtering by agent
    let agent_internal_id = if let Some(ref agent_id) = query.agent_id {
        let row = state
            .db
            .get_agent_by_public_id(org.org_id, &agent_id.to_string())
            .await
            .log_internal_error_json("resolve agent for filter")?;
        row.map(|r| r.id.uuid())
    } else {
        None
    };

    let (sessions, total) = state
        .session_service
        .list(
            org.org_id,
            &org.public_id,
            agent_internal_id,
            org.user_id,
            query.search.as_deref(),
            pagination,
        )
        .await
        .log_internal_error_json("list sessions")?;

    Ok(Json(PaginatedResponse::new(sessions, total, offset, limit)))
}

/// GET /v1/sessions/stats - Get session counts by status
#[utoipa::path(
    get,
    path = "/v1/sessions/stats",
    responses(
        (status = 200, description = "Session statistics", body = SessionStatsResponse),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session_stats(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<SessionStatsResponse>, StatusCode> {
    let stats = state
        .session_service
        .stats(org.org_id)
        .await
        .log_internal_error("get session stats")?;

    Ok(Json(SessionStatsResponse {
        total: stats.total,
        active: stats.active,
        idle: stats.idle,
        started: stats.started,
        waiting_for_tool_results: stats.waiting_for_tool_results,
    }))
}

/// GET /v1/sessions/{session_id} - Get session
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Session found", body = Session),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Session>, (StatusCode, Json<ErrorResponse>)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    let session = state
        .session_service
        .get(org.org_id, &org.public_id, session_id.uuid(), org.user_id)
        .await
        .log_internal_error_json("get session")?
        .ok_or_not_found_json("Session")?;

    Ok(Json(session))
}

/// PATCH /v1/sessions/{session_id} - Update session
#[utoipa::path(
    patch,
    path = "/v1/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    request_body = UpdateSessionRequest,
    responses(
        (status = 200, description = "Session updated successfully", body = Session),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn update_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<UpdateSessionRequest>,
) -> Result<Json<Session>, (StatusCode, Json<ErrorResponse>)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    let session = state
        .session_service
        .update(org.org_id, &org.public_id, session_id.uuid(), req)
        .await
        .log_internal_error_json("update session")?
        .ok_or_not_found_json("Session")?;

    Ok(Json(session))
}

/// DELETE /v1/sessions/{session_id} - Delete session
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 204, description = "Session deleted successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn delete_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    let deleted = state
        .session_service
        .delete(org.org_id, session_id.uuid())
        .await
        .log_internal_error_json("delete session")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Session not found".to_string(),
            }),
        ))
    }
}

/// PUT /v1/sessions/{session_id}/pin - Pin session for current user
#[utoipa::path(
    put,
    path = "/v1/sessions/{session_id}/pin",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 204, description = "Session pinned successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn pin_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let user_id = org.user_id.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Authentication required to pin sessions".to_string(),
            }),
        )
    })?;
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    // Verify session exists in this org
    state
        .session_service
        .get(org.org_id, &org.public_id, session_id.uuid(), None)
        .await
        .log_internal_error_json("get session for pin")?
        .ok_or_not_found_json("Session")?;

    state
        .session_service
        .pin(user_id, session_id.uuid(), org.org_id)
        .await
        .log_internal_error_json("pin session")?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /v1/sessions/{session_id}/pin - Unpin session for current user
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/pin",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 204, description = "Session unpinned successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 401, description = "Authentication required"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn unpin_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let user_id = org.user_id.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponse {
                error: "Authentication required to unpin sessions".to_string(),
            }),
        )
    })?;
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    state
        .session_service
        .unpin(user_id, session_id.uuid())
        .await
        .log_internal_error_json("unpin session")?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/sessions/{session_id}/cancel - Cancel current turn
///
/// Cancels the currently running turn in the session. If no turn is running,
/// this is a no-op and returns success (idempotent). When a turn is active:
/// 1. Cancel the underlying workflow execution
/// 2. Emit a turn.cancelled event
/// 3. Insert an agent message indicating the turn was cancelled
/// 4. Set the session status back to idle
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/cancel",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Turn cancelled successfully (or no-op if idle)", body = CancelTurnResponse),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn cancel_turn(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<CancelTurnResponse>, StatusCode> {
    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;

    // Verify session exists
    let session = state
        .session_service
        .get(org.org_id, &org.public_id, session_id.uuid(), None)
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
    let turn_id = TurnId::from_uuid(session_id.uuid());
    let input_message_id = MessageId::new(); // Placeholder since we don't have the original

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
        InputMessageData::new(user_cancel_message),
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

    const TEST_HARNESS_ID: &str = "harness_550e8400e29b41d4a716446655440000";
    const TEST_AGENT_ID: &str = "agent_550e8400e29b41d4a716446655440000";

    #[test]
    fn test_create_session_request_minimal() {
        // Test with only required harness_id
        let json = format!(r#"{{"harness_id": "{}"}}"#, TEST_HARNESS_ID);
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.harness_id.to_string(), TEST_HARNESS_ID);
        assert_eq!(req.agent_id, None);
        assert_eq!(req.title, None);
        assert!(req.tags.is_empty());
        assert_eq!(req.model_id, None);
        assert!(req.capabilities.is_empty());
    }

    #[test]
    fn test_create_session_request_missing_harness_id() {
        // harness_id is required, so this should fail
        let json = r#"{}"#;
        let result: Result<CreateSessionRequest, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_create_session_request_with_agent_id() {
        // agent_id is optional
        let json = format!(
            r#"{{"harness_id": "{}", "agent_id": "{}"}}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        let expected_agent_id: AgentId = TEST_AGENT_ID.parse().unwrap();
        assert_eq!(req.agent_id, Some(expected_agent_id));
    }

    #[test]
    fn test_create_session_request_with_title() {
        let json = format!(
            r#"{{"harness_id": "{}", "agent_id": "{}", "title": "Test Session"}}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.title, Some("Test Session".to_string()));
        assert!(req.tags.is_empty());
        assert_eq!(req.model_id, None);
    }

    #[test]
    fn test_create_session_request_with_model_id() {
        let model_id: ModelId = "model_550e8400e29b41d4a716446655440000".parse().unwrap();
        let json = format!(
            r#"{{"harness_id": "{}", "agent_id": "{}", "model_id": "{}"}}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID, model_id
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.model_id, Some(model_id));
    }

    #[test]
    fn test_create_session_request_full() {
        let model_id: ModelId = "model_550e8400e29b41d4a716446655440001".parse().unwrap();
        let json = format!(
            r#"{{"harness_id": "{}", "agent_id": "{}", "title": "Full Session", "tags": ["tag1", "tag2"], "model_id": "{}"}}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID, model_id
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.title, Some("Full Session".to_string()));
        assert_eq!(req.tags, vec!["tag1", "tag2"]);
        assert_eq!(req.model_id, Some(model_id));
        assert!(req.capabilities.is_empty());
    }

    #[test]
    fn test_create_session_request_with_capabilities() {
        let json = format!(
            r#"{{
                "harness_id": "{}",
                "agent_id": "{}",
                "capabilities": [
                    {{"ref": "current_time"}},
                    {{"ref": "web_fetch", "config": {{"timeout_ms": 30000}}}}
                ]
            }}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.capabilities[0].capability_id(), "current_time");
        assert_eq!(req.capabilities[1].capability_id(), "web_fetch");
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
