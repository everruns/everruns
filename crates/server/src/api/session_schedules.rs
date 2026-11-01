// Session Schedule HTTP routes
// Routes nested under /v1/sessions/{session_id}/schedules

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::session_schedules::{ScheduleChange, SessionScheduleService};
use crate::domains::sessions::{SESSION_MANAGE, SESSION_VIEW, SessionService};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_contracts::typed_id::{ScheduleId, SessionId};
use everruns_core::session_schedule::SessionSchedule;
use everruns_core::{Caller, Policy};

use super::common::{
    ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse, UrlBuilder, WithUrls, impl_auth_state,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;

// ============================================
// App State
// ============================================

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub schedule_service: Arc<SessionScheduleService>,
    pub session_service: Arc<SessionService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            schedule_service: Arc::new(SessionScheduleService::new(db.clone())),
            session_service: Arc::new(SessionService::new(db.clone())),
            db,
            auth,
        }
    }
}

impl_auth_state!(AppState);

// ============================================
// Request/Response types
// ============================================

#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateScheduleRequest {
    /// Enable or disable the schedule. Re-enabling clears `stop_reason`.
    pub enabled: Option<bool>,
    /// New end time; null removes it. A person may set any future time.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<chrono::DateTime<chrono::Utc>>)]
    pub ends_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    /// New run cap, counted from the schedule's first run; null removes it.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(value_type = Option<u32>, minimum = 1)]
    pub max_runs: Option<Option<u32>>,
    /// New spend cap for the schedule's runs, in USD.
    #[serde(default)]
    pub budget_usd: Option<f64>,
}

fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

// ============================================
// Routes
// ============================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/schedules", get(list_schedules))
        .route(
            "/v1/sessions/{session_id}/schedules/{schedule_id}",
            get(get_schedule)
                .patch(update_schedule)
                .delete(delete_schedule),
        )
        .route(
            "/v1/sessions/{session_id}/schedules/{schedule_id}/trigger",
            post(trigger_schedule),
        )
        .with_state(state)
}

// ============================================
// Handlers
// ============================================

/// List all schedules for a session.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/schedules",
    responses(
        (status = 200, description = "List of schedules", body = [WithUrls<SessionSchedule>]),
        (status = 403, description = "Caller may not view this session"),
    ),
    tag = "session-schedules"
)]
pub async fn list_schedules(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<SessionId>,
) -> ApiResult<Vec<WithUrls<SessionSchedule>>> {
    authorize(&state, &org, &SESSION_VIEW)?;
    let schedules = state
        .schedule_service
        .list(org.org_id, session_id)
        .await
        .log_internal_error_json("list schedules")?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap_vec(schedules)))
}

/// Get a specific schedule.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/schedules/{schedule_id}",
    responses(
        (status = 200, description = "Schedule details", body = WithUrls<SessionSchedule>),
        (status = 403, description = "Caller may not view this session"),
        (status = 404, description = "Schedule not found"),
    ),
    tag = "session-schedules"
)]
pub async fn get_schedule(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(SessionId, ScheduleId)>,
) -> ApiResult<WithUrls<SessionSchedule>> {
    authorize(&state, &org, &SESSION_VIEW)?;
    let schedule =
        get_schedule_in_session(&state, org.org_id, session_id, schedule_id, "get schedule")
            .await?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(schedule)))
}

/// Update a schedule (enable/disable).
#[utoipa::path(
    patch,
    path = "/v1/sessions/{session_id}/schedules/{schedule_id}",
    request_body = UpdateScheduleRequest,
    responses(
        (status = 200, description = "Updated schedule", body = WithUrls<SessionSchedule>),
        (status = 403, description = "Caller may not manage this session"),
        (status = 404, description = "Schedule not found"),
    ),
    tag = "session-schedules"
)]
pub async fn update_schedule(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(SessionId, ScheduleId)>,
    Json(req): Json<UpdateScheduleRequest>,
) -> ApiResult<WithUrls<SessionSchedule>> {
    authorize(&state, &org, &SESSION_MANAGE)?;
    // An empty body keeps its old meaning: enable the schedule.
    let enabled = match (&req.enabled, &req.ends_at, &req.max_runs, &req.budget_usd) {
        (None, None, None, None) => Some(true),
        _ => req.enabled,
    };
    get_schedule_in_session(
        &state,
        org.org_id,
        session_id,
        schedule_id,
        "update schedule parent",
    )
    .await?;
    require_platform_chat_owner(&state, &org, session_id).await?;

    let schedule = match state
        .schedule_service
        .update(
            org.org_id,
            schedule_id,
            ScheduleChange {
                enabled,
                ends_at: req.ends_at,
                max_runs: req.max_runs,
                budget_usd: req.budget_usd,
            },
        )
        .await
    {
        Ok(schedule) => schedule,
        Err(Ok(message)) => {
            return Err(ErrorResponse::new(message).into_response(StatusCode::BAD_REQUEST));
        }
        Err(Err(error)) => {
            return Err(Err::<Option<SessionSchedule>, _>(error)
                .log_internal_error_json("update schedule")
                .expect_err("an error stays an error"));
        }
    }
    .ok_or_not_found_json("Schedule")?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(schedule)))
}

