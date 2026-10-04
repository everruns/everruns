// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
#[path = "agents/preview.rs"]
pub mod preview;
use preview::preview_agent;
mod responses;
use responses::add_agents_counts;
pub use responses::{
    AgentChannelSummary, AgentHarnessSource, AgentHarnessStatus, AgentHarnessSummary,
    AgentWithCounts,
};

use crate::auth::rate_limit::OrgRateLimiter;
use crate::auth::{AuthState, ResolvedOrg};
use crate::records::Agent;
use crate::records::BuiltInHarnessRole;
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    response::Response,
    routing::{get, post},
};
use everruns_contracts::typed_id::{AgentId, AgentVersionId};
use everruns_core::host::HostComposition;
use everruns_core::{
    Caller, DeploymentGrade, OrgRole, PermissionResolver, ResourceConfigResponse,
    evaluate_policies_with,
};

use super::common::{
    ApiResult, ApiResultExt, ErrorResponse, PaginatedResponse, ResourceStatsResponse, UrlBuilder,
    WithUrls, impl_auth_state,
};
use super::dispatch::{Dispatchable, impl_dispatchable};
use crate::domains::agents::environment::selection_update as environment_update;
use crate::domains::agents::types::{
    AgentAnalysisResponse, CheckAgentNameQuery, CheckAgentNameResponse, CreateAgentRequest,
    CreateAgentVersionRequest, ForkAgentVersionRequest, ImportAgentQuery, ListAgentsQuery,
    PreviewAgentRequest, RollbackAgentVersionRequest, SetDefaultAgentVersionRequest,
    UpdateAgentRequest,
};
use crate::domains::common::Command;
use crate::domains::harnesses::HARNESS_VIEW;
use serde::Deserialize;
use std::sync::Arc;

use crate::domains::agents::{AGENT_DANGEROUS, AGENT_MANAGE, AGENT_VIEW};
use crate::services::CapabilityService;

/// App state for agents routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<crate::storage::EncryptionService>>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
    pub grade: DeploymentGrade,
    pub host_composition: Arc<HostComposition>,
    /// Operator-composed built-in harness templates (EVE-881).
    pub built_in_harnesses: Arc<Vec<crate::records::BuiltInHarnessDefinition>>,
    pub health_check_service: Option<Arc<crate::domains::agents::AgentHealthCheckService>>,
    pub org_rate_limiter: OrgRateLimiter,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<crate::storage::EncryptionService>>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
        grade: DeploymentGrade,
        host_composition: Arc<HostComposition>,
        built_in_harnesses: Arc<Vec<crate::records::BuiltInHarnessDefinition>>,
    ) -> Self {
        Self {
            db,
            encryption,
            capability_service,
            auth,
            grade,
            host_composition,
            built_in_harnesses,
            health_check_service: None,
            org_rate_limiter: OrgRateLimiter::default(),
        }
    }

    pub fn with_health_check_service(
        mut self,
        service: Arc<crate::domains::agents::AgentHealthCheckService>,
    ) -> Self {
        self.health_check_service = Some(service);
        self
    }

    pub fn with_org_rate_limiter(mut self, limiter: OrgRateLimiter) -> Self {
        self.org_rate_limiter = limiter;
        self
    }

    /// Build a domain Ctx from this AppState for the given org.
    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        let mut ctx = crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            self.encryption.clone(),
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
        .with_fallback_harness_name(
            crate::records::harness_for_role(&self.built_in_harnesses, BuiltInHarnessRole::Default)
                .map(|harness| harness.name.clone()),
        )
        .with_utility_llm_service(self.host_composition.utility_llm_service());
        if let Some(service) = &self.health_check_service {
            ctx = ctx.with_health_check_service(service.clone());
        }
        ctx
    }
}

fn require_agent_versions_enabled(
    org: &ResolvedOrg,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if org.feature_flags.agent_versions {
        Ok(())
    } else {
        Err(ErrorResponse::feature_not_enabled("agent_versions"))
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

fn authorize_effective_harness_view(
    resolver: &dyn PermissionResolver,
    caller: &Caller,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    HARNESS_VIEW
        .evaluate_with(resolver, caller)
        .map_err(|error| ErrorResponse::new(error.message).into_response(StatusCode::FORBIDDEN))
}

/// GET /v1/agents/check-name
///
/// Returns whether an agent name is available for use. Optionally excludes
/// a specific agent ID (for edit forms where the agent's own name is valid).
#[utoipa::path(
    get,
    path = "/v1/agents/check-name",
    params(CheckAgentNameQuery),
    responses(
        (status = 200, description = "Name availability result", body = CheckAgentNameResponse),
        (status = 400, description = "Invalid exclude_id", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn check_agent_name(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<CheckAgentNameQuery>,
) -> Result<Json<CheckAgentNameResponse>, (StatusCode, Json<ErrorResponse>)> {
    let result = crate::domains::agents::CheckAgentName {
        name: query.name,
        exclude_id: query.exclude_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(CheckAgentNameResponse {
        available: result.available,
    }))
}

/// GET /v1/agents/config
#[utoipa::path(
    get,
    path = "/v1/agents/config",
    responses(
        (status = 200, description = "Resource config for agents", body = ResourceConfigResponse),
    ),
    tag = "agents"
)]
pub async fn agent_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&AGENT_VIEW, &AGENT_MANAGE, &AGENT_DANGEROUS],
    );
    Json(ResourceConfigResponse { policies })
}

