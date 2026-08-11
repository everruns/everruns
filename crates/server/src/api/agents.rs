// Agent CRUD HTTP routes (M2)
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

use crate::auth::rate_limit::OrgRateLimiter;
use crate::auth::{AuthState, ResolvedOrg};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::Response,
    routing::{get, post},
};
use chrono::Utc;
use everruns_core::typed_id::{AgentId, AgentVersionId, HarnessId, ModelId};
use everruns_core::{
    AgentCapabilityConfig, Caller, DeploymentGrade, InitialFile, OrgRole, PlatformDefinition,
    ResourceConfigResponse, ScopedMcpServers, evaluate_policies_with,
};
use everruns_platform::Agent;
use everruns_platform::BuiltInHarnessRole;
use futures::future::try_join_all;

use super::common::{
    ApiResult, ApiResultExt, ErrorResponse, PaginatedResponse, ResourceStatsResponse,
    ResourceUrlable, UrlBuilder, WithUrls, impl_auth_state,
};
use super::dispatch::{Dispatchable, impl_dispatchable};
use super::validation::{
    validate_agent_name_format, validate_create_agent_input, validate_import_file_size,
};
use crate::domains::agents::types::{
    AgentAnalysisResponse, AgentPreviewResponse, CheckAgentNameQuery, CheckAgentNameResponse,
    CreateAgentRequest, CreateAgentVersionRequest, ForkAgentVersionRequest, ImportAgentQuery,
    ListAgentsQuery, PreviewAgentRequest, RollbackAgentVersionRequest,
    SetDefaultAgentVersionRequest, UpdateAgentRequest,
};
use crate::domains::common::Command;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;

/// Capability entry in agent file - supports both string and object formats
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum AgentFileCapability {
    /// Legacy format: just capability ID string
    Simple(String),
    /// New format: object with ref and config
    WithConfig {
        #[serde(rename = "ref")]
        capability_ref: String,
        #[serde(default)]
        config: serde_json::Value,
    },
}

impl AgentFileCapability {
    fn to_agent_capability_config(&self) -> AgentCapabilityConfig {
        match self {
            AgentFileCapability::Simple(id) => AgentCapabilityConfig::new(id.clone()),
            AgentFileCapability::WithConfig {
                capability_ref,
                config,
            } => AgentCapabilityConfig::with_config(capability_ref.clone(), config.clone()),
        }
    }
}

/// Entry in agent file initial_files - supports both string (glob pattern) and
/// object (fully-specified file) formats. String entries are glob patterns that
/// must be expanded by the CLI before sending to the server; the server silently
/// drops them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum AgentFileInitialFile {
    /// Glob pattern (e.g. ".", ".agents/*") - CLI-only, server ignores
    GlobPattern(String),
    /// Fully-specified file with content
    File(InitialFile),
}

impl AgentFileInitialFile {
    fn into_initial_file(self) -> Option<InitialFile> {
        match self {
            AgentFileInitialFile::GlobPattern(_) => None,
            AgentFileInitialFile::File(f) => Some(f),
        }
    }
}

/// Agent file format for import (matches CLI format)
/// Parsed from YAML front matter in Markdown files.
/// Supports both legacy (string list) and new (object with ref/config) capability formats.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AgentFile {
    /// Optional agent ID (format: agent_{32-hex}). Preserved during import/export.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<AgentId>,
    /// Name (e.g. "customer-support"). If absent, derived from display_name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Human-readable display name. Falls back to name if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<ModelId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_id: Option<HarnessId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness_name: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Capabilities - supports both string IDs and objects with ref/config
    #[serde(default)]
    pub capabilities: Vec<AgentFileCapability>,
    /// Initial files - supports string globs (CLI-only, stripped by server) and
    /// fully-specified InitialFile objects.
    #[serde(default)]
    pub initial_files: Vec<AgentFileInitialFile>,
    #[serde(default, rename = "mcpServers", alias = "mcp_servers")]
    pub mcp_servers: ScopedMcpServers,
}

use crate::domains::agents::{AGENT_DANGEROUS, AGENT_MANAGE, AGENT_VIEW};
use crate::services::CapabilityService;

