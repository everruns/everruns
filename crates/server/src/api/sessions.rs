// Session CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
// Policy enforcement happens at the service layer via #[policy] macro.

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::session::{SESSION_MANAGE, SESSION_VIEW};
use crate::services::{EventService, SessionService};
use crate::storage::StorageBackend;
use anyhow::Context;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::capability_types::AgentCapabilityConfig;
use everruns_core::events::{EventContext, EventRequest, InputMessageData, TurnCancelledData};
use everruns_core::typed_id::{
    AgentId, AgentIdentityId, HarnessId, MessageId, ModelId, SessionId, TurnId,
};
use everruns_core::{
    BuiltInHarnessRole, Caller, Message, PlatformDefinition, ResourceConfigResponse, Session,
    evaluate_policies_with,
};
use everruns_worker::AgentRunner;

use super::common::{
    ApiOptionExt, ApiPolicyResultExt, ApiResult, ApiResultExt, ErrorResponse, PaginatedResponse,
    Pagination, deserialize_nullable_update_field, impl_auth_state,
};
use super::validation::{self, normalize_locale};
use everruns_durable::UpdateField;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Request to create a session
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// ID of the harness for this session (format: harness_{32-hex}).
    /// If omitted, the org default harness is used. New orgs default that to Generic.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: Option<HarnessId>,
    /// ID of the agent to work in this session (optional, format: agent_{32-hex}).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// Optional resident agent identity used for unattended/background execution.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001")]
    pub agent_identity_id: Option<AgentIdentityId>,
    /// Human-readable title for the session.
    #[serde(default)]
    #[schema(example = "Debug login issue")]
    pub title: Option<String>,
    /// Session locale (BCP 47, e.g. `uk-UA`).
    #[serde(default)]
    #[schema(example = "uk-UA")]
    pub locale: Option<String>,
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
    /// Optional session-level system prompt override.
    /// Prepended to the agent's system prompt when building RuntimeAgent.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Session-level initial files (additive to agent initial_files).
    /// Files with matching paths override agent/harness files; new paths are appended.
    #[serde(default)]
    pub initial_files: Vec<everruns_core::InitialFile>,
    /// Session-level client hints — arbitrary key-value pairs that tell the
    /// server what the client can handle. These are defaults for every turn;
    /// per-message `controls.hints` override these key-by-key (shallow merge).
    ///
    /// Examples: `{"setup_connection": true, "rich_media": true}`
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub hints: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Maximum number of LLM iterations per turn for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
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
    /// Optional resident agent identity used for unattended/background execution.
    #[serde(default, deserialize_with = "deserialize_nullable_update_field")]
    #[schema(
        value_type = Option<String>,
        example = "identity_01933b5a00007000800000000000001",
        nullable = true
    )]
    pub agent_identity_id: UpdateField<AgentIdentityId>,
    /// Session locale (BCP 47, e.g. `uk-UA`).
    #[serde(default)]
    #[schema(example = "uk-UA")]
    pub locale: Option<String>,
    /// Tags for organizing and filtering sessions.
    #[serde(default)]
    #[schema(example = json!(["resolved"]))]
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GetOrCreateChatSessionRequest {
    /// Browser locale for seeding the global chat session (BCP 47, e.g. `uk-UA`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
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
    pub fallback_default_harness_name: Option<String>,
    pub chat_harness_name: Option<String>,
    pub chat_session_title: Option<String>,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, runner: Arc<dyn AgentRunner>, auth: AuthState) -> Self {
        Self::with_platform_definition(
            db,
            runner,
            auth,
            &crate::platform::oss_platform_definition(),
            crate::event_delivery::EventDelivery::in_memory(),
        )
    }

    pub fn with_platform_definition(
        db: Arc<StorageBackend>,
        runner: Arc<dyn AgentRunner>,
        auth: AuthState,
        platform_definition: &PlatformDefinition,
        event_delivery: crate::event_delivery::EventDelivery,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::with_registry(
                db.clone(),
                platform_definition.capability_registry().clone(),
            )),
            event_service: EventService::new(db.clone(), event_delivery.clone()),
            db,
            runner,
            auth,
            fallback_default_harness_name: platform_definition
                .harness_for_role(BuiltInHarnessRole::Default)
                .map(|h| h.name.clone()),
            chat_harness_name: platform_definition
                .harness_for_role(BuiltInHarnessRole::Chat)
                .map(|h| h.name.clone()),
            chat_session_title: platform_definition
                .harness_for_role(BuiltInHarnessRole::Chat)
                .map(|h| h.name.clone()),
        }
    }
}