/// Create agent routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/agents", post(create_agent).get(list_agents))
        .route("/v1/agents/check-name", get(check_agent_name))
        .route("/v1/agents/config", get(agent_config))
        .route(
            "/v1/agents/import",
            post(import_agent).layer(DefaultBodyLimit::max(
                everruns_core::agent_package::MAX_PACKAGE_BYTES,
            )),
        )
        .route(
            "/v1/agents/validate",
            post(validate_agent_package).layer(DefaultBodyLimit::max(
                everruns_core::agent_package::MAX_PACKAGE_BYTES,
            )),
        )
        .route(
            "/v1/agents/diff",
            post(diff_agent_package).layer(DefaultBodyLimit::max(
                everruns_core::agent_package::MAX_PACKAGE_BYTES,
            )),
        )
        .route("/v1/agents/preview", post(preview_agent))
        .route("/v1/agents/analyze", post(analyze_agent))
        .route(
            "/v1/agents/{agent_id}/health-checks",
            post(trigger_health_check).get(list_health_checks),
        )
        .route(
            "/v1/agents/{agent_id}/health-checks/latest",
            get(get_latest_health_check),
        )
        .route(
            "/v1/agents/{agent_id}/health-checks/{run_id}",
            get(get_health_check),
        )
        .merge(super::agent_mcp_attachments::routes())
        .route(
            "/v1/agents/{agent_id}",
            get(get_agent)
                .put(upsert_agent)
                .patch(update_agent)
                .delete(delete_agent),
        )
        .route("/v1/agents/{agent_id}/stats", get(get_agent_stats))
        .route("/v1/agents/{agent_id}/delete", post(destroy_agent))
        .route("/v1/agents/{agent_id}/export", get(export_agent))
        .route("/v1/agents/{agent_id}/copy", post(copy_agent))
        .route(
            "/v1/agents/{agent_id}/versions",
            get(list_agent_versions).post(create_agent_version),
        )
        .route(
            "/v1/agents/{agent_id}/versions/default",
            post(set_default_agent_version),
        )
        .route(
            "/v1/agents/{agent_id}/exposures/suspend",
            post(suspend_agent_exposures),
        )
        .route(
            "/v1/agents/{agent_id}/exposures/resume",
            post(resume_agent_exposures),
        )
        .route(
            "/v1/agents/{agent_id}/versions/{version_id}/rollback",
            post(rollback_agent_version),
        )
        .route(
            "/v1/agents/{agent_id}/versions/{version_id}/fork",
            post(fork_agent_version),
        )
        .route(
            "/v1/agents/{agent_id}/versions/{from_version_id}/diff/{to_version_id}",
            get(diff_agent_versions),
        )
        .with_state(state)
}

/// TM-AGENT-005: Reject if any requested capabilities are high-risk and the
/// caller does not have at least Admin role.
pub(crate) fn require_admin_for_high_risk(
    org: &ResolvedOrg,
    caps: &[everruns_contracts::CapabilityRef],
    capability_service: &CapabilityService,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if caps.is_empty() || org.role.has_permission(OrgRole::Admin) {
        return Ok(());
    }
    let refs: Vec<&str> = caps.iter().map(|c| c.capability_id()).collect();
    let high = capability_service.high_risk_ids(&refs);
    if !high.is_empty() {
        return Err(ErrorResponse::new(format!(
            "Admin role required to assign high-risk capabilities: {}",
            high.join(", ")
        ))
        .into_response(StatusCode::FORBIDDEN));
    }
    Ok(())
}

