// Session CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
// Policy enforcement happens at the service layer via #[policy] macro.

use crate::auth::{AuthState, ResolvedOrg, rate_limit::OrgRateLimiter};
use crate::domains::common::{Command, Ctx};
use crate::domains::sessions::{
    CancelSession, CreateSession, DeleteSession, GetOrCreateChatSession, GetSession,
    GetSessionContextReport, GetSessionStats, ListSessions, PinSession, SESSION_MANAGE,
    SESSION_VIEW, SessionService, UnpinSession, UpdateSessionCmd,
};
use crate::services::EventService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::capability_types::AgentCapabilityConfig;
use everruns_core::typed_id::{AgentId, AgentIdentityId, HarnessId, ModelId, SessionId};
use everruns_core::{
    BuiltInHarnessRole, Caller, PlatformDefinition, ResourceConfigResponse, ScopedMcpServers,
    Session, SessionContextReport, ToolDefinition, evaluate_policies_with,
};
use everruns_worker::AgentRunner;

use super::common::{
    ApiResult, ErrorResponse, PaginatedResponse, UrlBuilder, WithUrls,
    deserialize_nullable_update_field, impl_auth_state,
};
use everruns_durable::UpdateField;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Request to create a session
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSessionRequest {
    /// ID of the harness for this session (format: harness_{32-hex}).
    /// If omitted, the org default harness is used. New orgs default that to Generic.
    /// Mutually exclusive with `harness_name`.
    #[serde(default)]
    #[schema(value_type = Option<String>, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: Option<HarnessId>,
    /// Harness name (e.g. "generic", "deep-research").
    /// Alternative to `harness_id` — looked up by name within the org.
    /// Mutually exclusive with `harness_id`.
    #[serde(default)]
    #[schema(example = "generic")]
    pub harness_name: Option<String>,
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
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Client-side tools for this session (additive to agent tools).
    /// These tools are sent to the LLM but executed by the client.
    #[serde(default, deserialize_with = "deserialize_client_side_tools")]
    #[schema(example = json!([{"type": "client_side", "name": "open_url", "description": "Open URL in the user's browser", "parameters": {"type": "object", "properties": {"url": {"type": "string"}}, "required": ["url"]}}]))]
    pub tools: Vec<ToolDefinition>,
    /// Remote MCP servers scoped to this session only.
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: ScopedMcpServers,
    /// Optional session-level system prompt override.
    /// Prepended to the agent's system prompt when building RuntimeAgent.
    #[serde(default)]
    #[schema(
        example = "You are debugging a production incident. Be concise and cite log lines verbatim."
    )]
    pub system_prompt: Option<String>,
    /// Session-level initial files (additive to agent initial_files).
    /// Files with matching paths override agent/harness files; new paths are appended.
    #[serde(default)]
    #[schema(example = json!([{"path": "README.md", "content": "# Project notes\n"}]))]
    pub initial_files: Vec<everruns_core::InitialFile>,
    /// Session-level client hints — arbitrary key-value pairs that tell the
    /// server what the client can handle. These are defaults for every turn;
    /// per-message `controls.hints` override these key-by-key (shallow merge).
    #[serde(default)]
    #[schema(example = json!({"setup_connection": true, "rich_media": true}))]
    pub hints: Option<std::collections::HashMap<String, serde_json::Value>>,
    /// Network access list controlling which hosts/URLs this session can reach.
    /// Merged with harness and agent layers (allowed: intersect, blocked: union).
    /// Example shape is defined on `NetworkAccessList`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = 20)]
    pub max_iterations: Option<usize>,
    /// Internal: parent session for subagent nesting guard.
    /// Set by the worker when spawning a child session so the child cannot itself
    /// spawn further children (prevents runaway recursion).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(ignore)]
    pub parent_session_id: Option<SessionId>,
}