impl_auth_state!(AppState);

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
        // Config endpoint (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/config", get(session_config))
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

/// GET /v1/sessions/config
pub async fn session_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&SESSION_VIEW, &SESSION_MANAGE],
    );
    Json(ResourceConfigResponse { policies })
}

/// POST /v1/sessions - Create a new session
#[utoipa::path(
    post,
    path = "/v1/sessions",
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "Session created successfully", body = Session),
        (status = 404, description = "Harness, Agent, or Model not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn create_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(mut req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<Session>), (StatusCode, Json<ErrorResponse>)> {
    req.locale = normalize_locale(req.locale)
        .map_err(|err| -> (StatusCode, Json<ErrorResponse>) { err.into() })?;

    let harness_id = resolve_session_harness_id(
        &state.db,
        org.org_id,
        req.harness_id,
        state.fallback_default_harness_name.as_deref(),
    )
    .await
    .log_internal_error_json("resolve session harness fallback")?;
    req.harness_id = Some(harness_id);

    // Validate harness exists and belongs to this org
    state
        .db
        .get_harness(org.org_id, harness_id)
        .await
        .log_internal_error_json("resolve harness")?
        .ok_or_not_found_json("Harness")?;

    // Validate model exists and belongs to this org (if specified)
    if let Some(ref model_id) = req.model_id {
        state
            .db
            .get_llm_model(org.org_id, model_id.uuid())
            .await
            .log_internal_error_json("resolve model")?
            .ok_or_not_found_json("Model")?;
    }

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

    // Validate session-level system_prompt and initial_files
    if let Some(ref prompt) = req.system_prompt {
        validation::validate_agent_system_prompt(prompt)
            .map_err(|err| -> (StatusCode, Json<ErrorResponse>) { err.into() })?;
    }
    if !req.initial_files.is_empty() {
        validation::validate_initial_files(&req.initial_files)
            .map_err(|err| -> (StatusCode, Json<ErrorResponse>) { err.into() })?;
    }

    let caller = Caller::from(&org);
    let session = state
        .session_service
        .create(
            &caller,
            harness_id.uuid(),
            agent_internal_id,
            agent_public_id,
            req,
        )
        .await
        .map_policy_or_internal("create session")?;

    Ok((StatusCode::CREATED, Json(session)))
}

async fn resolve_session_harness_id(
    db: &StorageBackend,
    org_id: i64,
    requested_harness_id: Option<HarnessId>,
    fallback_default_harness_name: Option<&str>,
) -> anyhow::Result<HarnessId> {
    if let Some(harness_id) = requested_harness_id {
        return Ok(harness_id);
    }

    let settings = db.get_organization_settings(org_id).await?;
    if let Some(harness_id) = settings.and_then(|row| row.default_harness_id) {
        return Ok(harness_id);
    }

    let fallback_name = fallback_default_harness_name.context(
        "Session creation requires a default harness but no default harness role is configured",
    )?;
    let harnesses = db
        .list_harnesses(org_id, Some(fallback_name), false)
        .await?;
    harnesses
        .into_iter()
        .find(|h| h.is_built_in && h.name == fallback_name)
        .map(|h| h.id)
        .context(format!(
            "Built-in default harness '{fallback_name}' is not provisioned for org {org_id}"
        ))
}