/// App state for agents routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
    pub grade: DeploymentGrade,
    pub platform_definition: Arc<PlatformDefinition>,
    /// Operator-composed built-in harness templates (EVE-881).
    pub built_in_harnesses: Arc<Vec<everruns_platform::BuiltInHarnessDefinition>>,
    pub health_check_service: Option<Arc<crate::domains::agents::AgentHealthCheckService>>,
    pub org_rate_limiter: OrgRateLimiter,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
        grade: DeploymentGrade,
        platform_definition: Arc<PlatformDefinition>,
        built_in_harnesses: Arc<Vec<everruns_platform::BuiltInHarnessDefinition>>,
    ) -> Self {
        Self {
            db,
            capability_service,
            auth,
            grade,
            platform_definition,
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
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
        .with_fallback_harness_name(
            everruns_platform::harness_for_role(
                &self.built_in_harnesses,
                BuiltInHarnessRole::Default,
            )
            .map(|harness| harness.name.clone()),
        )
        .with_utility_llm_service(self.platform_definition.utility_llm_service());
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

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentHarnessSource {
    Explicit,
    OrganizationDefault,
}

#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentHarnessStatus {
    Active,
    Archived,
    Deleted,
    Unresolved,
}

/// Harness that a newly created session for this agent will resolve to.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentHarnessSummary {
    #[schema(value_type = Option<String>)]
    pub id: Option<HarnessId>,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub source: AgentHarnessSource,
    pub status: AgentHarnessStatus,
}

/// Agent list/detail payload with relationship counts and resolved harness metadata.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentWithCounts {
    pub session_count: u64,
    pub app_count: u64,
    pub effective_harness: AgentHarnessSummary,
    #[serde(flatten)]
    pub inner: Agent,
}

impl ResourceUrlable for AgentWithCounts {
    fn api_path() -> &'static str {
        Agent::api_path()
    }

    fn ui_path() -> &'static str {
        Agent::ui_path()
    }

    fn resource_id(&self) -> String {
        self.inner.resource_id()
    }

    fn allowed_actions(&self, api_base: &str) -> Vec<super::common::AllowedAction> {
        self.inner.allowed_actions(api_base)
    }
}

fn harness_source(value: &str) -> AgentHarnessSource {
    if value == "organization_default" {
        AgentHarnessSource::OrganizationDefault
    } else {
        AgentHarnessSource::Explicit
    }
}

fn harness_status(value: &str) -> AgentHarnessStatus {
    match value {
        "active" => AgentHarnessStatus::Active,
        "archived" => AgentHarnessStatus::Archived,
        "deleted" => AgentHarnessStatus::Deleted,
        _ => AgentHarnessStatus::Unresolved,
    }
}

async fn resolve_agent_harnesses(
    db: &StorageBackend,
    org_id: i64,
    agents: &[Agent],
    fallback_harness_name: Option<&str>,
) -> Result<HashMap<uuid::Uuid, AgentHarnessSummary>, (StatusCode, Json<ErrorResponse>)> {
    let agent_ids: Vec<AgentId> = agents
        .iter()
        .map(|agent| AgentId::from_uuid(agent.internal_id))
        .collect();
    let rows = db
        .get_agents_by_ids(org_id, &agent_ids)
        .await
        .log_internal_error_json("load agent harness bindings")?;

    let inherited_harness_id = if rows
        .iter()
        .any(|row| row.harness_source == "organization_default")
    {
        crate::domains::sessions::queries::resolve_session_harness_id(
            db,
            org_id,
            None,
            None,
            fallback_harness_name,
        )
        .await
        .ok()
    } else {
        None
    };

    let effective_ids: Vec<HarnessId> = rows
        .iter()
        .filter_map(|row| {
            if row.harness_source == "organization_default" {
                inherited_harness_id
            } else {
                Some(row.harness_id)
            }
        })
        .collect();
    let harnesses = if effective_ids.is_empty() {
        Vec::new()
    } else {
        db.get_harness_ancestry_by_ids(org_id, &effective_ids)
            .await
            .log_internal_error_json("load effective agent harnesses")?
    };
    let harnesses_by_id: HashMap<HarnessId, _> = harnesses
        .into_iter()
        .map(|harness| (harness.id, harness))
        .collect();

    Ok(rows
        .into_iter()
        .map(|row| {
            let source = harness_source(&row.harness_source);
            let effective_id = match source {
                AgentHarnessSource::Explicit => Some(row.harness_id),
                AgentHarnessSource::OrganizationDefault => inherited_harness_id,
            };
            let summary = effective_id
                .and_then(|id| harnesses_by_id.get(&id).map(|harness| (id, harness)))
                .map(|(id, harness)| AgentHarnessSummary {
                    id: Some(id),
                    name: Some(harness.name.clone()),
                    display_name: harness.display_name.clone(),
                    source,
                    status: harness_status(&harness.status),
                })
                .unwrap_or(AgentHarnessSummary {
                    id: effective_id,
                    name: None,
                    display_name: None,
                    source,
                    status: AgentHarnessStatus::Unresolved,
                });
            (row.id.uuid(), summary)
        })
        .collect())
}

