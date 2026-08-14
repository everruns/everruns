// Session resource routes.
//
// Exposes the session resource registry — a unified view of all resources
// active in a session (sandboxes, subagents, browser sessions, etc.).

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::session_resources::ListSessionResources;
use crate::kernel_imports::{Caller, SessionResourceEntry, everruns_provider::typed_id::SessionId};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use std::sync::Arc;

use super::common::{ApiResult, impl_auth_state};

/// App state for session resource routes.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
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
    State(state): State<AppState>,
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
