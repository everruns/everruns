// Agent CRUD HTTP routes (M2)
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

use crate::auth::{AuthState, ResolvedOrg};
use crate::errors::ResourceNotFoundError;
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
    Agent, AgentCapabilityConfig, AgentStatus, Caller, InitialFile, OrgRole,
    ResourceConfigResponse, ToolDefinition, evaluate_policies_with,
};

use super::common::{
    ApiOptionExt, ApiPolicyResultExt, ApiResult, ErrorResponse, PaginatedResponse, Pagination,
    impl_auth_state,
};
use super::validation::{
    validate_agent_name_format, validate_create_agent_input, validate_import_file_size,
    validate_update_agent_input,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

/// Request to create a new agent
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateAgentRequest {
    /// Client-supplied agent ID (format: agent_{32-hex}).
    /// If not provided, one is auto-generated.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub id: Option<AgentId>,
    /// Name, unique per org. Lowercase alphanumeric and hyphens.
    /// Format: lowercase alphanumeric and hyphens (e.g. "customer-support").
    #[schema(example = "customer-support")]
    pub name: String,
    /// Human-readable display name shown in UI.
    /// Falls back to `name` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "Customer Support Agent")]
    pub display_name: Option<String>,
    /// A human-readable description of what the agent does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Handles customer inquiries and support tickets")]
    pub description: Option<String>,
    /// The system prompt that defines the agent's behavior and capabilities.
    /// This is sent as the first message in every conversation.
    #[schema(example = "You are a helpful customer support agent. Be polite and professional.")]
    pub system_prompt: String,
    /// The ID of the default LLM model to use for this agent.
    /// If not specified, the system default model will be used.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<ModelId>,
    /// Tags for organizing and filtering agents.
    #[serde(default)]
    #[schema(example = json!(["support", "customer-facing"]))]
    pub tags: Vec<String>,
    /// Capabilities to enable for this agent with per-agent configuration.
    /// Each capability has a `ref` (capability ID) and optional `config`.
    #[serde(default)]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this agent.
    #[serde(default)]
    pub initial_files: Vec<InitialFile>,
    /// Client-side tools for this agent.
    /// These tools are sent to the LLM but executed by the client, not the server.
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    /// Network access list controlling which hosts/URLs this agent's sessions can reach.
    /// If set, merged with harness and session layers (allowed: intersect, blocked: union).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
}

/// Request to update an agent. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentRequest {
    /// Name, unique per org. Lowercase alphanumeric and hyphens.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "updated-support")]
    pub name: Option<String>,
    /// Human-readable display name shown in UI.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Updated Support Agent")]
    pub display_name: Option<String>,
    /// A human-readable description of what the agent does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Updated description for the agent")]
    pub description: Option<String>,
    /// The system prompt that defines the agent's behavior and capabilities.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "You are an updated helpful assistant.")]
    pub system_prompt: Option<String>,
    /// The ID of the default LLM model to use for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<ModelId>,
    /// Tags for organizing and filtering agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["updated-tag"]))]
    pub tags: Option<Vec<String>>,
    /// Capabilities to enable for this agent with per-agent configuration.
    /// Replaces existing capabilities. Each has a `ref` (capability ID) and optional `config`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    pub capabilities: Option<Vec<AgentCapabilityConfig>>,
    /// Starter files copied into each new session for this agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_files: Option<Vec<InitialFile>>,
    /// The status of the agent. Set to "archived" to soft-delete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
    /// Client-side tools for this agent.
    /// Replaces existing tools if provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    /// Network access list controlling which hosts/URLs this agent's sessions can reach.
    /// Send `{}` (empty object) to clear restrictions. Omit to leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    /// Maximum number of LLM iterations per turn for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<usize>,
}

/// Request to preview the final agent shape with capabilities applied
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PreviewAgentRequest {
    /// The base system prompt (before capability additions)
    #[schema(example = "You are a helpful customer support agent.")]
    pub system_prompt: String,
    /// Capabilities to apply with per-agent configuration.
    #[serde(default)]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "test_math", "config": {}}]))]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Client-side tools to include in the preview.
    #[serde(default)]
    #[schema(value_type = Vec<Object>)]
    pub tools: Vec<ToolDefinition>,
}

/// Response showing the final agent shape after applying capabilities
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AgentPreviewResponse {
    /// The full system prompt with capability additions prepended
    pub system_prompt: String,
    /// All tool definitions from capabilities
    #[schema(value_type = Vec<Object>)]
    pub tools: Vec<ToolDefinition>,
}

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
}

use crate::services::agent::{AGENT_DANGEROUS, AGENT_MANAGE, AGENT_VIEW};
use crate::services::{AgentService, CapabilityService};