/// POST /v1/agents - Create a new agent
#[utoipa::path(
    post,
    path = "/v1/agents",
    request_body = CreateAgentRequest,
    responses(
        (status = 201, description = "Agent created successfully", body = WithUrls<Agent>),
        (status = 400, description = "Input exceeds allowed limits", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn create_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<WithUrls<Agent>>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created_with_urls(crate::domains::agents::CreateAgent(req))
        .await
}

/// GET /v1/agents - List all active agents
#[utoipa::path(
    get,
    path = "/v1/agents",
    params(ListAgentsQuery),
    responses(
        (status = 200, description = "Paginated list of agents", body = PaginatedResponse<WithUrls<AgentWithCounts>>),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn list_agents(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListAgentsQuery>,
) -> ApiResult<PaginatedResponse<WithUrls<AgentWithCounts>>> {
    let result = crate::domains::agents::ListAgents {
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&org))
    .await?;

    authorize_effective_harness_view(state.auth.permission_resolver.as_ref(), &Caller::from(&org))?;

    let fallback_harness_name =
        crate::records::harness_for_role(&state.built_in_harnesses, BuiltInHarnessRole::Default)
            .map(|harness| harness.name.as_str());
    let data = add_agents_counts(&state.db, org.org_id, result.data, fallback_harness_name).await?;
    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(
        PaginatedResponse::new(data, result.total, result.offset, result.limit).with_urls(&builder),
    ))
}

/// GET /v1/agents/{agent_id} - Get agent by ID or name
///
/// Accepts either an agent ID (e.g. `agent_01933b5a...`) or a
/// name (e.g. `customer-support`). Names are resolved within the caller's org.
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name")
    ),
    responses(
        (status = 200, description = "Agent found", body = WithUrls<AgentWithCounts>),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn get_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id_or_name): Path<String>,
) -> ApiResult<WithUrls<AgentWithCounts>> {
    let agent = crate::domains::agents::GetAgent {
        id: agent_id_or_name,
    }
    .run(&state.ctx(&org))
    .await?;
    authorize_effective_harness_view(state.auth.permission_resolver.as_ref(), &Caller::from(&org))?;
    let fallback_harness_name =
        crate::records::harness_for_role(&state.built_in_harnesses, BuiltInHarnessRole::Default)
            .map(|harness| harness.name.as_str());
    let agent = add_agents_counts(&state.db, org.org_id, vec![agent], fallback_harness_name)
        .await?
        .pop()
        .expect("single agent decoration must return one item");
    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(builder.wrap(agent)))
}

/// GET /v1/agents/{agent_id}/stats - Get aggregate usage stats for an agent
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/stats",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name")
    ),
    responses(
        (status = 200, description = "Agent aggregate stats", body = ResourceStatsResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn get_agent_stats(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id_or_name): Path<String>,
) -> ApiResult<ResourceStatsResponse> {
    let agent = crate::domains::agents::GetAgent {
        id: agent_id_or_name,
    }
    .run(&state.ctx(&org))
    .await?;
    let stats = state
        .db
        .session_aggregate_stats(
            org.org_id,
            Some(AgentId::from_uuid(agent.internal_id)),
            None,
        )
        .await
        .log_internal_error_json("get agent stats")?;

    Ok(Json(stats.into()))
}

/// PATCH /v1/agents/{agent_id} - Update agent
#[utoipa::path(
    patch,
    path = "/v1/agents/{agent_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed, e.g., agt_...)")
    ),
    request_body = UpdateAgentRequest,
    responses(
        (status = 200, description = "Agent updated successfully", body = WithUrls<Agent>),
        (status = 400, description = "Invalid agent ID or input exceeds allowed limits", body = ErrorResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn update_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> ApiResult<WithUrls<Agent>> {
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::agents::UpdateAgentCmd { id: agent_id, req })
        .await
}

/// DELETE /v1/agents/{agent_id} - Archive agent
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed, e.g., agt_...)")
    ),
    responses(
        (status = 204, description = "Agent archived successfully"),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn delete_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::agents::DeleteAgent { id: agent_id })
        .await
}

pub async fn destroy_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::agents::DestroyAgent { id: agent_id })
        .await
}