// Trust boundary (client-side tools deprecation rollout): the `tools` field
// on session/agent create/update requests is documented as carrying only
// `client_side` definitions executed by the client, not the server. Pre-#1525
// the server silently accepted other shapes; #1525 turned the invariant into
// a hard 400. To give SDK/CLI consumers a migration window, we now drop any
// non-`client_side` entries during deserialization (with a `tracing::warn!`
// for ops visibility) instead of failing the request. Operators flip
// `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS=1` to opt back into hard-rejection
// once their clients are confirmed migrated. The migration timeline lives in
// `specs/client-side-tools.md`.
fn deserialize_client_side_tools<'de, D>(deserializer: D) -> Result<Vec<ToolDefinition>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let tools = Vec::<ToolDefinition>::deserialize(deserializer)?;
    filter_or_reject_client_side_tools(tools).map_err(serde::de::Error::custom)
}

/// Apply the deprecation policy to a parsed `tools` array. By default, drops
/// every non-`client_side` entry with a structured warning. When the env
/// var `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS` is set to a truthy value
/// (`1`/`true`/`yes`), returns the original hard-reject error instead.
pub(crate) fn filter_or_reject_client_side_tools(
    tools: Vec<ToolDefinition>,
) -> Result<Vec<ToolDefinition>, &'static str> {
    let (kept, dropped): (Vec<_>, Vec<_>) = tools
        .into_iter()
        .partition(|tool| matches!(tool, ToolDefinition::ClientSide(_)));
    if dropped.is_empty() {
        return Ok(kept);
    }
    if reject_non_client_side_tools_enabled() {
        return Err("tools must contain only client_side definitions");
    }
    // Soft-warn surface for the deprecation window: log type names only, no
    // payloads, so request fields like prompts or arguments cannot leak into
    // logs from the request body. Dedupe + sort the kind labels so a request
    // with many non-`client_side` entries doesn't allocate a huge Vec or
    // amplify telemetry — the count carries the cardinality, not the labels.
    let mut dropped_kinds: Vec<&'static str> =
        dropped.iter().map(tool_definition_kind_name).collect();
    dropped_kinds.sort_unstable();
    dropped_kinds.dedup();
    tracing::warn!(
        target = "client_tools_deprecation",
        dropped_count = dropped.len(),
        dropped_kinds = ?dropped_kinds,
        "tools[] contained {} non-client_side definition(s); dropping them. \
         The server will reject these in a future release; migrate clients \
         now. See specs/client-side-tools.md for the timeline.",
        dropped.len()
    );
    Ok(kept)
}

