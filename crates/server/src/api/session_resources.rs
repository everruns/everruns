// Session resource routes.
//
// Exposes the session resource registry — a unified view of all resources
// active in a session (sandboxes, subagents, browser sessions, etc.).

use crate::api::state::ApiState;
use crate::auth::ResolvedOrg;
use crate::domains::common::Command;
use crate::domains::session_resources::ListSessionResources;
use crate::kernel_imports::{SessionResourceEntry, contracts::typed_id::SessionId};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};

use super::common::ApiResult;

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/resources", get(list_resources))
        .with_state(state)
}

/// List all resources registered in the session resource registry.
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/resources",
    responses(
        (status = 200, description = "Session resources", body = Vec<SessionResourceEntry>),
        (status = 404, description = "Session not found"),
    ),
    tag = "session-resources"
)]
pub async fn list_resources(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(session_id): Path<SessionId>,
) -> ApiResult<Vec<SessionResourceEntry>> {
    Ok(Json(
        ListSessionResources {
            session_id: session_id.to_string(),
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}