/// Delete a schedule.
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/schedules/{schedule_id}",
    responses(
        (status = 204, description = "Schedule deleted"),
        (status = 403, description = "Caller may not manage this session"),
        (status = 404, description = "Schedule not found"),
    ),
    tag = "session-schedules"
)]
pub async fn delete_schedule(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(SessionId, ScheduleId)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    authorize(&state, &org, &SESSION_MANAGE)?;
    get_schedule_in_session(
        &state,
        org.org_id,
        session_id,
        schedule_id,
        "delete schedule parent",
    )
    .await?;
    require_platform_chat_owner(&state, &org, session_id).await?;

    let deleted = state
        .schedule_service
        .delete(org.org_id, schedule_id)
        .await
        .log_internal_error_json("delete schedule")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("Schedule not found".to_string())
            .into_response(StatusCode::NOT_FOUND))
    }
}

/// Manually trigger a schedule (fire it now).
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/schedules/{schedule_id}/trigger",
    responses(
        (status = 200, description = "Schedule triggered", body = WithUrls<SessionSchedule>),
        (status = 403, description = "Caller may not manage this session"),
        (status = 404, description = "Schedule not found"),
    ),
    tag = "session-schedules"
)]
pub async fn trigger_schedule(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(SessionId, ScheduleId)>,
) -> ApiResult<WithUrls<SessionSchedule>> {
    authorize(&state, &org, &SESSION_MANAGE)?;
    get_schedule_in_session(
        &state,
        org.org_id,
        session_id,
        schedule_id,
        "trigger schedule parent",
    )
    .await?;
    require_platform_chat_owner(&state, &org, session_id).await?;

    let schedule = state
        .schedule_service
        .mark_triggered(org.org_id, schedule_id)
        .await
        .log_internal_error_json("trigger schedule")?
        .ok_or_not_found_json("Schedule")?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(schedule)))
}

/// Schedules belong to a session, so they take the session's policies:
/// reading needs `SESSION_VIEW`, changing or firing needs `SESSION_MANAGE`.
///
/// THREAT[TM-SCHED-007]: evaluated with the configured resolver before any
/// schedule lookup, so a same-org caller the resolver denies learns nothing
/// about the schedule and cannot change it. Org scoping and the parent-session
/// match below still apply to callers who pass.
fn authorize(
    state: &AppState,
    org: &ResolvedOrg,
    policy: &Policy,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    policy
        .evaluate_with(state.auth.permission_resolver.as_ref(), &Caller::from(org))
        .map_err(|error| ErrorResponse::new(error.to_string()).into_response(StatusCode::FORBIDDEN))
}

/// THREAT[TM-AGENT-017]: Platform Chat acts with its persisted owner's
/// authority, and posting messages to it is owner-bound. Firing, re-enabling,
/// disabling, or deleting its schedules would let another same-org user with
/// `SESSION_MANAGE` make the assistant act (or stop acting) on the owner's
/// behalf at a time they choose, so changes bind to that owner too. Reads stay
/// unbound, like session reads. Runs after the policy check and the
/// parent-session match.
async fn require_platform_chat_owner(
    state: &AppState,
    org: &ResolvedOrg,
    session_id: SessionId,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let caller = Caller::from(org);
    let session = state
        .session_service
        .get(&caller, session_id.uuid(), None)
        .await
        .log_internal_error_json("get schedule session")?
        .ok_or_not_found_json("Schedule")?;
    if !crate::domains::sessions::platform_chat_owner_matches_session(&state.db, &caller, &session)
        .await
        .log_internal_error_json("authorize session owner")?
    {
        return Err(ErrorResponse::new(
            "Only the Platform Chat session owner can manage its schedules".to_string(),
        )
        .into_response(StatusCode::FORBIDDEN));
    }
    Ok(())
}

async fn get_schedule_in_session(
    state: &AppState,
    org_id: i64,
    session_id: SessionId,
    schedule_id: ScheduleId,
    log_context: &'static str,
) -> Result<SessionSchedule, (StatusCode, Json<ErrorResponse>)> {
    state
        .schedule_service
        .get(org_id, schedule_id)
        .await
        .log_internal_error_json(log_context)?
        .filter(|schedule| schedule.session_id == session_id)
        .ok_or_not_found_json("Schedule")
}