/// POST /v1/sessions/chat - Get or create global chat session
///
/// Returns the user's singleton global chat session. Creates one if it doesn't exist.
/// Uses the Platform Chat harness and tags for per-user singleton management.
#[utoipa::path(
    post,
    path = "/v1/sessions/chat",
    request_body = Option<GetOrCreateChatSessionRequest>,
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
    payload: Option<Json<GetOrCreateChatSessionRequest>>,
) -> ApiResult<Session> {
    let locale = normalize_locale(payload.and_then(|Json(body)| body.locale))
        .map_err(|err| -> (StatusCode, Json<ErrorResponse>) { err.into() })?;

    // Use authenticated user_id, or fall back to anonymous user (auth=none mode)
    let user_id = org.user_id.unwrap_or(everruns_core::ANONYMOUS_USER_ID);
    let chat_harness_name = state.chat_harness_name.clone().ok_or((
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::new(
            "Global chat is not configured for this platform",
        )),
    ))?;
    let chat_harness_id =
        resolve_named_built_in_harness_id(&state.db, org.org_id, &chat_harness_name)
            .await
            .log_internal_error_json("resolve chat harness")?;

    let caller = Caller::from(&org);
    let session = state
        .session_service
        .get_or_create_chat_session(
            &caller,
            user_id,
            chat_harness_id.uuid(),
            state
                .chat_session_title
                .as_deref()
                .unwrap_or(chat_harness_name.as_str()),
            locale,
        )
        .await
        .map_policy_or_internal("get or create chat session")?;

    Ok(Json(session))
}