/// Query parameters for listing agents.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAgentsQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived agents. Deleted agents never appear in lists.
    pub include_archived: Option<bool>,
    /// Pagination offset (default 0).
    #[param(minimum = 0, default = 0)]
    pub offset: Option<u32>,
    /// Maximum items to return (default 20, max 100).
    #[param(minimum = 1, maximum = 100, default = 20)]
    pub limit: Option<u32>,
}

/// Query parameters for checking agent name availability.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct CheckAgentNameQuery {
    /// The agent name to check.
    pub name: String,
    /// Optional agent ID to exclude (for edit forms where the current agent's own name is valid).
    pub exclude_id: Option<String>,
}

/// Response for agent name availability check.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CheckAgentNameResponse {
    /// Whether the name is available for use.
    pub available: bool,
}

/// Default limit for agent listing
const DEFAULT_LIMIT: u32 = 20;
/// Maximum allowed limit for agent listing
const MAX_LIMIT: u32 = 100;

/// App state for agents routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<AgentService>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        Self {
            service: Arc::new(AgentService::new(db)),
            capability_service,
            auth,
        }
    }
}

impl_auth_state!(AppState);

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
    // Validate format first — if the name is invalid, it's not "available"
    if validate_agent_name_format(&query.name).is_err() {
        return Ok(Json(CheckAgentNameResponse { available: false }));
    }

    let exclude_id = query
        .exclude_id
        .as_deref()
        .map(|id| id.parse::<AgentId>())
        .transpose()
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid exclude_id: {}", e),
                }),
            )
        })?;

    let caller = Caller::from(&org);
    let available = state
        .service
        .check_name_available(&caller, &query.name, exclude_id)
        .await
        .map_policy_or_internal("check agent name")?;

    Ok(Json(CheckAgentNameResponse { available }))
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
        (status = 201, description = "Agent created successfully", body = Agent),
        (status = 400, description = "Input exceeds allowed limits", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn create_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<Agent>), (StatusCode, Json<ErrorResponse>)> {
    // Validate slug name format
    validate_agent_name_format(&req.name)?;

    // Validate input sizes (last-resort protection against abuse)
    validate_create_agent_input(
        &req.name,
        req.display_name.as_deref(),
        req.description.as_deref(),
        &req.system_prompt,
        req.capabilities.len(),
        &req.initial_files,
    )?;

    // TM-AGENT-005: High-risk capabilities require admin role
    require_admin_for_high_risk(&org, &req.capabilities, &state.capability_service)?;

    let caller = Caller::from(&org);
    let client_id = req.id;
    let agent = match state.service.create(&caller, client_id, req).await {
        Ok(agent) => agent,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("duplicate key") || msg.contains("already exists") {
                return Err(ErrorResponse::conflict("Agent with this ID already exists"));
            }
            if let Some(not_found) = e.downcast_ref::<ResourceNotFoundError>() {
                return Err(ErrorResponse::not_found(not_found.resource()));
            }
            tracing::error!("Failed to create agent: {}", msg);
            return Err(ErrorResponse::internal_error());
        }
    };

    Ok((StatusCode::CREATED, Json(agent)))
}

/// GET /v1/agents - List all active agents
#[utoipa::path(
    get,
    path = "/v1/agents",
    params(ListAgentsQuery),
    responses(
        (status = 200, description = "Paginated list of agents", body = PaginatedResponse<Agent>),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn list_agents(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListAgentsQuery>,
) -> ApiResult<PaginatedResponse<Agent>> {
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);
    let pagination = Pagination::new(offset, limit);

    let caller = Caller::from(&org);
    let (agents, total) = state
        .service
        .list(
            &caller,
            query.search.as_deref(),
            query.include_archived.unwrap_or(false),
            pagination,
        )
        .await
        .map_policy_or_internal("list agents")?;

    Ok(Json(PaginatedResponse::new(agents, total, offset, limit)))
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
        (status = 200, description = "Agent found", body = Agent),
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
) -> ApiResult<Agent> {
    let caller = Caller::from(&org);

    // Try parsing as a prefixed ID first; fall back to name lookup.
    if let Ok(agent_id) = agent_id_or_name.parse::<AgentId>() {
        let agent = state
            .service
            .get_by_public_id(&caller, &agent_id.to_string())
            .await
            .map_policy_or_internal("get agent")?
            .ok_or_not_found_json("Agent")?;
        return Ok(Json(agent));
    }

    let agent = state
        .service
        .get_by_name(&caller, &agent_id_or_name)
        .await
        .map_policy_or_internal("get agent by name")?
        .ok_or_not_found_json("Agent")?;

    Ok(Json(agent))
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
        (status = 200, description = "Agent updated successfully", body = Agent),
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
) -> ApiResult<Agent> {
    let agent_id: AgentId = agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    // Validate slug name format if provided
    if let Some(ref name) = req.name {
        validate_agent_name_format(name)?;
    }

    // Validate input sizes (last-resort protection against abuse)
    validate_update_agent_input(
        req.display_name.as_deref(),
        req.description.as_deref(),
        req.system_prompt.as_deref(),
        req.capabilities.as_ref().map(|c| c.len()),
        req.initial_files.as_deref(),
    )?;

    // TM-AGENT-005: High-risk capabilities require admin role
    if let Some(caps) = &req.capabilities {
        require_admin_for_high_risk(&org, caps, &state.capability_service)?;
    }

    let caller = Caller::from(&org);
    let agent = state
        .service
        .update(&caller, &agent_id.to_string(), req)
        .await
        .map_policy_or_internal("update agent")?
        .ok_or_not_found_json("Agent")?;

    Ok(Json(agent))
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
    let agent_id: AgentId = agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .delete(&caller, &agent_id.to_string())
        .await
        .map_policy_or_internal("delete agent")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Agent not found".to_string(),
            }),
        ))
    }
}

