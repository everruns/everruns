// Sandbox Template and primary Session Sandbox HTTP routes.
//
// A Sandbox Template describes where commands run plus what they may touch.
// New Sessions return their immutable, resolved specification snapshot. Legacy
// sessions retain a capability-derived compatibility view; `resolved_from`
// makes the distinction explicit.
//
// See knowledge/harnesses/sandbox-templates.md.

pub use crate::domains::sandbox_templates::types::{
    SandboxCapabilities, SandboxContainment, SandboxTarget, SandboxTargetDescriptor,
    SandboxTargetsResponse, SessionSandboxResponse,
};
use std::sync::Arc;

use axum::{Json, Router, extract::Path, extract::State, http::StatusCode, routing::get};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::command_http::CommandRouterExt;
use crate::api::dispatch::impl_dispatchable;
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::sandbox_templates::commands::{GetSessionSandbox, ListSandboxTargets};
use crate::domains::sandboxes::{
    GetSandbox, GetSandboxFleetStats, GetSandboxTimeline, ListSandboxes,
};
use crate::domains::sessions::SessionService;
use crate::storage::StorageBackend;
use everruns_contracts::typed_id::SandboxTemplateId;
use everruns_core::Caller;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse, impl_auth_state};

const MAX_SANDBOX_TEMPLATE_DESCRIPTION_BYTES: usize = 10 * 1024;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateSandboxTemplateRequest {
    /// Stable addressable name used by Agent Sandbox policies.
    #[schema(example = "coding-daytona")]
    pub name: String,
    /// Human-readable name shown in management surfaces.
    #[schema(example = "Coding - Daytona")]
    pub display_name: String,
    /// Optional explanation of the Sandbox Template's intended workload.
    #[serde(default)]
    #[schema(example = "Recoverable coding workspace managed by Daytona")]
    pub description: Option<String>,
    /// Initial immutable Sandbox specification revision.
    #[serde(alias = "profile")]
    pub spec: crate::records::SandboxTemplateSpec,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ReviseSandboxTemplateRequest {
    /// Replacement display name; omit to preserve the current value.
    #[serde(default)]
    #[schema(example = "Coding - Daytona (large)")]
    pub display_name: Option<String>,
    /// Omit to preserve; send null to clear.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(example = "Larger recoverable workspace for repository builds")]
    pub description: Option<Option<String>>,
    /// Complete specification stored as the next immutable revision.
    #[serde(alias = "profile")]
    pub spec: crate::records::SandboxTemplateSpec,
}

fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

async fn validate_organization_connection(
    state: &AppState,
    org_id: i64,
    spec: &crate::records::SandboxTemplateSpec,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    use everruns_contracts::session_sandbox::SessionSandboxCredentialSource;
    if spec.target.credential.source != SessionSandboxCredentialSource::Organization {
        return Ok(());
    }
    let connection_id = spec.target.credential.connection_id.ok_or_else(|| {
        ErrorResponse::new("Organization Sandbox credentials require an account selection")
            .into_response(StatusCode::UNPROCESSABLE_ENTITY)
    })?;
    let connection = state
        .db
        .get_organization_connection(org_id, connection_id)
        .await
        .log_internal_error_json("validate Sandbox Template organization connection")?
        .ok_or_else(|| {
            ErrorResponse::new("Organization connection not found")
                .into_response(StatusCode::UNPROCESSABLE_ENTITY)
        })?;
    if connection.provider != spec.target.provider.as_deref().unwrap_or("") {
        return Err(ErrorResponse::new(
            "Organization connection does not match the Sandbox provider",
        )
        .into_response(StatusCode::UNPROCESSABLE_ENTITY));
    }
    Ok(())
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
        // Sandbox Templates are a core surface, not feature-gated.
        .with_feature_flags(org.feature_flags.clone())
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        // Org-wide Sandbox fleet (domains/sandboxes).
        .command::<ListSandboxes>()
        .command::<GetSandboxFleetStats>()
        .command::<GetSandboxTimeline>()
        .command::<GetSandbox>()
        .route(
            "/v1/sandbox-templates",
            get(list_sandbox_templates).post(create_sandbox_template),
        )
        .route(
            "/v1/sandbox-templates/{sandbox_template_id}",
            get(get_sandbox_template)
                .put(revise_sandbox_template)
                .delete(archive_sandbox_template),
        )
        .route(
            "/v1/sessions/{session_id}/sandbox",
            get(get_session_sandbox),
        )
        .route("/v1/sandbox-targets", get(list_sandbox_targets))
        // Compatibility aliases for clients released before the resource rename.
        .route(
            "/v1/environments",
            get(list_sandbox_templates).post(create_sandbox_template),
        )
        .route(
            "/v1/environments/{sandbox_template_id}",
            get(get_sandbox_template)
                .put(revise_sandbox_template)
                .delete(archive_sandbox_template),
        )
        .route(
            "/v1/sessions/{session_id}/environment",
            get(get_session_sandbox),
        )
        .route("/v1/environment-targets", get(list_sandbox_targets))
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

#[utoipa::path(description = "List active reusable Sandbox Templates available to the organization.", get, path = "/v1/sandbox-templates", responses((status = 200, body = Vec<crate::records::SandboxTemplate>)), tag = "sandbox-templates")]
pub async fn list_sandbox_templates(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<Vec<crate::records::SandboxTemplate>> {
    authorize(&state, &org, &crate::domains::harnesses::HARNESS_VIEW)?;
    Ok(Json(
        state
            .db
            .list_sandbox_templates(org.org_id, false)
            .await
            .log_internal_error_json("list Sandbox Templates")?,
    ))
}

#[utoipa::path(description = "Create a reusable Sandbox Template and its first immutable revision.", post, path = "/v1/sandbox-templates", request_body = CreateSandboxTemplateRequest, responses((status = 200, body = crate::records::SandboxTemplate)), tag = "sandbox-templates")]
pub async fn create_sandbox_template(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(request): Json<CreateSandboxTemplateRequest>,
) -> ApiResult<crate::records::SandboxTemplate> {
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
        .is_some_and(|description| description.len() > MAX_SANDBOX_TEMPLATE_DESCRIPTION_BYTES)
    {
        return Err(
            ErrorResponse::new("description must be at most 10240 bytes")
                .into_response(StatusCode::UNPROCESSABLE_ENTITY),
        );
    }
    if request.spec.template_revision_id.is_some() {
        return Err(ErrorResponse::new(
            "Reusable Sandbox Template specs cannot set template_revision_id",
        )
        .into_response(StatusCode::UNPROCESSABLE_ENTITY));
    }
    crate::domains::sandbox_templates::resolution::resolve_spec(&request.spec).map_err(
        |error| ErrorResponse::new(error).into_response(StatusCode::UNPROCESSABLE_ENTITY),
    )?;
    validate_organization_connection(&state, org.org_id, &request.spec).await?;
    if state
        .db
        .list_sandbox_templates(org.org_id, false)
        .await
        .log_internal_error_json("check Sandbox Template name")?
        .iter()
        .any(|template| template.name == request.name)
    {
        return Err(
            ErrorResponse::new("An active Sandbox Template already uses this name")
                .into_response(StatusCode::CONFLICT),
        );
    }
    let template = state
        .db
        .create_sandbox_template(
            org.org_id,
            &request.name,
            request.display_name.trim(),
            request.description.as_deref(),
            &request.spec,
            false,
        )
        .await
        .log_internal_error_json("create Sandbox Template")?;
    Ok(Json(template))
}

#[utoipa::path(description = "Get one reusable Sandbox Template and its current immutable revision.", get, path = "/v1/sandbox-templates/{sandbox_template_id}", params(("sandbox_template_id" = String, Path)), responses((status = 200, body = crate::records::SandboxTemplate)), tag = "sandbox-templates")]
pub async fn get_sandbox_template(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(sandbox_template_id): Path<SandboxTemplateId>,
) -> ApiResult<crate::records::SandboxTemplate> {
    authorize(&state, &org, &crate::domains::harnesses::HARNESS_VIEW)?;
    Ok(Json(
        state
            .db
            .get_sandbox_template(org.org_id, sandbox_template_id)
            .await
            .log_internal_error_json("get Sandbox Template")?
            .ok_or_not_found_json("Sandbox Template")?,
    ))
}

#[utoipa::path(description = "Create the next immutable revision of a reusable Sandbox Template.", put, path = "/v1/sandbox-templates/{sandbox_template_id}", params(("sandbox_template_id" = String, Path)), request_body = ReviseSandboxTemplateRequest, responses((status = 200, body = crate::records::SandboxTemplate)), tag = "sandbox-templates")]
pub async fn revise_sandbox_template(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(sandbox_template_id): Path<SandboxTemplateId>,
    Json(request): Json<ReviseSandboxTemplateRequest>,
) -> ApiResult<crate::records::SandboxTemplate> {
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
        .is_some_and(|description| description.len() > MAX_SANDBOX_TEMPLATE_DESCRIPTION_BYTES)
    {
        return Err(
            ErrorResponse::new("description must be at most 10240 bytes")
                .into_response(StatusCode::UNPROCESSABLE_ENTITY),
        );
    }
    if request.spec.template_revision_id.is_some() {
        return Err(ErrorResponse::new(
            "Reusable Sandbox Template specs cannot set template_revision_id",
        )
        .into_response(StatusCode::UNPROCESSABLE_ENTITY));
    }
    crate::domains::sandbox_templates::resolution::resolve_spec(&request.spec).map_err(
        |error| ErrorResponse::new(error).into_response(StatusCode::UNPROCESSABLE_ENTITY),
    )?;
    validate_organization_connection(&state, org.org_id, &request.spec).await?;
    let current = state
        .db
        .get_sandbox_template(org.org_id, sandbox_template_id)
        .await
        .log_internal_error_json("get Sandbox Template before revision")?
        .ok_or_not_found_json("Sandbox Template")?;
    if current.is_managed || current.status != "active" {
        return Err(
            ErrorResponse::new("Managed or archived Sandbox Templates cannot be revised")
                .into_response(StatusCode::CONFLICT),
        );
    }
    Ok(Json(
        state
            .db
            .revise_sandbox_template(
                org.org_id,
                sandbox_template_id,
                &request.spec,
                request.display_name.as_deref(),
                request
                    .description
                    .as_ref()
                    .map(|description| description.as_deref()),
            )
            .await
            .log_internal_error_json("revise Sandbox Template")?
            .ok_or_not_found_json("Sandbox Template")?,
    ))
}

#[utoipa::path(description = "Archive a user-managed Sandbox Template without changing pinned Sessions.", delete, path = "/v1/sandbox-templates/{sandbox_template_id}", params(("sandbox_template_id" = String, Path)), responses((status = 200, body = serde_json::Value)), tag = "sandbox-templates")]
pub async fn archive_sandbox_template(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(sandbox_template_id): Path<SandboxTemplateId>,
) -> ApiResult<serde_json::Value> {
    authorize(&state, &org, &crate::domains::harnesses::HARNESS_MANAGE)?;
    let archived = state
        .db
        .archive_sandbox_template(org.org_id, sandbox_template_id)
        .await
        .log_internal_error_json("archive Sandbox Template")?;
    if !archived {
        return Err(ErrorResponse::new("Sandbox Template not found or managed")
            .into_response(StatusCode::NOT_FOUND));
    }
    Ok(Json(serde_json::json!({"archived": true})))
}

#[utoipa::path(
    description = "Get the primary Sandbox a Session runs in: template, target, containment, and capabilities.",
    get,
    path = "/v1/sessions/{session_id}/sandbox",
    params(
        ("session_id" = String, Path, description = "Session ID")
    ),
    responses(
        (
            status = 200,
            description = "Resolved primary Session Sandbox",
            body = SessionSandboxResponse,
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
    tag = "sandboxes"
)]
pub async fn get_session_sandbox(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> ApiResult<SessionSandboxResponse> {
    Ok(Json(
        GetSessionSandbox { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "List the Sandbox targets this deployment can offer, with the capabilities each one actually has.",
    get,
    path = "/v1/sandbox-targets",
    responses(
        (status = 200, description = "Available Sandbox targets", body = SandboxTargetsResponse),
    ),
    tag = "sandbox-templates"
)]
pub async fn list_sandbox_targets(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<SandboxTargetsResponse> {
    Ok(Json(ListSandboxTargets.run(&state.ctx(&org)).await?))
}

#[cfg(test)]
mod tests {
    use super::ReviseSandboxTemplateRequest;

    fn request(description: &str) -> ReviseSandboxTemplateRequest {
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