async fn add_agent_counts(
    db: &StorageBackend,
    org_id: i64,
    agent: Agent,
    effective_harness: AgentHarnessSummary,
) -> Result<AgentWithCounts, (StatusCode, Json<ErrorResponse>)> {
    let agent_id = AgentId::from_uuid(agent.internal_id);
    let session_count = async {
        db.count_sessions_for_agent(org_id, agent_id)
            .await
            .log_internal_error_json("count agent sessions")
    };
    let app_count = async {
        db.count_apps_for_agent(org_id, agent_id)
            .await
            .log_internal_error_json("count agent apps")
    };
    let (session_count, app_count) = tokio::try_join!(session_count, app_count)?;

    Ok(AgentWithCounts {
        session_count,
        app_count,
        effective_harness,
        inner: agent,
    })
}

async fn add_agents_counts(
    db: &StorageBackend,
    org_id: i64,
    agents: Vec<Agent>,
    fallback_harness_name: Option<&str>,
) -> Result<Vec<AgentWithCounts>, (StatusCode, Json<ErrorResponse>)> {
    let mut harnesses = resolve_agent_harnesses(db, org_id, &agents, fallback_harness_name).await?;
    try_join_all(agents.into_iter().map(|agent| {
        let effective_harness =
            harnesses
                .remove(&agent.internal_id)
                .unwrap_or(AgentHarnessSummary {
                    id: None,
                    name: None,
                    display_name: None,
                    source: AgentHarnessSource::Explicit,
                    status: AgentHarnessStatus::Unresolved,
                });
        add_agent_counts(db, org_id, agent, effective_harness)
    }))
    .await
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
        .route("/v1/agents/import", post(import_agent))
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
    caps: &[AgentCapabilityConfig],
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

    let fallback_harness_name =
        everruns_platform::harness_for_role(&state.built_in_harnesses, BuiltInHarnessRole::Default)
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
    let fallback_harness_name =
        everruns_platform::harness_for_role(&state.built_in_harnesses, BuiltInHarnessRole::Default)
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
        (status = 200, description = "Saved agent versions", body = Vec<everruns_platform::AgentVersion>),
        (status = 404, description = "Agent not found or agent_versions disabled", body = ErrorResponse),
    ),
    tag = "agents"
)]
pub async fn list_agent_versions(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> ApiResult<Vec<everruns_platform::AgentVersion>> {
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
        (status = 200, description = "Agent version created", body = everruns_platform::AgentVersion),
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
) -> ApiResult<everruns_platform::AgentVersion> {
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
                    name: Some(req.name),
                    display_name: req.display_name,
                    description: req.description,
                    system_prompt: Some(req.system_prompt),
                    default_model_id: req.default_model_id,
                    harness_id: req.harness_id,
                    harness_name: req.harness_name,
                    tags: Some(req.tags),
                    capabilities: Some(req.capabilities),
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

/// GET /v1/agents/{agent_id}/export - Export agent in Markdown format with YAML front matter
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/export",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed, e.g., agt_...)")
    ),
    responses(
        (status = 200, description = "Agent exported as Markdown", content_type = "text/markdown"),
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
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let agent_id: AgentId = agent_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid agent ID: {}", e))
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    let agent = crate::domains::agents::GetAgent {
        id: agent_id.to_string(),
    }
    .run(&state.ctx(&org))
    .await?;

    let markdown = agent_to_markdown(&agent);
    let filename = format!("{}.md", slugify(&agent.name));

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/markdown; charset=utf-8")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", filename),
        )
        .body(Body::from(markdown))
        .unwrap())
}

/// POST /v1/agents/import - Import agent from file or built-in example
///
/// Two modes:
/// 1. **From example** — `POST /v1/agents/import?from-example={name}` (body ignored)
/// 2. **From file** — `POST /v1/agents/import` with a text body in Markdown/YAML/JSON
///
/// File mode accepts:
/// - Markdown with YAML front matter (if starts with ---)
/// - Pure YAML
/// - Pure JSON
/// - Plain text (treated as system prompt, name auto-generated)
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
    body: String,
) -> Result<(StatusCode, Json<WithUrls<Agent>>), (StatusCode, Json<ErrorResponse>)> {
    // Branch: import from built-in example
    if let Some(name) = query.from_example {
        return import_from_example(org, &state, &name).await;
    }

    // Branch: import from file body
    import_from_file(org, &state, body).await
}

