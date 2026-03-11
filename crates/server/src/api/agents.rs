// Agent CRUD HTTP routes (M2)
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

use crate::auth::{AuthState, ResolvedOrg};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
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
use everruns_core::{Agent, AgentCapabilityConfig, AgentStatus, OrgRole, ToolDefinition};

use super::common::{ApiOptionExt, ApiResultExt, ErrorResponse, ListResponse};
use super::validation::{
    validate_create_agent_input, validate_import_file_size, validate_update_agent_input,
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
    /// The name of the agent. Used for display purposes.
    #[schema(example = "Customer Support Agent")]
    pub name: String,
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
    /// Client-side tools for this agent.
    /// These tools are sent to the LLM but executed by the client, not the server.
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
}

/// Request to update an agent. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateAgentRequest {
    /// The name of the agent. Used for display purposes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Updated Support Agent")]
    pub name: Option<String>,
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
    /// The status of the agent. Set to "archived" to soft-delete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
    /// Client-side tools for this agent.
    /// Replaces existing tools if provided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
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

/// Agent file format for import (matches CLI format)
/// Parsed from YAML front matter in Markdown files.
/// Supports both legacy (string list) and new (object with ref/config) capability formats.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AgentFile {
    /// Optional agent ID (format: agent_{32-hex}). Preserved during import/export.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<AgentId>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<ModelId>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Capabilities - supports both string IDs and objects with ref/config
    #[serde(default)]
    pub capabilities: Vec<AgentFileCapability>,
}

use crate::services::{AgentService, CapabilityService};

/// Query parameters for listing agents.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAgentsQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
}

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

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create agent routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/agents", post(create_agent).get(list_agents))
        .route("/v1/agents/import", post(import_agent))
        .route("/v1/agents/preview", post(preview_agent))
        .route(
            "/v1/agents/{agent_id}",
            get(get_agent)
                .put(upsert_agent)
                .patch(update_agent)
                .delete(delete_agent),
        )
        .route("/v1/agents/{agent_id}/export", get(export_agent))
        .route("/v1/agents/{agent_id}/copy", post(copy_agent))
        .with_state(state)
}

