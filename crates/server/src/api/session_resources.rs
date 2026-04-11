// Session resource routes.
//
// Exposes a unified view of all resources active in a session — leased
// resources (sandboxes, browsers) and subagents.

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::SessionResourceService;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use everruns_core::{SessionId, SessionResource};
use std::sync::Arc;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, impl_auth_state};

/// App state for session resource routes.
#[derive(Clone)]
pub struct AppState {
    pub session_resource_service: Arc<SessionResourceService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(session_resource_service: Arc<SessionResourceService>, auth: AuthState) -> Self {
        Self {
            session_resource_service,
            auth,
        }
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/resources", get(list_resources))
        .with_state(state)
}

/// List all resources for a session (leased resources + subagents).
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/resources",
    responses(
        (status = 200, description = "Session resources", body = Vec<SessionResource>),
        (status = 404, description = "Session not found"),
    ),
    tag = "session-resources"
)]
pub async fn list_resources(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<SessionId>,
) -> ApiResult<Vec<SessionResource>> {
    let resources = state
        .session_resource_service
        .list_for_session(org.org_id, session_id)
        .await
        .log_internal_error_json("list session resources")?
        .ok_or_not_found_json("Session")?;

    Ok(Json(resources))
}
