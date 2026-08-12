// Session sandbox HTTP routes.
//
// Exposes the managed session-owned sandbox lifecycle surface.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::session_sandbox::{
    GetSessionSandbox, ManageSessionSandbox, SessionSandboxService,
};
use crate::domains::sessions::SessionService;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use everruns_core::Caller;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::{ApiResult, impl_auth_state};

/// Wire-facing status of a session sandbox. Mirrors
/// `everruns_platform::session_sandbox::SessionSandboxStatus` for the public API.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionSandboxStatusValue {
    Running,
    Paused,
    Lost,
}

impl From<everruns_platform::session_sandbox::SessionSandboxStatus> for SessionSandboxStatusValue {
    fn from(status: everruns_platform::session_sandbox::SessionSandboxStatus) -> Self {
        match status {
            everruns_platform::session_sandbox::SessionSandboxStatus::Running => Self::Running,
            everruns_platform::session_sandbox::SessionSandboxStatus::Paused => Self::Paused,
            everruns_platform::session_sandbox::SessionSandboxStatus::Lost => Self::Lost,
        }
    }
}

/// Operator action to take against a session's managed sandbox. `Pause`
/// suspends the instance, `Resume` restarts it, `Delete` releases the
/// lease.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionSandboxAction {
    Pause,
    Resume,
    Delete,
}

/// Response body for the `get_session_sandbox` operation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GetSessionSandboxResponse {
    /// Whether the session's harness opts in to a managed sandbox at all.
    pub configured: bool,
    /// Whether a sandbox instance currently exists for this session (within its lease).
    pub exists: bool,
    /// Sandbox provider (`daytona`, `e2b`, `docker`, etc.) when one is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Current sandbox lifecycle status (`starting`, `ready`, `error`, `released`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_status: Option<SessionSandboxStatusValue>,
    /// Provider-side sandbox identifier (workspace ID, container ID, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Human-readable sandbox label. Safe to render in user-facing messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Absolute path of the sandbox workspace root (used to scope file operations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
    /// Provider-specific metadata (URLs, ports, credentials envelopes).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Object)]
    pub metadata: Option<serde_json::Value>,
    /// Timestamp when sandbox initialization finished (RFC 3339); absent while still starting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub init_completed_at: Option<String>,
    /// Most recent initialization error message; cleared on successful re-init.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_init_error: Option<String>,
    /// Timestamp when this sandbox record was created (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Timestamp when this sandbox record was last updated (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
}

/// Request body for the `manage_session_sandbox` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ManageSessionSandboxRequest {
    /// Action to take on the sandbox (`reset`, `delete`, etc.).
    pub action: SessionSandboxAction,
}

/// Response body for the `manage_session_sandbox` operation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ManageSessionSandboxResponse {
    /// Action that was taken.
    pub action: SessionSandboxAction,
    /// Whether a sandbox instance still exists after the action.
    pub exists: bool,
    /// Whether the action deleted the sandbox (`true` after a successful `delete`).
    pub deleted: bool,
    /// Sandbox provider (`daytona`, `e2b`, `docker`, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Current sandbox lifecycle status after the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_status: Option<SessionSandboxStatusValue>,
    /// Provider-side sandbox identifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    /// Human-readable sandbox label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Absolute path of the sandbox workspace root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<String>,
}

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
        .route(
            "/v1/sessions/{session_id}/sandbox",
            get(get_sandbox).post(manage_sandbox),
        )
        .with_state(state)
}

#[utoipa::path(
    description = "Get the current session sandbox status (lease, runtime image, network access).",
    get,
    path = "/v1/sessions/{session_id}/sandbox",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (status = 200, description = "Managed sandbox status", body = GetSessionSandboxResponse),
        (status = 404, description = "Session not found"),
    ),
    tag = "session-sandbox"
)]
pub async fn get_sandbox(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<GetSessionSandboxResponse> {
    Ok(Json(
        GetSessionSandbox { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
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