pub async fn destroy_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let agent_id: AgentId = agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .destroy(&caller, &agent_id.to_string())
        .await
        .map_policy_or_internal("destroy agent")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Agent not found".to_string(),
            }),
        ))
    }
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
        (status = 201, description = "Agent copied successfully", body = Agent),
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
) -> Result<(StatusCode, Json<Agent>), (StatusCode, Json<ErrorResponse>)> {
    let agent_id: AgentId = agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let agent = state
        .service
        .copy(&caller, &agent_id.to_string())
        .await
        .map_policy_or_internal("copy agent")?
        .ok_or_not_found_json("Agent")?;

    Ok((StatusCode::CREATED, Json(agent)))
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
        (status = 200, description = "Agent updated", body = Agent),
        (status = 201, description = "Agent created", body = Agent),
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
) -> Result<(StatusCode, Json<Agent>), (StatusCode, Json<ErrorResponse>)> {
    // Validate name format
    validate_agent_name_format(&req.name)?;

    // Validate input sizes
    validate_create_agent_input(
        &req.name,
        req.display_name.as_deref(),
        req.description.as_deref(),
        &req.system_prompt,
        req.capabilities.len(),
        &req.initial_files,
    )?;

    // TM-AGENT-005: High-risk capabilities require admin role
    require_admin_for_high_risk(&org, &req.capabilities, &state.capability_service)?;

    let caller = Caller::from(&org);

    // Try parsing as a prefixed ID first; fall back to upsert by name.
    let (agent, was_created) = if let Ok(agent_id) = agent_id_or_name.parse::<AgentId>() {
        state
            .service
            .upsert(&caller, &agent_id.to_string(), req)
            .await
            .map_policy_or_internal("upsert agent")?
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
        state
            .service
            .upsert_by_name(&caller, req)
            .await
            .map_policy_or_internal("upsert agent by name")?
    };

    let status = if was_created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    Ok((status, Json(agent)))
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

    let caller = Caller::from(&org);
    let agent = state
        .service
        .get_by_public_id(&caller, &agent_id.to_string())
        .await
        .map_policy_or_internal("get agent for export")?
        .ok_or_not_found_json("Agent")?;

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

/// POST /v1/agents/import - Import agent from Markdown, YAML, or JSON
///
/// Accepts agent definition in multiple formats:
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
    request_body(content = String, content_type = "text/plain"),
    responses(
        (status = 200, description = "Agent updated via import", body = Agent),
        (status = 201, description = "Agent imported successfully", body = Agent),
        (status = 400, description = "Invalid format or input exceeds limits", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn import_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    body: String,
) -> Result<(StatusCode, Json<Agent>), (StatusCode, Json<ErrorResponse>)> {
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
        network_access: None,
        max_iterations: None,
    };

    // TM-AGENT-005: High-risk capabilities require admin role
    require_admin_for_high_risk(&org, &request.capabilities, &state.capability_service)?;

    let caller = Caller::from(&org);

    // If the file has an ID, upsert (create or update). Otherwise, always create.
    if let Some(ref id) = client_id {
        let (agent, was_created) = state
            .service
            .upsert(&caller, &id.to_string(), request)
            .await
            .map_policy_or_internal("import agent")?;

        let status = if was_created {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        };

        Ok((status, Json(agent)))
    } else {
        let agent = state
            .service
            .create(&caller, None, request)
            .await
            .map_err(|e| {
                tracing::error!("Failed to import agent: {}", e);
                ErrorResponse::internal_error()
            })?;

        Ok((StatusCode::CREATED, Json(agent)))
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
    let (system_prompt, mut tools) = state
        .capability_service
        .preview(org.org_id, &req.system_prompt, &req.capabilities)
        .await
        .map_err(|e| {
            tracing::error!("Failed to generate agent preview: {}", e);
            ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;
    tools.extend(req.tools);

    Ok(Json(AgentPreviewResponse {
        system_prompt,
        tools,
    }))
}
