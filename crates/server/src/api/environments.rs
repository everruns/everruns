// Environment HTTP routes and wire types.
//
// An Environment is where a session's commands run plus what they may touch.
// New sessions return their immutable, resolved profile snapshot. Legacy
// sessions retain a capability-derived compatibility view; `resolved_from`
// makes the distinction explicit.
//
// See knowledge/harnesses/execution-environments.md.

use std::sync::Arc;

use axum::{Json, Router, extract::Path, extract::State, http::StatusCode, routing::get};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::environments::commands::{GetSessionEnvironment, ListEnvironmentTargets};
use crate::domains::sessions::SessionService;
use crate::storage::StorageBackend;
use everruns_contracts::typed_id::EnvironmentId;
use everruns_core::Caller;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse, impl_auth_state};

const MAX_ENVIRONMENT_DESCRIPTION_BYTES: usize = 10 * 1024;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEnvironmentRequest {
    /// Stable addressable name used by Agent environment references.
    #[schema(example = "coding-daytona")]
    pub name: String,
    /// Human-readable name shown in management surfaces.
    #[schema(example = "Coding - Daytona")]
    pub display_name: String,
    /// Optional explanation of the Environment's intended workload.
    #[serde(default)]
    #[schema(example = "Recoverable coding workspace managed by Daytona")]
    pub description: Option<String>,
    /// Initial immutable execution profile revision.
    pub profile: crate::records::EnvironmentProfile,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ReviseEnvironmentRequest {
    /// Replacement display name; omit to preserve the current value.
    #[serde(default)]
    #[schema(example = "Coding - Daytona (large)")]
    pub display_name: Option<String>,
    /// Omit to preserve; send null to clear.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(example = "Larger recoverable workspace for repository builds")]
    pub description: Option<Option<String>>,
    /// Complete profile stored as the next immutable revision.
    pub profile: crate::records::EnvironmentProfile,
}

fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

/// Where a session's commands run.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentTarget {
    /// Shape of the target: `host`, `machine`, `vfs`, `container`, `managed`.
    #[schema(example = "managed")]
    pub kind: String,
    /// Concrete provider, when the kind has one (`bashkit`, `daytona`, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "daytona")]
    pub provider: Option<String>,
    /// Registered connection used by a machine target.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "conn_01933b5a000070008000000000000001")]
    pub connection_id: Option<String>,
}

/// What commands may touch, and who enforces it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentContainment {
    /// `none`, `native`, or `isolated`.
    #[schema(example = "isolated")]
    pub level: String,
    /// Outbound network policy: `deny`, `allowlist`, or `allow`.
    #[schema(example = "allow")]
    pub network: String,
}

/// What the environment can actually do.
///
/// Read this before assuming a shell behaves like Linux. Bashkit reports
/// `native_processes: false`, which is why a build fails there; the answer is
/// available before the first turn rather than after a confusing tool error.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentCapabilities {
    /// Whether commands can spawn operating-system processes.
    pub native_processes: bool,
    /// Whether the runtime can install operating-system packages.
    pub packages: bool,
    /// Whether interactive pseudo-terminals are supported.
    pub pty: bool,
    /// Whether workloads can bind and expose network ports.
    pub ports: bool,
    /// Whether filesystem state has a portable checkpoint representation.
    pub portable_checkpoint: bool,
    /// Whether the target enforces the declared outbound network policy.
    pub network_enforced: bool,
}

/// The environment a session is running in.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionEnvironmentResponse {
    /// Deprecated alias for `sandbox_id` during the Environment-to-Sandbox migration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Durable logical primary Sandbox id. Absent for legacy capability-derived sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,
    /// Reusable Environment revision pinned into this Sandbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment_revision_id: Option<String>,
    /// `primary` for the implicit shell/files binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Agent profile name selected for this Session (`inline` for one-offs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// Immutable resolved profile pinned when the Session was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<crate::records::ResolvedEnvironmentProfile>,
    /// Control-plane lifecycle intent and latest observed physical state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desired_state: Option<String>,
    /// Latest lifecycle state observed from the physical provider resource.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed_state: Option<String>,
    /// Physical incarnation fence. Increments whenever compute is replaced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation: Option<i64>,
    /// Most recent checkpoint used to recover this logical Sandbox.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_checkpoint_id: Option<String>,
    /// Last recorded runtime activity used by lifecycle policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_activity_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// One target this deployment can offer, and what it can do.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentTargetDescriptor {
    /// Provider-neutral target class.
    pub kind: String,
    /// Concrete provider adapter, when the target class requires one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Whether this deployment can actually run it right now.
    pub available: bool,
    /// Why it is unavailable. Present only when `available` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Capabilities the deployment can honestly provide for this target.
    pub capabilities: EnvironmentCapabilities,
    /// Containment levels this target supports, weakest first.
    pub containment_levels: Vec<String>,
    /// Recovery guarantee offered by this target.
    pub durability: String,
}