/// Import an agent from a built-in example by name.
async fn import_from_example(
    org: ResolvedOrg,
    state: &AppState,
    name: &str,
) -> Result<(StatusCode, Json<WithUrls<Agent>>), (StatusCode, Json<ErrorResponse>)> {
    use crate::seed::SEED_AGENTS;

    let seed = SEED_AGENTS
        .iter()
        .find(|s| s.name == name)
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
        .filter(|id| !state.platform_definition.capability_registry().has(id))
        .collect();
    if !missing.is_empty() {
        return Err(ErrorResponse::new(format!(
            "Example requires unregistered capabilities: {missing:?}"
        ))
        .into_response(StatusCode::BAD_REQUEST));
    }

    let capabilities: Vec<AgentCapabilityConfig> = seed
        .capabilities
        .iter()
        .map(|cap| {
            let config = cap.config.map_or_else(|| serde_json::json!({}), |f| f());
            AgentCapabilityConfig::with_config(cap.id.to_string(), config)
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
        id: None,
        name: unique_name,
        display_name: Some(seed.display_name.to_string()),
        description: Some(seed.description.to_string()),
        system_prompt: seed.system_prompt.to_string(),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        capabilities,
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

/// Import an agent from a file body (Markdown/YAML/JSON).
async fn import_from_file(
    org: ResolvedOrg,
    state: &AppState,
    body: String,
) -> Result<(StatusCode, Json<WithUrls<Agent>>), (StatusCode, Json<ErrorResponse>)> {
    // Validate import file size (last-resort protection against abuse)
    validate_import_file_size(body.len())?;

    let agent_file = parse_agent_content(&body).map_err(|e| {
        ErrorResponse::new(format!("Invalid format: {}", e)).into_response(StatusCode::BAD_REQUEST)
    })?;

    // Derive display_name and slug name.
    // Legacy files may only have `name` (the old display name); in that case,
    // treat it as display_name and derive the slug from it.
    let display_name = agent_file.display_name.or(agent_file.name.clone());
    let name_fallback = display_name
        .clone()
        .unwrap_or_else(|| format!("agent-{}", Utc::now().format("%Y%m%d-%H%M%S")));
    let name = agent_file
        .name
        .map(|n| {
            // If it looks like a slug already, use it; otherwise slugify
            if n.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            {
                n
            } else {
                slugify(&n)
            }
        })
        .unwrap_or_else(|| slugify(&name_fallback));
    validate_agent_name_format(&name)?;

    // System prompt is required (either from body or front matter)
    let system_prompt = agent_file.system_prompt.unwrap_or_default();
    if system_prompt.is_empty() {
        return Err(ErrorResponse::new(
            "System prompt is required (provide in front matter or as markdown body)",
        )
        .into_response(StatusCode::BAD_REQUEST));
    }

    // Validate parsed content sizes (last-resort protection against abuse)
    // Strip glob patterns (CLI-only); keep only fully-specified files
    let initial_files: Vec<InitialFile> = agent_file
        .initial_files
        .into_iter()
        .filter_map(|f| f.into_initial_file())
        .collect();

    validate_create_agent_input(
        &name,
        display_name.as_deref(),
        agent_file.description.as_deref(),
        &system_prompt,
        agent_file.capabilities.len(),
        &initial_files,
    )?;

    let client_id = agent_file.id;
    let request = CreateAgentRequest {
        id: None, // Already extracted as client_id
        name,
        display_name,
        description: agent_file.description,
        system_prompt,
        default_model_id: agent_file.default_model_id,
        harness_id: agent_file.harness_id,
        harness_name: agent_file.harness_name,
        tags: agent_file.tags,
        capabilities: agent_file
            .capabilities
            .iter()
            .map(|c| c.to_agent_capability_config())
            .collect(),
        initial_files,
        tools: vec![],
        mcp_servers: agent_file.mcp_servers,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
    };

    // TM-AGENT-005: High-risk capabilities require admin role
    require_admin_for_high_risk(&org, &request.capabilities, &state.capability_service)?;

    let _caller = Caller::from(&org);

    let builder = UrlBuilder::from_auth_config(&state.auth.config);

    // If the file has an ID, upsert (create or update). Otherwise, always create.
    if let Some(ref id) = client_id {
        let result = crate::domains::agents::UpsertAgent {
            id: id.to_string(),
            req: request,
        }
        .run(&state.ctx(&org))
        .await?;

        let status = if result.was_created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        };

        Ok((status, Json(builder.wrap(result.agent))))
    } else {
        let agent = crate::domains::agents::CreateAgent(request)
            .run(&state.ctx(&org))
            .await?;

        Ok((StatusCode::CREATED, Json(builder.wrap(agent))))
    }
}

/// Convert agent to Markdown format with YAML front matter
fn agent_to_markdown(agent: &Agent) -> String {
    // Build YAML front matter (skip empty/default fields)
    let mut yaml_lines = vec![];
    yaml_lines.push(format!("id: \"{}\"", agent.public_id));
    yaml_lines.push(format!("name: \"{}\"", agent.name.replace('"', "\\\"")));
    if let Some(ref dn) = agent.display_name {
        yaml_lines.push(format!("display_name: \"{}\"", dn.replace('"', "\\\"")));
    }

    if let Some(desc) = &agent.description {
        yaml_lines.push(format!("description: \"{}\"", desc.replace('"', "\\\"")));
    }

    if let Some(model_id) = agent.default_model_id {
        yaml_lines.push(format!("default_model_id: \"{}\"", model_id));
    }
    yaml_lines.push(format!("harness_id: \"{}\"", agent.harness_id));

    if !agent.tags.is_empty() {
        yaml_lines.push("tags:".to_string());
        for tag in &agent.tags {
            yaml_lines.push(format!("  - \"{}\"", tag.replace('"', "\\\"")));
        }
    }

    if !agent.capabilities.is_empty() {
        yaml_lines.push("capabilities:".to_string());
        for cap in &agent.capabilities {
            // Export capabilities with ref and config (inline JSON for config)
            let config_json =
                serde_json::to_string(cap.config_value()).unwrap_or_else(|_| "{}".to_string());
            yaml_lines.push(format!("  - ref: {}", cap.capability_id()));
            yaml_lines.push(format!("    config: {}", config_json));
        }
    }

    if !agent.initial_files.is_empty() {
        yaml_lines.push("initial_files:".to_string());
        for file in &agent.initial_files {
            yaml_lines.push(format!(
                "  - path: {}",
                serde_json::to_string(&file.path).unwrap_or_else(|_| "\"/\"".to_string())
            ));
            yaml_lines.push(format!("    encoding: {}", file.encoding));
            yaml_lines.push(format!("    is_readonly: {}", file.is_readonly));
            yaml_lines.push(format!(
                "    content: {}",
                serde_json::to_string(&file.content).unwrap_or_else(|_| "\"\"".to_string())
            ));
        }
    }

    format!(
        "---\n{}\n---\n{}",
        yaml_lines.join("\n"),
        agent.system_prompt
    )
}

/// Parse agent content from multiple formats (matches CLI behavior).
/// Tries: Markdown with front matter, JSON, YAML, plain text.
fn parse_agent_content(content: &str) -> Result<AgentFile, String> {
    let content = content.trim();

    // Try markdown with front matter first (if starts with ---)
    if content.starts_with("---")
        && let Ok(agent) = parse_markdown_frontmatter(content)
    {
        return Ok(agent);
    }

    // Try JSON (if starts with {)
    if content.starts_with('{')
        && let Ok(agent) = serde_json::from_str::<AgentFile>(content)
    {
        return Ok(agent);
    }

    // Try YAML
    if let Ok(agent) = serde_yaml::from_str::<AgentFile>(content) {
        // Only accept if it parsed something meaningful (has name or system_prompt)
        if agent.name.is_some() || agent.system_prompt.is_some() {
            return Ok(agent);
        }
    }

    // Fall back to treating entire content as system prompt
    Ok(AgentFile {
        id: None,
        name: None, // Will be auto-generated
        display_name: None,
        description: None,
        system_prompt: Some(content.to_string()),
        default_model_id: None,
        harness_id: None,
        harness_name: None,
        tags: vec![],
        capabilities: vec![],
        initial_files: vec![],
        mcp_servers: Default::default(),
    })
}

/// Parse markdown with YAML front matter.
fn parse_markdown_frontmatter(content: &str) -> Result<AgentFile, String> {
    // Find the closing delimiter
    let rest = &content[3..];
    let end_pos = rest
        .find("\n---")
        .ok_or("Missing closing front matter delimiter (---)")?;

    let front_matter = rest[..end_pos].trim();
    let body = rest.get(end_pos + 4..).unwrap_or("").trim();

    // Parse front matter as YAML
    let mut config: AgentFile =
        serde_yaml::from_str(front_matter).map_err(|e| format!("Invalid YAML: {}", e))?;

    // Body becomes system_prompt if not empty
    if !body.is_empty() {
        config.system_prompt = Some(body.to_string());
    }

    Ok(config)
}

/// Convert string to URL-safe slug
fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// POST /v1/agents/preview - Preview the final agent shape with capabilities applied
///
/// Returns the merged system prompt and all tools that would be available to the agent.
/// This is useful for previewing what the agent will look like before saving.
#[utoipa::path(
    post,
    path = "/v1/agents/preview",
    request_body = PreviewAgentRequest,
    responses(
        (status = 200, description = "Agent preview generated", body = AgentPreviewResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn preview_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<PreviewAgentRequest>,
) -> ApiResult<AgentPreviewResponse> {
    let result = crate::domains::agents::PreviewAgent {
        system_prompt: Some(req.system_prompt),
        capabilities: req.capabilities,
        tools: req.tools,
        mcp_servers: req.mcp_servers,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(AgentPreviewResponse {
        system_prompt: result.system_prompt,
        tools: result.tools,
        findings: result.findings,
    }))
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
mod high_risk_admin_gate_tests {
    use super::*;
    use crate::services::CapabilityService;
    use crate::storage::StorageBackend;
    use std::sync::Arc;

    fn capability_service() -> CapabilityService {
        let db = Arc::new(StorageBackend::in_memory());
        CapabilityService::with_registry(
            db,
            None,
            everruns_platform::capabilities::hosted_capability_registry(),
        )
    }

    fn org_with_role(role: OrgRole) -> ResolvedOrg {
        ResolvedOrg {
            org_id: 1,
            public_id: "org_test".to_string(),
            name: "Test".to_string(),
            user_id: None,
            role,
            is_platform_user: false,
            feature_flags: everruns_platform::FeatureFlags::default(),
        }
    }

    fn caps(refs: &[&str]) -> Vec<AgentCapabilityConfig> {
        refs.iter()
            .map(|r| AgentCapabilityConfig::new((*r).to_string()))
            .collect()
    }

    #[test]
    fn member_blocked_from_assigning_bashkit_shell() {
        let svc = capability_service();
        let result = require_admin_for_high_risk(
            &org_with_role(OrgRole::Member),
            &caps(&["bashkit_shell"]),
            &svc,
        );
        let (status, body) = result.expect_err("member must not assign bashkit_shell");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(
            body.0
                .detail
                .as_deref()
                .unwrap_or("")
                .contains("bashkit_shell")
        );
    }

    #[test]
    fn member_blocked_from_assigning_legacy_virtual_bash_alias() {
        // The pre-rename `virtual_bash` ID resolves to `bashkit_shell` via
        // registry aliasing; the admin gate must cover it identically.
        let svc = capability_service();
        let result = require_admin_for_high_risk(
            &org_with_role(OrgRole::Member),
            &caps(&["virtual_bash"]),
            &svc,
        );
        let (status, _body) = result.expect_err("member must not assign via legacy alias");
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[test]
    fn member_blocked_from_assigning_web_fetch() {
        let svc = capability_service();
        let result = require_admin_for_high_risk(
            &org_with_role(OrgRole::Member),
            &caps(&["web_fetch"]),
            &svc,
        );
        let (status, body) = result.expect_err("member must not assign web_fetch");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.0.detail.as_deref().unwrap_or("").contains("web_fetch"));
    }

    #[test]
    fn member_allowed_for_low_risk_capability() {
        let svc = capability_service();
        let result = require_admin_for_high_risk(
            &org_with_role(OrgRole::Member),
            &caps(&["current_time"]),
            &svc,
        );
        assert!(
            result.is_ok(),
            "low-risk capabilities must remain assignable by members"
        );
    }

    #[test]
    fn admin_allowed_for_bashkit_shell() {
        let svc = capability_service();
        let result = require_admin_for_high_risk(
            &org_with_role(OrgRole::Admin),
            &caps(&["bashkit_shell"]),
            &svc,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn owner_allowed_for_web_fetch() {
        let svc = capability_service();
        let result = require_admin_for_high_risk(
            &org_with_role(OrgRole::Owner),
            &caps(&["web_fetch"]),
            &svc,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn empty_capability_list_allowed_for_members() {
        let svc = capability_service();
        let result = require_admin_for_high_risk(&org_with_role(OrgRole::Member), &[], &svc);
        assert!(result.is_ok());
    }
}
