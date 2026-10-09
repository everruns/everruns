// Session CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
// Policy enforcement happens at the service layer via #[policy] macro.
use super::common::{
    ApiResult, ApiResultExt, ErrorResponse, PaginatedResponse, UrlBuilder, WithUrls,
    impl_auth_state,
};
use crate::auth::{AuthState, ResolvedOrg, rate_limit::OrgRateLimiter};
use crate::domains::common::{Command, Ctx};
use crate::domains::harnesses::record::BuiltInHarnessRole;
use crate::domains::sessions::record::{Session, SessionParticipant};
pub use crate::domains::sessions::types::CreateSessionRequest;
pub(crate) use crate::domains::sessions::types::filter_or_reject_client_side_tools;
pub use crate::domains::sessions::types::{
    AddSessionParticipantRequest, CancelStatus, CancelTurnResponse, ForkSessionRequest,
    ListSessionsQuery, SessionFacetCount, SessionFacetsResponse, SessionStatsResponse,
    UpdateSessionRequest,
};
use crate::domains::sessions::{
    AddSessionParticipant, ArchiveSession, CancelSession, CreateSession, DeleteSession,
    ForkSession, GetSession, GetSessionContextReport, GetSessionFacets, GetSessionStats,
    LeaveSessionParticipant, ListSessionParticipants, ListSessions, PinSession, SESSION_MANAGE,
    SESSION_VIEW, SessionFilterArgs, SessionService, UnarchiveSession, UnpinSession,
    UpdateSessionCmd,
};
use crate::kernel_imports::{
    Caller, ResourceConfigResponse, SessionContextReport, SessionSeedMode, evaluate_policies_with,
};
use crate::services::EventService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_contracts::typed_id::{AgentId, ModelId, VirtualUserId};
use everruns_core::host::HostComposition;
use everruns_core::host::TurnBackend;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Query parameters for the sessions facet rail. Mirrors `ListSessionsQuery`
/// minus pagination, so counts and page always share a predicate.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SessionFacetsQuery {
    /// Exclude the permanent Chat from side-conversation pages.
    #[serde(
        default,
        deserialize_with = "crate::domains::common::deserialize_opt_bool_lenient"
    )]
    pub side_chats_only: Option<bool>,
    /// Filter by the fixed Playground end-user identity.
    #[param(value_type = Option<String>)]
    pub playground_user_id: Option<VirtualUserId>,
    /// Return only archived sessions.
    #[serde(
        default,
        deserialize_with = "crate::domains::common::deserialize_opt_bool_lenient"
    )]
    pub archived_only: Option<bool>,
    #[param(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    pub search: Option<String>,
    pub source: Option<String>,
    pub status: Option<String>,
    pub mine: Option<bool>,
    pub include_archived: Option<bool>,
    pub created_after: Option<String>,
    pub created_before: Option<String>,
    pub order: Option<String>,
}

/// App state for sessions routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub event_service: EventService,
    pub runner: Arc<dyn TurnBackend>,
    pub auth: AuthState,
    pub fallback_default_harness_name: Option<String>,
    pub org_rate_limiter: OrgRateLimiter,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, runner: Arc<dyn TurnBackend>, auth: AuthState) -> Self {
        Self::with_host_composition(
            db,
            runner,
            auth,
            &crate::platform::oss_host_composition(),
            &crate::platform::oss_built_in_harnesses(),
            crate::live_updates::event_delivery::EventDelivery::in_memory(),
        )
    }

    pub fn with_host_composition(
        db: Arc<StorageBackend>,
        runner: Arc<dyn TurnBackend>,
        auth: AuthState,
        host_composition: &HostComposition,
        built_in_harnesses: &[crate::domains::harnesses::record::BuiltInHarnessDefinition],
        event_delivery: crate::live_updates::event_delivery::EventDelivery,
    ) -> Self {
        Self {
            session_service: Arc::new(SessionService::with_registry(
                db.clone(),
                (*host_composition.capability_registry()).clone(),
            )),
            event_service: EventService::new(db.clone(), event_delivery.clone()),
            db,
            runner,
            auth,
            fallback_default_harness_name: crate::domains::harnesses::record::harness_for_role(
                built_in_harnesses,
                BuiltInHarnessRole::Default,
            )
            .map(|h| h.name.clone()),
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
        .with_feature_flags(org.feature_flags.clone())
        .with_session_service(self.session_service.clone())
        .with_event_service(Arc::new(self.event_service.clone()))
        .with_runner(self.runner.clone())
        .with_fallback_harness_name(self.fallback_default_harness_name.clone())
        .with_org_rate_limiter(self.org_rate_limiter.clone())
    }
}