/// Response body for the `list_environment_targets` operation.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct EnvironmentTargetsResponse {
    /// Target descriptors known to this deployment.
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
        // Carry the org-effective flags for capability and policy resolution.
        // Environments are a core surface, not feature-gated.
        .with_feature_flags(org.feature_flags.clone())
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/environments",
            get(list_environments).post(create_environment),
        )
        .route(
            "/v1/environments/{environment_id}",
            get(get_environment_definition)
                .put(revise_environment)
                .delete(archive_environment),
        )
        .route(
            "/v1/sessions/{session_id}/environment",
            get(get_session_environment),
        )
        .route("/v1/environment-targets", get(list_environment_targets))
        .with_state(state)
}

fn authorize(
    state: &AppState,
    org: &ResolvedOrg,
    policy: &everruns_core::Policy,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    policy
        .evaluate_with(state.auth.permission_resolver.as_ref(), &Caller::from(org))
        .map_err(|error| ErrorResponse::new(error.to_string()).into_response(StatusCode::FORBIDDEN))
}

#[utoipa::path(description = "List active reusable Environments available to the organization.", get, path = "/v1/environments", responses((status = 200, body = Vec<crate::records::EnvironmentDefinition>)), tag = "environments")]
pub async fn list_environments(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<Vec<crate::records::EnvironmentDefinition>> {
    authorize(&state, &org, &crate::domains::harnesses::HARNESS_VIEW)?;
    Ok(Json(
        state
            .db
            .list_environment_definitions(org.org_id, false)
            .await
            .log_internal_error_json("list environments")?,
    ))
}

#[utoipa::path(description = "Create a reusable Environment and its first immutable revision.", post, path = "/v1/environments", request_body = CreateEnvironmentRequest, responses((status = 200, body = crate::records::EnvironmentDefinition)), tag = "environments")]
pub async fn create_environment(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(request): Json<CreateEnvironmentRequest>,
) -> ApiResult<crate::records::EnvironmentDefinition> {
    authorize(&state, &org, &crate::domains::harnesses::HARNESS_MANAGE)?;
    crate::records::validate_addressable_name(&request.name).map_err(|error| {
        ErrorResponse::new(error).into_response(StatusCode::UNPROCESSABLE_ENTITY)
    })?;
    if request.display_name.trim().is_empty() || request.display_name.len() > 200 {
        return Err(
            ErrorResponse::new("display_name must be between 1 and 200 bytes")
                .into_response(StatusCode::UNPROCESSABLE_ENTITY),
        );
    }
    if request
        .description
        .as_ref()
        .is_some_and(|description| description.len() > MAX_ENVIRONMENT_DESCRIPTION_BYTES)
    {
        return Err(
            ErrorResponse::new("description must be at most 10240 bytes")
                .into_response(StatusCode::UNPROCESSABLE_ENTITY),
        );
    }
    if request.profile.source_revision_id.is_some() {
        return Err(ErrorResponse::new(
            "Reusable Environment profiles cannot set source_revision_id",
        )
        .into_response(StatusCode::UNPROCESSABLE_ENTITY));
    }
    crate::domains::environments::profiles::resolve_profile(&request.profile).map_err(|error| {
        ErrorResponse::new(error).into_response(StatusCode::UNPROCESSABLE_ENTITY)
    })?;
    if state
        .db
        .list_environment_definitions(org.org_id, false)
        .await
        .log_internal_error_json("check environment name")?
        .iter()
        .any(|environment| environment.name == request.name)
    {
        return Err(
            ErrorResponse::new("An active Environment already uses this name")
                .into_response(StatusCode::CONFLICT),
        );
    }
    let environment = state
        .db
        .create_environment_definition(
            org.org_id,
            &request.name,
            request.display_name.trim(),
            request.description.as_deref(),
            &request.profile,
            false,
        )
        .await
        .log_internal_error_json("create environment")?;
    Ok(Json(environment))
}

#[utoipa::path(description = "Get one reusable Environment and its current immutable revision.", get, path = "/v1/environments/{environment_id}", params(("environment_id" = String, Path)), responses((status = 200, body = crate::records::EnvironmentDefinition)), tag = "environments")]
pub async fn get_environment_definition(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(environment_id): Path<EnvironmentId>,
) -> ApiResult<crate::records::EnvironmentDefinition> {
    authorize(&state, &org, &crate::domains::harnesses::HARNESS_VIEW)?;
    Ok(Json(
        state
            .db
            .get_environment_definition(org.org_id, environment_id)
            .await
            .log_internal_error_json("get environment")?
            .ok_or_not_found_json("Environment")?,
    ))
}

#[utoipa::path(description = "Create the next immutable revision of a reusable Environment.", put, path = "/v1/environments/{environment_id}", params(("environment_id" = String, Path)), request_body = ReviseEnvironmentRequest, responses((status = 200, body = crate::records::EnvironmentDefinition)), tag = "environments")]
pub async fn revise_environment(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(environment_id): Path<EnvironmentId>,
    Json(request): Json<ReviseEnvironmentRequest>,
) -> ApiResult<crate::records::EnvironmentDefinition> {
    authorize(&state, &org, &crate::domains::harnesses::HARNESS_MANAGE)?;
    if request
        .display_name
        .as_ref()
        .is_some_and(|display_name| display_name.trim().is_empty() || display_name.len() > 200)
    {
        return Err(
            ErrorResponse::new("display_name must be between 1 and 200 bytes")
                .into_response(StatusCode::UNPROCESSABLE_ENTITY),
        );
    }
    if request
        .description
        .as_ref()
        .and_then(|description| description.as_ref())
        .is_some_and(|description| description.len() > MAX_ENVIRONMENT_DESCRIPTION_BYTES)
    {
        return Err(
            ErrorResponse::new("description must be at most 10240 bytes")
                .into_response(StatusCode::UNPROCESSABLE_ENTITY),
        );
    }
    if request.profile.source_revision_id.is_some() {
        return Err(ErrorResponse::new(
            "Reusable Environment profiles cannot set source_revision_id",
        )
        .into_response(StatusCode::UNPROCESSABLE_ENTITY));
    }
    crate::domains::environments::profiles::resolve_profile(&request.profile).map_err(|error| {
        ErrorResponse::new(error).into_response(StatusCode::UNPROCESSABLE_ENTITY)
    })?;
    let current = state
        .db
        .get_environment_definition(org.org_id, environment_id)
        .await
        .log_internal_error_json("get environment before revision")?
        .ok_or_not_found_json("Environment")?;
    if current.is_managed || current.status != "active" {
        return Err(
            ErrorResponse::new("Managed or archived Environments cannot be revised")
                .into_response(StatusCode::CONFLICT),
        );
    }
    Ok(Json(
        state
            .db
            .revise_environment_definition(
                org.org_id,
                environment_id,
                &request.profile,
                request.display_name.as_deref(),
                request
                    .description
                    .as_ref()
                    .map(|description| description.as_deref()),
            )
            .await
            .log_internal_error_json("revise environment")?
            .ok_or_not_found_json("Environment")?,
    ))
}

#[utoipa::path(description = "Archive a user-managed reusable Environment without changing pinned Sessions.", delete, path = "/v1/environments/{environment_id}", params(("environment_id" = String, Path)), responses((status = 200, body = serde_json::Value)), tag = "environments")]
pub async fn archive_environment(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(environment_id): Path<EnvironmentId>,
) -> ApiResult<serde_json::Value> {
    authorize(&state, &org, &crate::domains::harnesses::HARNESS_MANAGE)?;
    let archived = state
        .db
        .archive_environment_definition(org.org_id, environment_id)
        .await
        .log_internal_error_json("archive environment")?;
    if !archived {
        return Err(ErrorResponse::new("Environment not found or managed")
            .into_response(StatusCode::NOT_FOUND));
    }
    Ok(Json(serde_json::json!({"archived": true})))
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

#[cfg(test)]
mod tests {
    use super::ReviseEnvironmentRequest;

    fn request(description: &str) -> ReviseEnvironmentRequest {
        serde_json::from_str(&format!(
            r#"{{{description}"profile":{{"target":{{"kind":"vfs","provider":"bashkit"}}}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn revision_description_distinguishes_omitted_null_and_value() {
        assert_eq!(request("").description, None);
        assert_eq!(request(r#""description":null,"#).description, Some(None));
        assert_eq!(
            request(r#""description":"new","#).description,
            Some(Some("new".to_string()))
        );
    }
}
