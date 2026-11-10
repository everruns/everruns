// Session sandbox HTTP routes.
//
// Exposes the managed session-owned sandbox lifecycle surface.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
pub use crate::domains::session_sandbox::types::{
    ManageSessionSandboxRequest, ManageSessionSandboxResponse, SessionRepositoriesResponse,
    SessionRepository, SessionRepositoryFile, SessionSandboxAction, SessionSandboxStatusValue,
};
use crate::domains::session_sandbox::{
    GetSessionRepositories, ManageSessionSandbox, SessionSandboxService,
};
use crate::domains::sessions::SessionService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use everruns_core::Caller;
use std::sync::Arc;

use super::common::{ApiResult, impl_auth_state};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub session_sandbox_service: Arc<SessionSandboxService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        session_service: Arc<SessionService>,
        session_sandbox_service: Arc<SessionSandboxService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            session_service,
            session_sandbox_service,
            auth,
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
        .with_session_sandbox_service(self.session_sandbox_service.clone())
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/sandbox", post(manage_sandbox))
        .route(
            "/v1/sessions/{session_id}/repositories",
            get(get_session_repositories),
        )
        .with_state(state)
}

#[utoipa::path(
    description = "Manage the session sandbox lifecycle (start, stop, reset).",
    post,
    path = "/v1/sessions/{session_id}/sandbox",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    request_body = ManageSessionSandboxRequest,
    responses(
        (status = 200, description = "Managed sandbox lifecycle updated", body = ManageSessionSandboxResponse),
        (status = 400, description = "Invalid action or sandbox not configured"),
        (status = 404, description = "Session not found"),
    ),
    tag = "session-sandbox"
)]
pub async fn manage_sandbox(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Json(body): Json<ManageSessionSandboxRequest>,
) -> ApiResult<ManageSessionSandboxResponse> {
    Ok(Json(
        ManageSessionSandbox {
            session_id,
            action: body.action,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

#[utoipa::path(
    description = "List the repositories checked out in the session's sandbox and what the session changed in each. Reads a running sandbox only; it never starts or resumes one.",
    get,
    path = "/v1/sessions/{session_id}/repositories",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Repositories and their changes", body = SessionRepositoriesResponse),
        (status = 404, description = "Session not found"),
    ),
    tag = "session-sandbox"
)]
pub async fn get_session_repositories(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionRepositoriesResponse> {
    Ok(Json(
        GetSessionRepositories { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}