impl_auth_state!(AppState);

/// Create session routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        // Config endpoint (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/config", get(session_config))
        .route("/v1/sessions/platform-chat", post(ensure_platform_chat))
        // Global chat session (must be before /{session_id} to avoid conflict)
        // Session facets (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/facets", get(get_session_facets))
        // Session stats (must be before /{session_id} to avoid conflict)
        .route("/v1/sessions/stats", get(get_session_stats))
        // Session CRUD
        .route("/v1/sessions", post(create_session).get(list_sessions))
        .route(
            "/v1/sessions/{session_id}/participants",
            get(list_session_participants).post(add_session_participant),
        )
        .route(
            "/v1/sessions/{session_id}/participants/{participant_id}",
            axum::routing::delete(leave_session_participant),
        )
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
        .route(
            "/v1/sessions/{session_id}/resolved-model",
            get(get_session_resolved_model),
        )
        // Pin/unpin
        .route(
            "/v1/sessions/{session_id}/pin",
            axum::routing::put(pin_session).delete(unpin_session),
        )
        .route(
            "/v1/sessions/{session_id}/archive",
            axum::routing::put(archive_session).delete(unarchive_session),
        )
        // Cancel turn endpoint
        .route("/v1/sessions/{session_id}/cancel", post(cancel_turn))
        // Fork a session into an independent copy
        .route("/v1/sessions/{session_id}/fork", post(fork_session))
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
    tag = "sessions"
)]
pub async fn create_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<WithUrls<Session>>), (StatusCode, Json<ErrorResponse>)> {
    let mut req = req;
    strip_internal_only_fields(&mut req);
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = CreateSession(req).run(&state.ctx(&org)).await?;

    Ok((StatusCode::CREATED, Json(urls.wrap(session))))
}

/// Strip request fields that only trusted internal dispatch paths may set.
///
/// Trusted worker dispatch sets these fields by invoking the domain command
/// directly. Values supplied at the public HTTP boundary are forgery attempts
/// and must never influence delegation ownership, lineage, or budget linkage.
fn strip_internal_only_fields(req: &mut CreateSessionRequest) {
    // THREAT[TM-TENANT-014]: Never let public callers select delegation or
    // budget ownership metadata.
    req.parent_session_id = None;
    req.forked_from_session_id = None;
    req.budget_root_session_id = None;
    req.seed = SessionSeedMode::Fresh;
}

/// POST /v1/sessions/{session_id}/fork - Fork a session into an independent copy
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/fork",
    params(("session_id" = String, Path, description = "Session to fork")),
    request_body(
        content = ForkSessionRequest,
        example = json!({ "title": "Branch: try the async rewrite", "tags": ["experiment"] })
    ),
    responses(
        (status = 201, description = "Fork created successfully", body = WithUrls<Session>),
        (status = 404, description = "Parent session, agent, or harness not found", body = ErrorResponse),
        (status = 409, description = "Parent session is mid-turn, or ran on the OpenAI Agents API backend (code `agents_api_session_not_forkable`), and cannot be forked", body = ErrorResponse),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn fork_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    body: Option<Json<ForkSessionRequest>>,
) -> Result<(StatusCode, Json<WithUrls<Session>>), (StatusCode, Json<ErrorResponse>)> {
    let ctx = state.ctx(&org);

    // Authorize before consuming the org's session-create budget. Forking
    // creates a new session and deep-copies parent state, so successful forks
    // share the same per-org velocity limit as ordinary session creation.
    if let Some(policy) = ForkSession::policy() {
        policy
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|e| crate::domains::common::CommandError::forbidden(e.message))?;
    }

    // Per-org session-create throttle is enforced inside `ForkSession::execute`
    // (shared across REST and MCP dispatch), so no separate pre-check here.

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    let session = ForkSession {
        session_id,
        overrides: body.map(|Json(b)| b).unwrap_or_default(),
    }
    .run(&ctx)
    .await?;

    Ok((StatusCode::CREATED, Json(urls.wrap(session))))
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
        filters: SessionFilterArgs {
            agent_id: query.agent_id,
            side_chats_only: query.side_chats_only,
            playground_user_id: query.playground_user_id,
            archived_only: query.archived_only,
            search: query.search,
            source: query.source,
            status: query.status,
            mine: query.mine,
            include_archived: query.include_archived,
            created_after: query.created_after,
            created_before: query.created_before,
            order: query.order,
        },
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(
        PaginatedResponse::new(page.data, page.total, page.offset, page.limit).with_urls(&urls),
    ))
}