/// Return `true` when the operator has opted into the legacy hard-reject
/// behavior. Used to gate the deprecation window.
pub(crate) fn reject_non_client_side_tools_enabled() -> bool {
    matches!(
        std::env::var("EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS")
            .ok()
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Log-safe kind label for a `ToolDefinition` variant. Returns the same
/// `snake_case` discriminator that serde uses on the wire, so dashboards can
/// pivot on `dropped_kinds` without parsing free-form text.
fn tool_definition_kind_name(tool: &ToolDefinition) -> &'static str {
    match tool {
        ToolDefinition::Builtin(_) => "builtin",
        ToolDefinition::ClientSide(_) => "client_side",
    }
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

/// Request body for the `get_or_create_chat_session` operation.
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
    pub org_rate_limiter: OrgRateLimiter,
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
                .map(|h| h.display_name.clone()),
            org_rate_limiter: OrgRateLimiter::default(),
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_session_service(self.session_service.clone())
        .with_event_service(Arc::new(self.event_service.clone()))
        .with_runner(self.runner.clone())
        .with_fallback_harness_name(self.fallback_default_harness_name.clone())
        .with_chat_harness_name(self.chat_harness_name.clone())
        .with_chat_session_title(self.chat_session_title.clone())
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
        .route(
            "/v1/sessions/{session_id}/context-report",
            get(get_session_context_report),
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
    request_body(
        content = CreateSessionRequest,
        example = json!({
            "harness_name": "generic",
            "title": "Debug login issue",
            "tags": ["debugging", "urgent"]
        })
    ),
    responses(
        (
            status = 201,
            description = "Session created successfully",
            body = WithUrls<Session>,
            example = json!({
                "self_url": "https://app.everruns.com/api/v1/sessions/session_01933b5a00007000800000000000001",
                "view_url": "https://app.everruns.com/sessions/session_01933b5a00007000800000000000001/chat",
                "ui_link":  "https://app.everruns.com/sessions/session_01933b5a00007000800000000000001/chat",
                "id": "session_01933b5a00007000800000000000001",
                "status": "started",
                "organization_id": "org_00000000000000000000000000000001",
                "harness_id": "harness_01933b5a00007000800000000000001",
                "owner_principal_id": "principal_01933b5a000070008000000000000001",
                "title": "Debug login issue",
                "tags": ["debugging", "urgent"],
                "created_at": "2026-05-27T15:24:00Z",
                "updated_at": "2026-05-27T15:24:00Z"
            })
        ),
        (
            status = 404,
            description = "Harness, Agent, or Model not found",
            body = ErrorResponse,
            example = json!({
                "type": "https://docs.everruns.com/errors/harness_not_found",
                "title": "Not Found",
                "status": 404,
                "detail": "Harness 'generic' not found in org org_00000000000000000000000000000001.",
                "code": "harness_not_found"
            })
        ),
        (status = 500, description = "Internal server error")
    ),
    extensions(
        ("x-cost-tier" = json!("paid")),
    ),
    tag = "sessions"
)]
pub async fn create_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<WithUrls<Session>>), (StatusCode, Json<ErrorResponse>)> {
    if state
        .org_rate_limiter
        .check_session_create(org.org_id)
        .await
        .is_err()
    {
        return Err(
            ErrorResponse::new("Too many requests. Please try again later.")
                .with_code("rate_limited")
                .with_retry_after(60)
                .into_response(StatusCode::TOO_MANY_REQUESTS),
        );
    }
    // `parent_session_id` is internal-only: the subagent/handoff spawn flow
    // sets it via the platform store (DirectPlatformStore::create_session
    // dispatches the command directly). External HTTP callers must never set
    // it — that would let a client forge cross-session/cross-org parent links —
    // so strip any client-supplied value at the public boundary.
    let mut req = req;
    req.parent_session_id = None;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = CreateSession(req).run(&state.ctx(&org)).await?;

    Ok((StatusCode::CREATED, Json(urls.wrap(session))))
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
        (status = 200, description = "Chat session returned", body = WithUrls<Session>),
        (status = 401, description = "Authentication required"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_or_create_chat_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    payload: Option<Json<GetOrCreateChatSessionRequest>>,
) -> ApiResult<WithUrls<Session>> {
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = GetOrCreateChatSession {
        locale: payload.and_then(|Json(body)| body.locale),
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(urls.wrap(session)))
}

/// GET /v1/sessions - List sessions in organization
#[utoipa::path(
    get,
    path = "/v1/sessions",
    params(ListSessionsQuery),
    responses(
        (status = 200, description = "Paginated list of sessions", body = PaginatedResponse<WithUrls<Session>>),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn list_sessions(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListSessionsQuery>,
) -> ApiResult<PaginatedResponse<WithUrls<Session>>> {
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let page = ListSessions {
        agent_id: query.agent_id,
        search: query.search,
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(
        PaginatedResponse::new(page.data, page.total, page.offset, page.limit).with_urls(&urls),
    ))
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
    Ok(Json(GetSessionStats.run(&state.ctx(&org)).await?))
}

/// GET /v1/sessions/{session_id} - Get session
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Session found", body = WithUrls<Session>),
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
) -> ApiResult<WithUrls<Session>> {
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = GetSession { session_id }.run(&state.ctx(&org)).await?;

    Ok(Json(urls.wrap(session)))
}

/// GET /v1/sessions/{session_id}/context-report - Latest context breakdown
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/context-report",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Session context report", body = SessionContextReport),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session_context_report(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionContextReport> {
    Ok(Json(
        GetSessionContextReport { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
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
        (status = 200, description = "Session updated successfully", body = WithUrls<Session>),
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
) -> ApiResult<WithUrls<Session>> {
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = UpdateSessionCmd { session_id, req }
        .run(&state.ctx(&org))
        .await?;

    Ok(Json(urls.wrap(session)))
}

/// DELETE /v1/sessions/{session_id} - Delete session
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    extensions(
        ("x-side-effect" = json!("reversible")),
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
    DeleteSession { session_id }.run(&state.ctx(&org)).await?;

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
    if org.user_id.is_none() {
        return Err(
            ErrorResponse::new("Authentication required to pin sessions".to_string())
                .into_response(StatusCode::UNAUTHORIZED),
        );
    }
    PinSession { session_id }.run(&state.ctx(&org)).await?;

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
    if org.user_id.is_none() {
        return Err(
            ErrorResponse::new("Authentication required to unpin sessions".to_string())
                .into_response(StatusCode::UNAUTHORIZED),
        );
    }
    UnpinSession { session_id }.run(&state.ctx(&org)).await?;

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
        (
            status = 200,
            description = "Turn cancelled, or no-op (status: no_op) if no turn was running",
            body = CancelTurnResponse,
            example = json!({
                "status": "cancelled",
                "message": "Turn cancelled successfully"
            })
        ),
        (
            status = 404,
            description = "Session not found",
            body = ErrorResponse,
            example = json!({
                "type": "https://docs.everruns.com/errors/session_not_found",
                "title": "Not Found",
                "status": 404,
                "detail": "Session session_01933b5a000070008000000000000001 not found in org org_00000000000000000000000000000001.",
                "code": "session_not_found"
            })
        ),
        (status = 400, description = "Invalid session ID"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn cancel_turn(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<CancelTurnResponse> {
    Ok(Json(
        CancelSession { session_id }.run(&state.ctx(&org)).await?,
    ))
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
    fn test_create_session_request_drops_builtin_tools_with_warn() {
        // Deprecation window: legacy `builtin` entries are dropped, not
        // rejected, so existing SDK/CLI clients keep working while operators
        // monitor the migration.
        let json = format!(
            r#"{{
                "harness_id": "{}",
                "tools": [
                    {{
                        "type": "builtin",
                        "name": "read_file",
                        "description": "Read file",
                        "parameters": {{"type": "object"}}
                    }},
                    {{
                        "type": "client_side",
                        "name": "lookup_crm",
                        "description": "Lookup",
                        "parameters": {{"type": "object"}}
                    }}
                ]
            }}"#,
            TEST_HARNESS_ID
        );

        let req: CreateSessionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req.tools.len(), 1);
        // Only the client_side entry survives.
        assert!(matches!(
            req.tools[0],
            everruns_core::ToolDefinition::ClientSide(_)
        ));
    }

    // Strict-mode tests live in `tests/strict_client_tools.rs` so they run in
    // their own process and don't race with the lenient round-trip tests
    // here over the `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS` env var.

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
                    name: "generic".to_string(),
                    display_name: Some("Generic".to_string()),
                    description: Some("Generic".to_string()),
                    system_prompt: "You are helpful.".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec!["generic".to_string()],
                    initial_files: serde_json::json!([]),
                    mcp_servers: serde_json::json!({}),
                    is_built_in: true,
                    network_access: None,
                },
            )
            .await
            .unwrap();

        let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
            &db,
            42,
            None,
            Some("generic"),
        )
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

        let harness_id = crate::domains::sessions::queries::resolve_session_harness_id(
            &db,
            42,
            None,
            Some("generic"),
        )
        .await
        .unwrap();
        assert_eq!(harness_id, default_harness_id);
    }
}