/// POST /v1/agents/{agent_id}/copy - Copy an agent
///
/// Creates a new agent with the same configuration as the source agent.
/// The new agent's name will be "{original name} (copy)".
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/copy",
    params(
        ("agent_id" = String, Path, description = "Source agent ID to copy")
    ),
    responses(
        (status = 201, description = "Agent copied successfully", body = WithUrls<Agent>),
        (status = 400, description = "Invalid agent ID", body = ErrorResponse),
        (status = 404, description = "Source agent not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn copy_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<(StatusCode, Json<WithUrls<Agent>>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created_with_urls(crate::domains::agents::CopyAgent { id: agent_id })
        .await
}

/// GET /v1/agents/{agent_id}/versions - List saved agent versions
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/versions",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name")
    ),
    responses(
        (status = 200, description = "Saved agent versions", body = Vec<crate::records::AgentVersion>),
        (status = 404, description = "Agent not found or agent_versions disabled", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn list_agent_versions(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<Vec<crate::records::AgentVersion>> {
    require_agent_versions_enabled(&org)?;
    state
        .dispatcher(&org)
        .run(crate::domains::agents::ListAgentVersions { agent_id })
        .await
}

/// POST /v1/agents/{agent_id}/versions - Save the current agent configuration as a version
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/versions",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name")
    ),
    request_body = CreateAgentVersionRequest,
    responses(
        (status = 200, description = "Agent version created", body = crate::records::AgentVersion),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Agent not found or agent_versions disabled", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn create_agent_version(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateAgentVersionRequest>,
) -> ApiResult<crate::records::AgentVersion> {
    require_agent_versions_enabled(&org)?;
    state
        .dispatcher(&org)
        .run(crate::domains::agents::CreateAgentVersionCmd { agent_id, req })
        .await
}

/// POST /v1/agents/{agent_id}/versions/default - Set the default version for an agent
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/versions/default",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name")
    ),
    request_body = SetDefaultAgentVersionRequest,
    responses(
        (status = 200, description = "Default version updated", body = WithUrls<Agent>),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Agent or version not found, or agent_versions disabled", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn set_default_agent_version(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<SetDefaultAgentVersionRequest>,
) -> ApiResult<WithUrls<Agent>> {
    require_agent_versions_enabled(&org)?;
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::agents::SetDefaultAgentVersion { agent_id, req })
        .await
}

/// POST /v1/agents/{agent_id}/exposures/suspend - Take every endpoint offline
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/exposures/suspend",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name")
    ),
    responses(
        (status = 200, description = "Exposures suspended", body = WithUrls<Agent>),
        (status = 404, description = "Agent not found", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn suspend_agent_exposures(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<WithUrls<Agent>> {
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::agents::SuspendAgentExposures { agent_id })
        .await
}

/// POST /v1/agents/{agent_id}/exposures/resume - Let live endpoints serve again
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/exposures/resume",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name")
    ),
    responses(
        (status = 200, description = "Exposures resumed", body = WithUrls<Agent>),
        (status = 404, description = "Agent not found", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn resume_agent_exposures(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<WithUrls<Agent>> {
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::agents::ResumeAgentExposures { agent_id })
        .await
}

/// POST /v1/agents/{agent_id}/versions/{version_id}/rollback - Restore an agent from a saved version
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/versions/{version_id}/rollback",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name"),
        ("version_id" = AgentVersionId, Path, description = "Agent version ID")
    ),
    request_body = RollbackAgentVersionRequest,
    responses(
        (status = 200, description = "Agent rolled back", body = WithUrls<Agent>),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Agent or version not found, or agent_versions disabled", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn rollback_agent_version(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, version_id)): Path<(String, AgentVersionId)>,
    Json(req): Json<RollbackAgentVersionRequest>,
) -> ApiResult<WithUrls<Agent>> {
    require_agent_versions_enabled(&org)?;
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::agents::RollbackAgentVersion {
            agent_id,
            version_id,
            req,
        })
        .await
}

/// POST /v1/agents/{agent_id}/versions/{version_id}/fork - Create a new agent from a saved version
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/versions/{version_id}/fork",
    params(
        ("agent_id" = String, Path, description = "Source agent ID (prefixed) or name"),
        ("version_id" = AgentVersionId, Path, description = "Agent version ID")
    ),
    request_body = ForkAgentVersionRequest,
    responses(
        (status = 200, description = "Agent fork created", body = WithUrls<Agent>),
        (status = 400, description = "Invalid request", body = ErrorResponse),
        (status = 404, description = "Agent or version not found, or agent_versions disabled", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn fork_agent_version(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, version_id)): Path<(String, AgentVersionId)>,
    Json(req): Json<ForkAgentVersionRequest>,
) -> ApiResult<WithUrls<Agent>> {
    require_agent_versions_enabled(&org)?;
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::agents::ForkAgentVersion {
            agent_id,
            version_id,
            req,
        })
        .await
}

