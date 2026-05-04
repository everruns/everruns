// Agent CRUD HTTP routes (M2)
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

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
use everruns_core::typed_id::{AgentId, ModelId};
use everruns_core::{
    Agent, AgentCapabilityConfig, Caller, DeploymentGrade, InitialFile, OrgRole,
    PlatformDefinition, ResourceConfigResponse, ScopedMcpServers, evaluate_policies_with,
};
use futures::future::try_join_all;

use super::common::{
    ApiResult, ApiResultExt, ErrorResponse, PaginatedResponse, ResourceStatsResponse,
    ResourceWithCounts, UrlBuilder, WithUrls, impl_auth_state,
};
use super::validation::{
    validate_agent_name_format, validate_create_agent_input, validate_import_file_size,
};
use crate::domains::agents::types::{
    AgentPreviewResponse, CheckAgentNameQuery, CheckAgentNameResponse, CreateAgentRequest,
    ImportAgentQuery, ListAgentsQuery, PreviewAgentRequest, UpdateAgentRequest,
};
use crate::domains::common::Command;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
        grade: DeploymentGrade,
        platform_definition: Arc<PlatformDefinition>,
    ) -> Self {
        Self {
            db,
            capability_service,
            auth,
            grade,
            platform_definition,
        }
    }

    /// Build a domain Ctx from this AppState for the given org.
    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);

async fn add_agent_counts(
    db: &StorageBackend,
    org_id: i64,
    agent: Agent,
) -> Result<ResourceWithCounts<Agent>, (StatusCode, Json<ErrorResponse>)> {
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

    Ok(ResourceWithCounts {
        session_count,
        app_count,
        inner: agent,
    })
}

async fn add_agents_counts(
    db: &StorageBackend,
    org_id: i64,
    agents: Vec<Agent>,
) -> Result<Vec<ResourceWithCounts<Agent>>, (StatusCode, Json<ErrorResponse>)> {
    try_join_all(
        agents
            .into_iter()
            .map(|agent| add_agent_counts(db, org_id, agent)),
    )
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
    let refs: Vec<&str> = caps.iter().map(|c| c.capability_ref.as_str()).collect();
    let high = capability_service.high_risk_ids(&refs);
    if !high.is_empty() {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorResponse {
                error: format!(
                    "Admin role required to assign high-risk capabilities: {}",
                    high.join(", ")
                ),
            }),
        ));
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
    let agent = crate::domains::agents::CreateAgent(req)
        .run(&state.ctx(&org))
        .await?;
    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(builder.wrap(agent))))
}

/// GET /v1/agents - List all active agents
#[utoipa::path(
    get,
    path = "/v1/agents",
    params(ListAgentsQuery),
    responses(
        (status = 200, description = "Paginated list of agents", body = PaginatedResponse<WithUrls<ResourceWithCounts<Agent>>>),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn list_agents(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListAgentsQuery>,
) -> ApiResult<PaginatedResponse<WithUrls<ResourceWithCounts<Agent>>>> {
    let result = crate::domains::agents::ListAgents {
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
        offset: query.offset,
        limit: query.limit,
    }
    .run(&state.ctx(&org))
    .await?;

    let data = add_agents_counts(&state.db, org.org_id, result.data).await?;
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
        (status = 200, description = "Agent found", body = WithUrls<ResourceWithCounts<Agent>>),
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
) -> ApiResult<WithUrls<ResourceWithCounts<Agent>>> {
    let agent = crate::domains::agents::GetAgent {
        id: agent_id_or_name,
    }
    .run(&state.ctx(&org))
    .await?;
    let agent = add_agent_counts(&state.db, org.org_id, agent).await?;
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
    let agent = crate::domains::agents::UpdateAgentCmd { id: agent_id, req }
        .run(&state.ctx(&org))
        .await?;
    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(builder.wrap(agent)))
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
    crate::domains::agents::DeleteAgent { id: agent_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn destroy_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    crate::domains::agents::DestroyAgent { id: agent_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
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
    let agent = crate::domains::agents::CopyAgent { id: agent_id }
        .run(&state.ctx(&org))
        .await?;
    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(builder.wrap(agent))))
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
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(
                    "Agent name in URL must match name in request body",
                )),
            ));
        }
        // Name-based upsert: try create, if name taken → update
        let create_result = crate::domains::agents::CreateAgent(req.clone())
            .run(&state.ctx(&org))
            .await;
        match create_result {
            Ok(agent) => (agent, true),
            Err(crate::domains::common::CommandError::Conflict(_)) => {
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
                    tags: Some(req.tags),
                    capabilities: Some(req.capabilities),
                    initial_files: Some(req.initial_files),
                    tools: Some(req.tools),
                    mcp_servers: Some(req.mcp_servers),
                    network_access: req.network_access,
                    max_iterations: req.max_iterations,
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
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
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
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Example requires unregistered capabilities: {missing:?}"),
            }),
        ));
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
        use rand::Rng;

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
        tags: seed.tags.iter().map(|s| s.to_string()).collect(),
        capabilities,
        initial_files: vec![],
        tools: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        max_iterations: None,
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
                serde_json::to_string(&cap.config).unwrap_or_else(|_| "{}".to_string());
            yaml_lines.push(format!("  - ref: {}", cap.capability_ref.as_str()));
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
    }))
}

// Regression tests for fix(capabilities): restore high-risk levels for
// bash/fetch (#1500). `require_admin_for_high_risk` is the HTTP-side gate
// that enforces TM-AGENT-005: a member cannot assign `virtual_bash` or
// `web_fetch` to an agent. The fix re-classified both capabilities as
// High; without that classification this gate silently becomes a no-op
// for the two most dangerous capabilities.
#[cfg(test)]
mod high_risk_admin_gate_tests {
    use super::*;
    use crate::services::CapabilityService;
    use crate::storage::StorageBackend;
    use everruns_core::CapabilityRegistry;
    use std::sync::Arc;

    fn capability_service() -> CapabilityService {
        let db = Arc::new(StorageBackend::in_memory());
        CapabilityService::with_registry(db, None, CapabilityRegistry::with_builtins())
    }

    fn org_with_role(role: OrgRole) -> ResolvedOrg {
        ResolvedOrg {
            org_id: 1,
            public_id: "org_test".to_string(),
            name: "Test".to_string(),
            user_id: None,
            role,
            is_platform_user: false,
        }
    }

    fn caps(refs: &[&str]) -> Vec<AgentCapabilityConfig> {
        refs.iter()
            .map(|r| AgentCapabilityConfig::new((*r).to_string()))
            .collect()
    }

    #[test]
    fn member_blocked_from_assigning_virtual_bash() {
        let svc = capability_service();
        let result = require_admin_for_high_risk(
            &org_with_role(OrgRole::Member),
            &caps(&["virtual_bash"]),
            &svc,
        );
        let (status, body) = result.expect_err("member must not assign virtual_bash");
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(body.0.error.contains("virtual_bash"));
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
        assert!(body.0.error.contains("web_fetch"));
    }

    #[test]
    fn member_allowed_for_low_risk_capability() {
        let svc = capability_service();
        let result =
            require_admin_for_high_risk(&org_with_role(OrgRole::Member), &caps(&["noop"]), &svc);
        assert!(
            result.is_ok(),
            "low-risk capabilities must remain assignable by members"
        );
    }

    #[test]
    fn admin_allowed_for_virtual_bash() {
        let svc = capability_service();
        let result = require_admin_for_high_risk(
            &org_with_role(OrgRole::Admin),
            &caps(&["virtual_bash"]),
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