async fn resolve_named_built_in_harness_id(
    db: &StorageBackend,
    org_id: i64,
    harness_name: &str,
) -> anyhow::Result<HarnessId> {
    let harnesses = db.list_harnesses(org_id, Some(harness_name), false).await?;
    harnesses
        .into_iter()
        .find(|h| h.is_built_in && h.name == harness_name)
        .map(|h| h.id)
        .context(format!(
            "Built-in harness '{harness_name}' is not provisioned for org {org_id}"
        ))
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
) -> ApiResult<PaginatedResponse<Session>> {
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let pagination = Pagination::new(offset, limit);

    // Resolve agent public_id to internal UUID if filtering by agent.
    // If agent_id is provided but not found, return empty results rather than
    // silently dropping the filter and returning unfiltered sessions.
    let agent_internal_id = if let Some(ref agent_id) = query.agent_id {
        let row = state
            .db
            .get_agent_by_public_id(org.org_id, &agent_id.to_string())
            .await
            .log_internal_error_json("resolve agent for filter")?;
        match row {
            Some(r) => Some(r.id.uuid()),
            None => {
                return Ok(Json(PaginatedResponse::new(vec![], 0, offset, limit)));
            }
        }
    } else {
        None
    };

    let caller = Caller::from(&org);
    let (sessions, total) = state
        .session_service
        .list(
            &caller,
            agent_internal_id,
            org.user_id,
            query.search.as_deref(),
            pagination,
        )
        .await
        .map_policy_or_internal("list sessions")?;

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
) -> ApiResult<SessionStatsResponse> {
    let caller = Caller::from(&org);
    let stats = state
        .session_service
        .stats(&caller)
        .await
        .map_policy_or_internal("get session stats")?;

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
) -> ApiResult<Session> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), org.user_id)
        .await
        .map_policy_or_internal("get session")?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Session not found".to_string(),
                }),
            )
        })?;

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
    Json(mut req): Json<UpdateSessionRequest>,
) -> ApiResult<Session> {
    req.locale = normalize_locale(req.locale)
        .map_err(|err| -> (StatusCode, Json<ErrorResponse>) { err.into() })?;

    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let session = state
        .session_service
        .update(&caller, session_id.uuid(), req)
        .await
        .map_policy_or_internal("update session")?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Session not found".to_string(),
                }),
            )
        })?;

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

    let caller = Caller::from(&org);
    state
        .session_service
        .delete(&caller, session_id.uuid())
        .await
        .map_policy_or_internal("delete session")?;

    Ok(StatusCode::NO_CONTENT)
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

    let caller = Caller::from(&org);
    state
        .session_service
        .pin(&caller, user_id, session_id.uuid())
        .await
        .map_policy_or_internal("pin session")?;

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

    let caller = Caller::from(&org);
    state
        .session_service
        .unpin(&caller, user_id, session_id.uuid())
        .await
        .map_policy_or_internal("unpin session")?;

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
) -> ApiResult<CancelTurnResponse> {
    let session_id: SessionId = session_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid session ID: {}", e),
            }),
        )
    })?;

    // Verify session exists
    let caller = Caller::from(&org);
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .map_policy_or_internal("get session for cancel")?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Session not found".to_string(),
                }),
            )
        })?;

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
    use crate::storage::{
        StorageBackend,
        models::{CreateHarnessRow, UpdateOrganizationSettings},
    };
    use everruns_durable::UpdateField;

    const TEST_HARNESS_ID: &str = "harness_550e8400e29b41d4a716446655440000";
    const TEST_AGENT_ID: &str = "agent_550e8400e29b41d4a716446655440000";

    #[test]
    fn test_create_session_request_minimal() {
        let json = format!(r#"{{"harness_id": "{}"}}"#, TEST_HARNESS_ID);
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.harness_id.unwrap().to_string(), TEST_HARNESS_ID);
        assert_eq!(req.agent_id, None);
        assert_eq!(req.title, None);
        assert_eq!(req.locale, None);
        assert!(req.tags.is_empty());
        assert_eq!(req.model_id, None);
        assert!(req.capabilities.is_empty());
    }

    #[test]
    fn test_create_session_request_missing_harness_id_is_none() {
        let json = r#"{}"#;
        let req: CreateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.harness_id, None);
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
            r#"{{"harness_id": "{}", "agent_id": "{}", "title": "Test Session", "locale": "uk-UA"}}"#,
            TEST_HARNESS_ID, TEST_AGENT_ID
        );
        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.title, Some("Test Session".to_string()));
        assert_eq!(req.locale.as_deref(), Some("uk-UA"));
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
        assert_eq!(req.agent_identity_id, UpdateField::Unchanged);
        assert_eq!(req.locale, None);
        assert_eq!(req.tags, None);
    }

    #[test]
    fn test_update_session_request_clears_agent_identity_when_null() {
        let json = r#"{"agent_identity_id":null}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.agent_identity_id, UpdateField::Clear);
    }

    #[test]
    fn test_update_session_request_sets_agent_identity_when_present() {
        let json = r#"{"agent_identity_id":"identity_550e8400e29b41d4a716446655440000"}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert!(matches!(req.agent_identity_id, UpdateField::Set(_)));
    }

    #[test]
    fn test_update_session_request_with_title() {
        let json = r#"{"title": "Updated Title", "locale": "en-US"}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, Some("Updated Title".to_string()));
        assert_eq!(req.locale.as_deref(), Some("en-US"));
        assert_eq!(req.tags, None);
    }

    #[test]
    fn test_update_session_request_with_tags() {
        let json = r#"{"tags": ["new-tag"]}"#;
        let req: UpdateSessionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.title, None);
        assert_eq!(req.tags, Some(vec!["new-tag".to_string()]));
    }

    #[tokio::test]
    async fn test_resolve_session_harness_id_defaults_to_generic() {
        let db = StorageBackend::in_memory();
        let row = db
            .create_harness(
                42,
                CreateHarnessRow {
                    name: "Generic".to_string(),
                    description: Some("Generic".to_string()),
                    system_prompt: "You are helpful.".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec!["generic".to_string()],
                    initial_files: serde_json::json!([]),
                    is_built_in: true,
                },
            )
            .await
            .unwrap();

        let harness_id = resolve_session_harness_id(&db, 42, None, Some("Generic"))
            .await
            .unwrap();
        assert_eq!(harness_id, row.id);
    }

    #[tokio::test]
    async fn test_resolve_session_harness_id_uses_org_default_harness() {
        let db = StorageBackend::in_memory();
        let default_harness_id: HarnessId = TEST_HARNESS_ID.parse().unwrap();

        db.patch_organization_settings(
            42,
            UpdateOrganizationSettings {
                default_harness_id: UpdateField::Set(default_harness_id),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let harness_id = resolve_session_harness_id(&db, 42, None, Some("Generic"))
            .await
            .unwrap();
        assert_eq!(harness_id, default_harness_id);
    }
}
