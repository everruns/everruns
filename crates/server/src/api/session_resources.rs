// Session leased-resource routes.
//
// These routes expose the generic leased-resource model so users can inspect
// provider-owned state that may outlive an individual tool call.

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::LeasedResourceService;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use everruns_core::{LeasedResource, SessionId};
use std::sync::Arc;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, impl_auth_state};

/// App state for session leased-resource routes.
#[derive(Clone)]
pub struct AppState {
    pub leased_resource_service: Arc<LeasedResourceService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(leased_resource_service: Arc<LeasedResourceService>, auth: AuthState) -> Self {
        Self {
            leased_resource_service,
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

/// List lifecycle-managed resources for a session.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/resources",
    responses(
        (status = 200, description = "Session leased resources", body = Vec<LeasedResource>),
        (status = 404, description = "Session not found"),
    ),
    tag = "session-resources"
)]
pub async fn list_resources(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<SessionId>,
) -> ApiResult<Vec<LeasedResource>> {
    let resources = state
        .leased_resource_service
        .list_for_session(org.org_id, session_id)
        .await
        .log_internal_error_json("list session resources")?
        .ok_or_not_found_json("Session")?;

    Ok(Json(resources))
}
