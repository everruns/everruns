// Session schedules HTTP routes
//
// Lists session-scoped schedules for the session detail UI.

use crate::auth::{AuthState, ResolvedOrg};
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::session_schedule::{SessionSchedule, SessionScheduleStatus};
use everruns_core::traits::SessionScheduleStore;
use everruns_core::typed_id::SessionId;
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::ListResponse;

/// Session schedule response (API shape)
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionScheduleResponse {
    pub id: String,
    pub session_id: String,
    pub description: String,
    pub scheduled_at: String,
    pub status: String,
    pub triggered_at: Option<String>,
    pub error: Option<String>,
    pub created_at: String,
}

impl From<SessionSchedule> for SessionScheduleResponse {
    fn from(s: SessionSchedule) -> Self {
        Self {
            id: s.id.to_string(),
            session_id: s.session_id.to_string(),
            description: s.description,
            scheduled_at: s.scheduled_at.to_rfc3339(),
            status: s.status.to_string(),
            triggered_at: s.triggered_at.map(|t| t.to_rfc3339()),
            error: s.error,
            created_at: s.created_at.to_rfc3339(),
        }
    }
}

/// Summary for a session: count of pending schedules
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SessionScheduleSummary {
    pub pending_count: usize,
}

/// App state for session schedule routes
#[derive(Clone)]
pub struct AppState {
    pub schedule_store: Option<Arc<dyn SessionScheduleStore>>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(schedule_store: Option<Arc<dyn SessionScheduleStore>>, auth: AuthState) -> Self {
        Self {
            schedule_store,
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/sessions/{session_id}/schedules",
            get(list_session_schedules),
        )
        .route(
            "/v1/sessions/{session_id}/schedules/summary",
            get(get_schedule_summary),
        )
        .with_state(state)
}

/// List all schedules for a session
async fn list_session_schedules(
    _org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<ListResponse<SessionScheduleResponse>>, StatusCode> {
    let store = state
        .schedule_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;

    let schedules = store.list_schedules(session_id).await.map_err(|e| {
        tracing::error!("Failed to list session schedules: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let data: Vec<SessionScheduleResponse> = schedules.into_iter().map(Into::into).collect();

    Ok(Json(ListResponse { data }))
}

/// Get summary (pending count) for a session's schedules
async fn get_schedule_summary(
    _org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<SessionScheduleSummary>, StatusCode> {
    let store = state
        .schedule_store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let session_id: SessionId = session_id.parse().map_err(|_| StatusCode::BAD_REQUEST)?;

    let schedules = store.list_schedules(session_id).await.map_err(|e| {
        tracing::error!("Failed to list session schedules for summary: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let pending_count = schedules
        .iter()
        .filter(|s| s.status == SessionScheduleStatus::Pending)
        .count();

    Ok(Json(SessionScheduleSummary { pending_count }))
}
