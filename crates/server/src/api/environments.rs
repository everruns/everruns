// Environment HTTP routes and wire types.
//
// An Environment is where a session's commands run plus what they may touch.
// Today both are derived from the session's effective capabilities, because
// environment profiles are not yet first-class configuration; the wire shape
// says so with `resolved_from` rather than implying a stored profile.
//
// See knowledge/harnesses/execution-environments.md.

use std::sync::Arc;

use axum::{Json, Router, extract::Path, extract::State, routing::get};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::environments::commands::{GetSessionEnvironment, ListEnvironmentTargets};
use crate::domains::sessions::SessionService;
use crate::storage::StorageBackend;
use everruns_core::Caller;

use super::common::{ApiResult, impl_auth_state};

/// Where a session's commands run.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentTarget {
    /// Shape of the target: `host`, `machine`, `vfs`, `container`, `managed`.
    pub kind: String,
    /// Concrete provider, when the kind has one (`bashkit`, `daytona`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
}

/// What commands may touch, and who enforces it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentContainment {
    /// `none`, `native`, or `isolated`.
    pub level: String,
    /// Outbound network policy: `deny`, `allowlist`, or `allow`.
    pub network: String,
}

/// What the environment can actually do.
///
/// Read this before assuming a shell behaves like Linux. Bashkit reports
/// `native_processes: false`, which is why a build fails there; the answer is
/// available before the first turn rather than after a confusing tool error.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentCapabilities {
    pub native_processes: bool,
    pub packages: bool,
    pub pty: bool,
    pub ports: bool,
    pub portable_checkpoint: bool,
    pub network_enforced: bool,
}

/// The environment a session is running in.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionEnvironmentResponse {
    /// Target, absent when the session has no compute at all and only reads and
    /// writes files. That is a real configuration, not a misconfiguration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EnvironmentTarget>,
    pub containment: EnvironmentContainment,
    /// `checkpointed`, `provider_snapshot`, or `none`. Declared per target, so a
    /// session on somebody else's machine is never reported as recoverable.
    pub durability: String,
    pub capabilities: EnvironmentCapabilities,
    /// How this view was produced. `capabilities` means it was derived from the
    /// session's effective capability set rather than read from a stored
    /// environment profile.
    pub resolved_from: String,
    /// Capability that supplied the compute, for operators tracing a surprise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_capability: Option<String>,
}

/// One target this deployment can offer, and what it can do.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentTargetDescriptor {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Whether this deployment can actually run it right now.
    pub available: bool,
    /// Why it is unavailable. Present only when `available` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub capabilities: EnvironmentCapabilities,
    /// Containment levels this target supports, weakest first.
    pub containment_levels: Vec<String>,
    pub durability: String,
}

/// Response body for the `list_environment_targets` operation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentTargetsResponse {
    pub items: Vec<EnvironmentTargetDescriptor>,
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub session_service: Arc<SessionService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        session_service: Arc<SessionService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            session_service,
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
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/sessions/{session_id}/environment",
            get(get_session_environment),
        )
        .route("/v1/environment-targets", get(list_environment_targets))
        .with_state(state)
}

#[utoipa::path(
    description = "Get the environment a session runs in: target, containment, and what it can actually do.",
    get,
    path = "/v1/sessions/{session_id}/environment",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (
            status = 200,
            description = "Resolved session environment",
            body = SessionEnvironmentResponse,
            example = json!({
                "target": { "kind": "vfs", "provider": "bashkit" },
                "containment": { "level": "isolated", "network": "deny" },
                "durability": "checkpointed",
                "capabilities": {
                    "native_processes": false,
                    "packages": false,
                    "pty": false,
                    "ports": false,
                    "portable_checkpoint": true,
                    "network_enforced": true
                },
                "resolved_from": "capabilities",
                "source_capability": "bashkit_shell"
            })
        ),
        (status = 404, description = "Session not found"),
    ),
    tag = "environments"
)]
pub async fn get_session_environment(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionEnvironmentResponse> {
    Ok(Json(
        GetSessionEnvironment { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "List the environment targets this deployment can offer, with the capabilities each one actually has.",
    get,
    path = "/v1/environment-targets",
    responses(
        (status = 200, description = "Available environment targets", body = EnvironmentTargetsResponse),
    ),
    tag = "environments"
)]
pub async fn list_environment_targets(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<EnvironmentTargetsResponse> {
    Ok(Json(ListEnvironmentTargets.run(&state.ctx(&org)).await?))
}
