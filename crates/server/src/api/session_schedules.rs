// Session Schedule HTTP routes
// Routes nested under /v1/sessions/{session_id}/schedules

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::SessionScheduleService;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::session_schedule::SessionSchedule;
use everruns_core::typed_id::{ScheduleId, SessionId};

use super::common::{ApiOptionExt, ApiResultExt, ErrorResponse};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;

// ============================================
// App State
// ============================================

#[derive(Clone)]
pub struct AppState {
    pub schedule_service: Arc<SessionScheduleService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(schedule_service: Arc<SessionScheduleService>, auth: AuthState) -> Self {
        Self {
            schedule_service,
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(state: &AppState) -> Self {
        state.auth.clone()
    }
}

// ============================================
// Request/Response types
// ============================================

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateScheduleRequest {
    /// Enable or disable the schedule.
    pub enabled: Option<bool>,
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
        (status = 200, description = "List of schedules", body = Vec<SessionSchedule>),
    ),
    tag = "session-schedules"
)]
pub async fn list_schedules(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<SessionId>,
) -> Result<Json<Vec<SessionSchedule>>, (StatusCode, Json<ErrorResponse>)> {
    let schedules = state
        .schedule_service
        .list(org.org_id, session_id)
        .await
        .log_internal_error_json("list schedules")?;

    Ok(Json(schedules))
}

/// Get a specific schedule.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/schedules/{schedule_id}",
    responses(
        (status = 200, description = "Schedule details", body = SessionSchedule),
        (status = 404, description = "Schedule not found"),
    ),
    tag = "session-schedules"
)]
pub async fn get_schedule(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(SessionId, ScheduleId)>,
) -> Result<Json<SessionSchedule>, (StatusCode, Json<ErrorResponse>)> {
    let schedule =
        get_schedule_in_session(&state, org.org_id, session_id, schedule_id, "get schedule")
            .await?;

    Ok(Json(schedule))
}

/// Update a schedule (enable/disable).
#[utoipa::path(
    patch,
    path = "/v1/sessions/{session_id}/schedules/{schedule_id}",
    request_body = UpdateScheduleRequest,
    responses(
        (status = 200, description = "Updated schedule", body = SessionSchedule),
        (status = 404, description = "Schedule not found"),
    ),
    tag = "session-schedules"
)]
pub async fn update_schedule(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(SessionId, ScheduleId)>,
    Json(req): Json<UpdateScheduleRequest>,
) -> Result<Json<SessionSchedule>, (StatusCode, Json<ErrorResponse>)> {
    let enabled = req.enabled.unwrap_or(true);
    get_schedule_in_session(
        &state,
        org.org_id,
        session_id,
        schedule_id,
        "update schedule parent",
    )
    .await?;

    let schedule = state
        .schedule_service
        .update_enabled(org.org_id, schedule_id, enabled)
        .await
        .log_internal_error_json("update schedule")?
        .ok_or_not_found_json("Schedule")?;

    Ok(Json(schedule))
}

/// Delete a schedule.
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/schedules/{schedule_id}",
    responses(
        (status = 204, description = "Schedule deleted"),
        (status = 404, description = "Schedule not found"),
    ),
    tag = "session-schedules"
)]
pub async fn delete_schedule(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(SessionId, ScheduleId)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    get_schedule_in_session(
        &state,
        org.org_id,
        session_id,
        schedule_id,
        "delete schedule parent",
    )
    .await?;

    let deleted = state
        .schedule_service
        .delete(org.org_id, schedule_id)
        .await
        .log_internal_error_json("delete schedule")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Schedule not found".to_string(),
            }),
        ))
    }
}

/// Manually trigger a schedule (fire it now).
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/schedules/{schedule_id}/trigger",
    responses(
        (status = 200, description = "Schedule triggered", body = SessionSchedule),
        (status = 404, description = "Schedule not found"),
    ),
    tag = "session-schedules"
)]
pub async fn trigger_schedule(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id, schedule_id)): Path<(SessionId, ScheduleId)>,
) -> Result<Json<SessionSchedule>, (StatusCode, Json<ErrorResponse>)> {
    get_schedule_in_session(
        &state,
        org.org_id,
        session_id,
        schedule_id,
        "trigger schedule parent",
    )
    .await?;

    let schedule = state
        .schedule_service
        .mark_triggered(org.org_id, schedule_id)
        .await
        .log_internal_error_json("trigger schedule")?
        .ok_or_not_found_json("Schedule")?;

    Ok(Json(schedule))
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