/// GET /v1/agents/{agent_id}/versions/{from_version_id}/diff/{to_version_id} - Diff two agent versions
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/versions/{from_version_id}/diff/{to_version_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name"),
        ("from_version_id" = AgentVersionId, Path, description = "Base agent version ID"),
        ("to_version_id" = AgentVersionId, Path, description = "Comparison agent version ID")
    ),
    responses(
        (status = 200, description = "Agent version diff", body = crate::domains::agents::types::AgentVersionDiffResponse),
        (status = 404, description = "Agent or version not found, or agent_versions disabled", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn diff_agent_versions(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, from_version_id, to_version_id)): Path<(
        String,
        AgentVersionId,
        AgentVersionId,
    )>,
) -> ApiResult<crate::domains::agents::types::AgentVersionDiffResponse> {
    require_agent_versions_enabled(&org)?;
    state
        .dispatcher(&org)
        .run(crate::domains::agents::DiffAgentVersions {
            agent_id,
            from_version_id,
            to_version_id,
        })
        .await
}

/// PUT /v1/agents/{agent_id} - Create or update agent (upsert)
///
/// Accepts either an agent ID (e.g. `agent_01933b5a...`) or a
/// name (e.g. `customer-support`). If the agent exists, update it; if not,
/// create it. Returns 201 on create, 200 on update.
#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed) or name")
    ),
    request_body = CreateAgentRequest,
    responses(
        (status = 200, description = "Agent updated", body = WithUrls<Agent>),
        (status = 201, description = "Agent created", body = WithUrls<Agent>),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn upsert_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id_or_name): Path<String>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<WithUrls<Agent>>), (StatusCode, Json<ErrorResponse>)> {
    // Path may be ID or name. For name-based upsert we go through the legacy
    // service path (upsert_by_name uniqueness semantics not yet expressed as
    // a command). ID-based upsert uses the domain command.
    let _caller = Caller::from(&org);
    let (agent, was_created) = if let Ok(agent_id) = agent_id_or_name.parse::<AgentId>() {
        let result = crate::domains::agents::UpsertAgent {
            replace_capabilities: false,
            id: agent_id.to_string(),
            req,
        }
        .run(&state.ctx(&org))
        .await?;
        (result.agent, result.was_created)
    } else {
        // Enforce path name matches body name to prevent ambiguous updates.
        if req.name != agent_id_or_name {
            return Err(
                ErrorResponse::new("Agent name in URL must match name in request body")
                    .into_response(StatusCode::BAD_REQUEST),
            );
        }
        // Name-based upsert: try create, if name taken → update
        let create_result = crate::domains::agents::CreateAgent(req.clone())
            .run(&state.ctx(&org))
            .await;
        match create_result {
            Ok(agent) => (agent, true),
            Err(crate::domains::common::CommandError {
                kind: crate::domains::common::CommandErrorKind::Conflict(_),
                ..
            }) => {
                let existing = crate::domains::agents::queries::get_by_name(
                    &state.db,
                    state.ctx(&org).org_id(),
                    &req.name,
                )
                .await
                .map_err(crate::domains::common::classify_anyhow)?
                .ok_or_else(|| crate::domains::common::CommandError::not_found("Agent"))?;
                let update_req = UpdateAgentRequest {
                    service_virtual_user_id: everruns_durable::UpdateField::Unchanged,

                    name: Some(req.name),
                    display_name: req.display_name,
                    description: req.description,
                    intro_markdown: Some(req.intro_markdown),
                    short_description: Some(req.short_description),
                    starters: Some(req.starters),
                    system_prompt: Some(req.system_prompt),
                    default_model_id: req.default_model_id,
                    harness_id: req.harness_id,
                    harness_name: req.harness_name,
                    tags: Some(req.tags),
                    capabilities: Some(req.capabilities),
                    environments: environment_update(req.environments),
                    initial_files: Some(req.initial_files),
                    tools: Some(req.tools),
                    mcp_servers: Some(req.mcp_servers),
                    network_access: req.network_access,
                    max_iterations: req.max_iterations,
                    parallel_tool_calls: req.parallel_tool_calls,
                    status: None,
                };
                let agent = crate::domains::agents::UpdateAgentCmd {
                    id: existing.public_id.to_string(),
                    req: update_req,
                }
                .run(&state.ctx(&org))
                .await?;
                (agent, false)
            }
            Err(e) => return Err(e.into()),
        }
    };

    let status = if was_created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((status, Json(builder.wrap(agent))))
}

#[derive(Debug, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ExportAgentQuery {
    /// markdown (default), json, yaml, toml, or zip.
    pub format: Option<String>,
}