/// TM-AGENT-005: Reject if any requested capabilities are high-risk and the
/// caller does not have at least Admin role.
fn require_admin_for_high_risk(
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
    // Validate input sizes (last-resort protection against abuse)
    validate_create_agent_input(
        &req.name,
        req.description.as_deref(),
        &req.system_prompt,
        req.capabilities.len(),
    )?;

    // TM-AGENT-005: High-risk capabilities require admin role
    require_admin_for_high_risk(&org, &req.capabilities, &state.capability_service)?;

    let client_id = req.id;
    let agent = match state.service.create(org.org_id, client_id, req).await {
        Ok(agent) => agent,
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("duplicate key") || msg.contains("already exists") {
                return Err(ErrorResponse::conflict("Agent with this ID already exists"));
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
        (status = 200, description = "List of agents", body = ListResponse<Agent>),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn list_agents(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListAgentsQuery>,
) -> Result<Json<ListResponse<Agent>>, StatusCode> {
    let agents = state
        .service
        .list(org.org_id, query.search.as_deref())
        .await
        .log_internal_error("list agents")?;

    Ok(Json(ListResponse::new(agents)))
}

/// GET /v1/agents/{agent_id} - Get agent by ID
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (prefixed, e.g., agt_...)")
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
    Path(agent_id): Path<String>,
) -> Result<Json<Agent>, (StatusCode, Json<ErrorResponse>)> {
    let agent_id: AgentId = agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    let agent = state
        .service
        .get_by_public_id(org.org_id, &agent_id.to_string())
        .await
        .log_internal_error_json("get agent")?
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
) -> Result<Json<Agent>, (StatusCode, Json<ErrorResponse>)> {
    let agent_id: AgentId = agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    // Validate input sizes (last-resort protection against abuse)
    validate_update_agent_input(
        req.name.as_deref(),
        req.description.as_deref(),
        req.system_prompt.as_deref(),
        req.capabilities.as_ref().map(|c| c.len()),
    )?;

    // TM-AGENT-005: High-risk capabilities require admin role
    if let Some(caps) = &req.capabilities {
        require_admin_for_high_risk(&org, caps, &state.capability_service)?;
    }

    let agent = state
        .service
        .update(org.org_id, &agent_id.to_string(), req)
        .await
        .log_internal_error_json("update agent")?
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

    let deleted = state
        .service
        .delete(org.org_id, &agent_id.to_string())
        .await
        .log_internal_error_json("delete agent")?;

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

    let agent = state
        .service
        .copy(org.org_id, &agent_id.to_string())
        .await
        .log_internal_error_json("copy agent")?
        .ok_or_not_found_json("Agent")?;

    Ok((StatusCode::CREATED, Json(agent)))
}

/// PUT /v1/agents/{agent_id} - Create or update agent (upsert)
///
/// If agent with this ID exists, update it. If not, create it.
/// Returns 201 on create, 200 on update.
#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}",
    params(
        ("agent_id" = String, Path, description = "Agent ID (format: agent_{32-hex})")
    ),
    request_body = CreateAgentRequest,
    responses(
        (status = 200, description = "Agent updated", body = Agent),
        (status = 201, description = "Agent created", body = Agent),
        (status = 400, description = "Invalid agent ID or input exceeds allowed limits", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "agents"
)]
pub async fn upsert_agent(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<Agent>), (StatusCode, Json<ErrorResponse>)> {
    let agent_id: AgentId = agent_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid agent ID: {}", e),
            }),
        )
    })?;

    // Validate input sizes
    validate_create_agent_input(
        &req.name,
        req.description.as_deref(),
        &req.system_prompt,
        req.capabilities.len(),
    )?;

    // TM-AGENT-005: High-risk capabilities require admin role
    require_admin_for_high_risk(&org, &req.capabilities, &state.capability_service)?;

    let (agent, was_created) = state
        .service
        .upsert(org.org_id, &agent_id.to_string(), req)
        .await
        .log_internal_error_json("upsert agent")?;

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

    let agent = state
        .service
        .get_by_public_id(org.org_id, &agent_id.to_string())
        .await
        .log_internal_error_json("get agent for export")?
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
#[utoipa::path(
    post,
    path = "/v1/agents/import",
    request_body(content = String, content_type = "text/plain"),
    responses(
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

    // Generate date-based name if not provided
    let name = agent_file
        .name
        .unwrap_or_else(|| format!("agent-{}", Utc::now().format("%Y%m%d-%H%M%S")));

    // System prompt is required (either from body or front matter)
    let system_prompt = agent_file.system_prompt.unwrap_or_default();
    if system_prompt.is_empty() {
        return Err(ErrorResponse::new(
            "System prompt is required (provide in front matter or as markdown body)",
        )
        .into_response(StatusCode::BAD_REQUEST));
    }

    // Validate parsed content sizes (last-resort protection against abuse)
    validate_create_agent_input(
        &name,
        agent_file.description.as_deref(),
        &system_prompt,
        agent_file.capabilities.len(),
    )?;

    let client_id = agent_file.id;
    let request = CreateAgentRequest {
        id: None, // Already extracted as client_id
        name,
        description: agent_file.description,
        system_prompt,
        default_model_id: agent_file.default_model_id,
        tags: agent_file.tags,
        capabilities: agent_file
            .capabilities
            .iter()
            .map(|c| c.to_agent_capability_config())
            .collect(),
        tools: vec![],
    };

    // TM-AGENT-005: High-risk capabilities require admin role
    require_admin_for_high_risk(&org, &request.capabilities, &state.capability_service)?;

    let agent = state
        .service
        .create(org.org_id, client_id, request)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("duplicate key") || msg.contains("already exists") {
                return ErrorResponse::conflict("Agent with this ID already exists");
            }
            tracing::error!("Failed to import agent: {}", msg);
            ErrorResponse::internal_error()
        })?;

    Ok((StatusCode::CREATED, Json(agent)))
}

/// Convert agent to Markdown format with YAML front matter
fn agent_to_markdown(agent: &Agent) -> String {
    // Build YAML front matter (skip empty/default fields)
    let mut yaml_lines = vec![];
    yaml_lines.push(format!("id: \"{}\"", agent.public_id));
    yaml_lines.push(format!("name: \"{}\"", agent.name.replace('"', "\\\"")));

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
        description: None,
        system_prompt: Some(content.to_string()),
        default_model_id: None,
        tags: vec![],
        capabilities: vec![],
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
) -> Result<Json<AgentPreviewResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (system_prompt, tools) = state
        .capability_service
        .preview(org.org_id, &req.system_prompt, &req.capabilities)
        .await
        .map_err(|e| {
            tracing::error!("Failed to generate agent preview: {}", e);
            ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;

    Ok(Json(AgentPreviewResponse {
        system_prompt,
        tools,
    }))
}