/// GET /v1/sessions/facets - Facet counts and masthead metrics
#[utoipa::path(
    get,
    path = "/v1/sessions/facets",
    params(SessionFacetsQuery),
    responses(
        (status = 200, description = "Facet counts over the applied filters", body = SessionFacetsResponse),
        (status = 400, description = "Unknown filter value"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session_facets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<SessionFacetsQuery>,
) -> ApiResult<SessionFacetsResponse> {
    Ok(Json(
        GetSessionFacets {
            filters: SessionFilterArgs {
                agent_id: query.agent_id,
                side_chats_only: query.side_chats_only,
                playground_user_id: query.playground_user_id,
                archived_only: query.archived_only,
                search: query.search,
                source: query.source,
                status: query.status,
                mine: query.mine,
                include_archived: query.include_archived,
                created_after: query.created_after,
                created_before: query.created_before,
                order: query.order,
            },
        }
        .run(&state.ctx(&org))
        .await?,
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

/// The model the runtime will use when a turn has no per-message override.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionResolvedModelResponse {
    #[schema(value_type = Option<String>)]
    pub model_id: Option<ModelId>,
}

/// GET /v1/sessions/{session_id}/resolved-model - Resolve the session's active model
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/resolved-model",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Resolved model for turns without a model override", body = SessionResolvedModelResponse),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn get_session_resolved_model(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionResolvedModelResponse> {
    let session = GetSession { session_id }.run(&state.ctx(&org)).await?;
    let model_id = state
        .session_service
        .resolved_model_id(org.org_id, &session)
        .await
        .log_internal_error_json("resolve session model")?;

    Ok(Json(SessionResolvedModelResponse { model_id }))
}

/// GET /v1/sessions/{session_id}/participants - List session participants
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/participants",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 200, description = "Session participant history", body = Vec<SessionParticipant>),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn list_session_participants(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Vec<SessionParticipant>> {
    Ok(Json(
        ListSessionParticipants { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

/// POST /v1/sessions/{session_id}/participants - Add a session participant
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/participants",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    request_body = AddSessionParticipantRequest,
    responses(
        (status = 201, description = "Participant added successfully", body = SessionParticipant),
        (status = 400, description = "Invalid participant request"),
        (status = 404, description = "Session or agent not found"),
        (status = 409, description = "Participant conflicts with current membership"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn add_session_participant(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(req): Json<AddSessionParticipantRequest>,
) -> Result<(StatusCode, Json<SessionParticipant>), (StatusCode, Json<ErrorResponse>)> {
    let participant = AddSessionParticipant { session_id, req }
        .run(&state.ctx(&org))
        .await?;

    Ok((StatusCode::CREATED, Json(participant)))
}

/// DELETE /v1/sessions/{session_id}/participants/{participant_id} - Leave a participant
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/participants/{participant_id}",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)"),
        ("participant_id" = String, Path, description = "Participant ID (prefixed, e.g., part_...)")
    ),
    responses(
        (status = 200, description = "Participant left successfully", body = SessionParticipant),
        (status = 400, description = "Invalid ID"),
        (status = 404, description = "Session or participant not found"),
        (status = 409, description = "Host participant cannot leave through this endpoint"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn leave_session_participant(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, participant_id)): Path<(String, String)>,
) -> ApiResult<SessionParticipant> {
    Ok(Json(
        LeaveSessionParticipant {
            session_id,
            participant_id,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
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

/// PUT /v1/sessions/{session_id}/archive - Archive session
#[utoipa::path(
    put,
    path = "/v1/sessions/{session_id}/archive",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 204, description = "Session archived successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn archive_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ArchiveSession { session_id }.run(&state.ctx(&org)).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /v1/sessions/{session_id}/archive - Restore an archived session
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/archive",
    params(
        ("session_id" = String, Path, description = "Session ID (prefixed, e.g., session_...)")
    ),
    responses(
        (status = 204, description = "Session unarchived successfully"),
        (status = 400, description = "Invalid session ID"),
        (status = 404, description = "Session not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "sessions"
)]
pub async fn unarchive_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    UnarchiveSession { session_id }
        .run(&state.ctx(&org))
        .await?;

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
mod tests;

/// Open the current user's permanent platform conversation, creating it only when absent.
#[utoipa::path(post, path = "/v1/sessions/platform-chat", responses((status = 200, description = "Permanent platform conversation", body = WithUrls<Session>)), tag = "sessions")]
pub async fn ensure_platform_chat(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<WithUrls<Session>>, (StatusCode, Json<ErrorResponse>)> {
    let session = crate::domains::sessions::EnsurePlatformChat {}
        .run(&state.ctx(&org))
        .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(session)))
}