/// GET /v1/agents/{agent_id}/export - Export agent in Markdown format with YAML front matter
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/export",
    params(
        ("agent_id" = String, Path, description = "Agent name or ID"),
        ExportAgentQuery
    ),
    responses(
        (status = 200, description = "Portable agent definition (format selects text or ZIP)", body = everruns_core::agent_package::Manifest, content_type = "application/json"),
        (status = 400, description = "Invalid agent ID"),
        (status = 404, description = "Agent not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn export_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Query(query): Query<ExportAgentQuery>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let package = crate::domains::agents::packages::export(&state.ctx(&org), &agent_id).await?;
    let (bytes, extension, content_type) = if query.format.as_deref() == Some("zip") {
        (
            package
                .to_zip()
                .map_err(crate::domains::agents::packages::package_error)?,
            "zip",
            "application/zip",
        )
    } else {
        let format =
            crate::domains::agents::packages::format(query.format.as_deref().or(Some("markdown")))?;
        let content_type = match format {
            everruns_core::agent_package::Format::Json => "application/json",
            everruns_core::agent_package::Format::Toml => "application/toml",
            everruns_core::agent_package::Format::Yaml => "application/yaml",
            _ => "text/markdown; charset=utf-8",
        };
        (
            package
                .to_string(format)
                .map_err(crate::domains::agents::packages::package_error)?
                .into_bytes(),
            format.extension(),
            content_type,
        )
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(
            header::CONTENT_DISPOSITION,
            format!(
                "attachment; filename=\"{}.{}\"",
                package.manifest.name, extension
            ),
        )
        .body(Body::from(bytes))
        .expect("valid export response"))
}

/// POST /v1/agents/import - Import agent from file or built-in example
///
/// Two modes:
/// 1. **From example** — `POST /v1/agents/import?from-example={name}` (body ignored)
/// 2. **From package** — text (Markdown/TOML/YAML/JSON), ZIP, or a PackageInput envelope.
///
/// Package mode accepts:
/// - Markdown with YAML front matter (if starts with ---)
/// - Pure YAML
/// - Pure JSON
/// - Plain text (treated as instructions; defaults to name agent)
/// - TOML and ZIP folders, with complete assets and skill directories
///
/// If the file contains an `id` field and an agent with that ID already exists,
/// the agent is updated (upsert). Returns 201 on create, 200 on update.
#[utoipa::path(
    post,
    path = "/v1/agents/import",
    params(ImportAgentQuery),
    request_body(content = String, content_type = "text/plain"),
    responses(
        (status = 200, description = "Agent updated via import", body = WithUrls<Agent>),
        (status = 201, description = "Agent imported successfully", body = WithUrls<Agent>),
        (status = 400, description = "Invalid format or input exceeds limits", body = ErrorResponse),
        (status = 404, description = "Example not found", body = ErrorResponse),
        (status = 403, description = "High-risk capabilities require admin role", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn import_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ImportAgentQuery>,
    body: Bytes,
) -> Result<(StatusCode, Json<WithUrls<Agent>>), (StatusCode, Json<ErrorResponse>)> {
    // Branch: import from built-in example
    if let Some(name) = &query.from_example {
        return import_from_example(org, &state, name).await;
    }

    // Branch: import from file body
    let ctx = state.ctx(&org);
    let input = package_body(&body, &query)?;
    let package = crate::domains::agents::packages::parse_input(&ctx, &input).await?;
    let target = input.target;
    let (agent, created) =
        crate::domains::agents::packages::apply(&ctx, &package, target.as_deref()).await?;
    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((
        if created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(builder.wrap(agent)),
    ))
}

/// Import an agent from a built-in example by name.
async fn import_from_example(
    org: ResolvedOrg,
    state: &AppState,
    name: &str,
) -> Result<(StatusCode, Json<WithUrls<Agent>>), (StatusCode, Json<ErrorResponse>)> {
    let seed = crate::agent_templates::find_agent_example(name)
        .ok_or_else(|| ErrorResponse::not_found(&format!("agent example '{name}'")))?;

    // Check dev-only
    if seed.dev_only && !state.grade.experimental_features_enabled() {
        return Err(ErrorResponse::not_found(&format!("agent example '{name}'")));
    }

    // Check capabilities registered
    let missing: Vec<&str> = seed
        .capabilities
        .iter()
        .map(|c| c.id)
        .filter(|id| !state.host_composition.capability_registry().has(id))
        .collect();
    if !missing.is_empty() {
        return Err(ErrorResponse::new(format!(
            "Example requires unregistered capabilities: {missing:?}"
        ))
        .into_response(StatusCode::BAD_REQUEST));
    }

    let capabilities: Vec<everruns_contracts::CapabilityRef> = seed
        .capabilities
        .iter()
        .map(|cap| {
            let config = cap.config.map_or_else(|| serde_json::json!({}), |f| f());
            everruns_contracts::CapabilityRef::with_config(cap.id.to_string(), config)
        })
        .collect();

    // TM-AGENT-005: High-risk capabilities require admin role
    require_admin_for_high_risk(&org, &capabilities, &state.capability_service)?;

    let _caller = Caller::from(&org);

    // If an agent with the same name exists, keep trying suffixed variants
    // to avoid one-shot collisions and reduce failures under concurrency.
    let unique_name = {
        use rand::RngExt;

        let base = seed.name;
        let mut selected = None;

        for attempt in 0..10 {
            let candidate = if attempt == 0 {
                base.to_string()
            } else {
                let suffix: String = rand::rng()
                    .sample_iter(&rand::distr::Alphanumeric)
                    .take(5)
                    .map(|c| (c as char).to_ascii_lowercase())
                    .collect();
                format!("{base}-{suffix}")
            };

            let result = crate::domains::agents::CheckAgentName {
                name: candidate.clone(),
                exclude_id: None,
            }
            .run(&state.ctx(&org))
            .await?;
            let available = result.available;

            if available {
                selected = Some(candidate);
                break;
            }
        }

        selected.ok_or_else(ErrorResponse::internal_error)?
    };

    let req = CreateAgentRequest {
        service_virtual_user_id: None,

        id: None,
        name: unique_name,
        display_name: Some(seed.display_name.to_string()),
        description: Some(seed.description.to_string()),
        intro_markdown: None,
        short_description: None,
        starters: Vec::new(),
        system_prompt: seed.system_prompt.to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: Some(seed.harness_name.to_string()),
        tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        capabilities,
        environments: None,
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    };

    let agent = crate::domains::agents::CreateAgent(req)
        .run(&state.ctx(&org))
        .await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(builder.wrap(agent))))
}

/// Validate an agent package and its destination dependencies without mutation.
#[utoipa::path(post, path = "/v1/agents/validate", params(ImportAgentQuery), request_body = crate::domains::agents::packages::PackageInput, responses((status = 200, description = "Validation diagnostics", body = serde_json::Value)), tag = "agents")]
pub async fn validate_agent_package(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ImportAgentQuery>,
    body: Bytes,
) -> ApiResult<serde_json::Value> {
    let input = match package_body(&body, &query) {
        Ok(input) => input,
        Err(error) => {
            return Ok(Json(
                serde_json::json!({"valid":false,"diagnostics":[{"path":"package","message":error.message()}]}),
            ));
        }
    };
    Ok(Json(
        crate::domains::agents::ValidateAgentPackage(input)
            .run(&state.ctx(&org))
            .await?,
    ))
}

/// Generate a semantic diff against an existing agent without mutation.
#[utoipa::path(post, path = "/v1/agents/diff", params(ImportAgentQuery), request_body = crate::domains::agents::packages::PackageInput, responses((status = 200, description = "Semantic changes", body = serde_json::Value)), tag = "agents")]
pub async fn diff_agent_package(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ImportAgentQuery>,
    body: Bytes,
) -> ApiResult<serde_json::Value> {
    let input = package_body(&body, &query)?;
    Ok(Json(
        crate::domains::agents::DiffAgentPackage(input)
            .run(&state.ctx(&org))
            .await?,
    ))
}

fn package_body(
    body: &[u8],
    query: &ImportAgentQuery,
) -> Result<crate::domains::agents::packages::PackageInput, crate::domains::common::CommandError> {
    use crate::domains::{
        agents::packages::{PackageInput, package_error},
        common::CommandError,
    };
    if query.format.as_deref() == Some("zip") || body.starts_with(b"PK\x03\x04") {
        let package =
            everruns_core::agent_package::AgentPackage::from_zip(body).map_err(package_error)?;
        return Ok(PackageInput {
            content: package
                .to_string(everruns_core::agent_package::Format::Json)
                .map_err(package_error)?,
            file: None,
            format: Some("json".into()),
            target: query.target.clone(),
        });
    }
    let content = std::str::from_utf8(body)
        .map_err(|_| CommandError::bad_request("Agent definition must be UTF-8 or ZIP"))?;
    if let Ok(mut input) = serde_json::from_str::<PackageInput>(content)
        && (!input.content.is_empty() || input.file.is_some())
    {
        input.target = query.target.clone().or(input.target);
        return Ok(input);
    }
    Ok(PackageInput {
        content: content.into(),
        file: None,
        format: query.format.clone(),
        target: query.target.clone(),
    })
}

/// POST /v1/agents/analyze - Run advisory checks against an agent shape
///
/// Runs built-in rules plus on-demand LLM analysis (knowledge/evaluation/agent-checks.md)
/// and returns merged advisory findings. Requires the system utility LLM
/// service to be configured.
#[utoipa::path(
    post,
    path = "/v1/agents/analyze",
    request_body = PreviewAgentRequest,
    responses(
        (status = 200, description = "Agent analysis completed", body = AgentAnalysisResponse),
        (status = 400, description = "Utility LLM service not configured", body = ErrorResponse),
        (status = 422, description = "Utility LLM provider rejected the analysis", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn analyze_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<PreviewAgentRequest>,
) -> ApiResult<AgentAnalysisResponse> {
    let result = crate::domains::agents::AnalyzeAgent {
        harness_id: req.harness_id,
        initial_files: req.initial_files,
        system_prompt: Some(req.system_prompt),
        capabilities: req.capabilities,
        tools: req.tools,
        mcp_servers: req.mcp_servers,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(AgentAnalysisResponse {
        findings: result.findings,
    }))
}

/// POST /v1/agents/{agent_id}/health-checks - Trigger a behavioral health check
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/health-checks",
    params(("agent_id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Health check run started", body = crate::domains::agents::health_check::types::HealthCheckRun),
        (status = 400, description = "Health checks unavailable on this deployment", body = ErrorResponse),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn trigger_health_check(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<crate::domains::agents::health_check::types::HealthCheckRun> {
    let ctx = state.ctx(&org);

    // Authorize *before* consuming the org's session-create rate budget.
    // Otherwise a caller lacking AGENT_HEALTH_CHECK_RUN (a more restrictive
    // policy than plain session creation) would be rejected by the command yet
    // still burn the org's quota, starving legitimate session creation — a DoS.
    // The command re-checks the policy in `run`; paying it twice is cheap and
    // keeps the command the single source of truth.
    if let Some(policy) =
        crate::domains::agents::health_check::commands::TriggerAgentHealthCheck::policy()
    {
        policy
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|e| crate::domains::common::CommandError::forbidden(e.message))?;
    }

    if state
        .org_rate_limiter
        .check_session_create(org.org_id)
        .await
        .is_err()
    {
        return Err(
            ErrorResponse::new("Too many requests. Please try again later.")
                .with_code("rate_limited")
                .with_retry_after(60)
                .into_response(StatusCode::TOO_MANY_REQUESTS),
        );
    }

    let run = crate::domains::agents::health_check::commands::TriggerAgentHealthCheck { agent_id }
        .run(&ctx)
        .await?;
    Ok(Json(run))
}

/// GET /v1/agents/{agent_id}/health-checks - List recent health check runs
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/health-checks",
    params(("agent_id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Health check runs", body = Vec<crate::domains::agents::health_check::types::HealthCheckRun>),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn list_health_checks(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<Vec<crate::domains::agents::health_check::types::HealthCheckRun>> {
    let runs =
        crate::domains::agents::health_check::commands::ListAgentHealthCheckRuns { agent_id }
            .run(&state.ctx(&org))
            .await?;
    Ok(Json(runs))
}

/// GET /v1/agents/{agent_id}/health-checks/{run_id} - Get a health check run
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/health-checks/{run_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID"),
        ("run_id" = String, Path, description = "Health check run ID")
    ),
    responses(
        (status = 200, description = "Health check run", body = crate::domains::agents::health_check::types::HealthCheckRun),
        (status = 404, description = "Run not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn get_health_check(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((agent_id, run_id)): Path<(String, String)>,
) -> ApiResult<crate::domains::agents::health_check::types::HealthCheckRun> {
    let run =
        crate::domains::agents::health_check::commands::GetAgentHealthCheckRun { agent_id, run_id }
            .run(&state.ctx(&org))
            .await?;
    Ok(Json(run))
}

/// GET /v1/agents/{agent_id}/health-checks/latest - Latest run + stale flag
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/health-checks/latest",
    params(("agent_id" = String, Path, description = "Agent ID")),
    responses(
        (status = 200, description = "Latest health check run with stale-config flag", body = crate::domains::agents::health_check::types::LatestHealthCheckRun),
        (status = 404, description = "Agent not found", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn get_latest_health_check(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<crate::domains::agents::health_check::types::LatestHealthCheckRun> {
    let result =
        crate::domains::agents::health_check::commands::GetLatestAgentHealthCheckRun { agent_id }
            .run(&state.ctx(&org))
            .await?;
    Ok(Json(result))
}

// Regression tests for fix(capabilities): restore high-risk levels for
// bash/fetch (#1500). `require_admin_for_high_risk` is the HTTP-side gate
// that enforces TM-AGENT-005: a member cannot assign `bashkit_shell` or
// `web_fetch` to an agent. The fix re-classified both capabilities as
// High; without that classification this gate silently becomes a no-op
// for the two most dangerous capabilities.
#[cfg(test)]
#[path = "agents/high_risk_admin_gate_tests.rs"]
mod high_risk_admin_gate_tests;
