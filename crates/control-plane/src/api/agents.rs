// Agent CRUD HTTP routes (M2)

use crate::storage::Database;
use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use everruns_core::{Agent, AgentStatus, CapabilityId};

use super::common::ListResponse;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Request to create a new agent
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateAgentRequest {
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
    pub default_model_id: Option<Uuid>,
    /// Tags for organizing and filtering agents.
    #[serde(default)]
    #[schema(example = json!(["support", "customer-facing"]))]
    pub tags: Vec<String>,
    /// Capabilities to enable for this agent.
    /// Capabilities provide tools and system prompt additions.
    #[serde(default)]
    #[schema(example = json!(["current_time", "web_fetch"]), value_type = Vec<String>)]
    pub capabilities: Vec<CapabilityId>,
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
    pub default_model_id: Option<Uuid>,
    /// Tags for organizing and filtering agents.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["updated-tag"]))]
    pub tags: Option<Vec<String>>,
    /// Capabilities to enable for this agent. Replaces existing capabilities.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["current_time", "web_fetch"]), value_type = Option<Vec<String>>)]
    pub capabilities: Option<Vec<CapabilityId>>,
    /// The status of the agent. Set to "archived" to soft-delete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
}

/// Agent file format for import (matches CLI format)
/// Parsed from YAML front matter in Markdown files.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AgentFile {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<Uuid>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

use crate::services::AgentService;

/// App state for agents routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<AgentService>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            service: Arc::new(AgentService::new(db)),
        }
    }
}

/// Create agent routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/agents", post(create_agent).get(list_agents))
        .route("/v1/agents/import", post(import_agent))
        .route(
            "/v1/agents/:agent_id",
            get(get_agent).patch(update_agent).delete(delete_agent),
        )
        .route("/v1/agents/:agent_id/export", get(export_agent))
        .with_state(state)
}

/// POST /v1/agents - Create a new agent
#[utoipa::path(
    post,
    path = "/v1/agents",
    request_body = CreateAgentRequest,
    responses(
        (status = 201, description = "Agent created successfully", body = Agent),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn create_agent(
    State(state): State<AppState>,
    Json(req): Json<CreateAgentRequest>,
) -> Result<(StatusCode, Json<Agent>), StatusCode> {
    let agent = state.service.create(req).await.map_err(|e| {
        tracing::error!("Failed to create agent: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok((StatusCode::CREATED, Json(agent)))
}

/// GET /v1/agents - List all active agents
#[utoipa::path(
    get,
    path = "/v1/agents",
    responses(
        (status = 200, description = "List of agents", body = ListResponse<Agent>),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn list_agents(
    State(state): State<AppState>,
) -> Result<Json<ListResponse<Agent>>, StatusCode> {
    let agents = state.service.list().await.map_err(|e| {
        tracing::error!("Failed to list agents: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(ListResponse::new(agents)))
}

/// GET /v1/agents/{agent_id} - Get agent by ID
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "Agent found", body = Agent),
        (status = 404, description = "Agent not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn get_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
) -> Result<Json<Agent>, StatusCode> {
    let agent = state
        .service
        .get(agent_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(agent))
}

/// PATCH /v1/agents/{agent_id} - Update agent
#[utoipa::path(
    patch,
    path = "/v1/agents/{agent_id}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    request_body = UpdateAgentRequest,
    responses(
        (status = 200, description = "Agent updated successfully", body = Agent),
        (status = 404, description = "Agent not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn update_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
    Json(req): Json<UpdateAgentRequest>,
) -> Result<Json<Agent>, StatusCode> {
    let agent = state
        .service
        .update(agent_id, req)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update agent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(agent))
}

/// DELETE /v1/agents/{agent_id} - Archive agent
#[utoipa::path(
    delete,
    path = "/v1/agents/{agent_id}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 204, description = "Agent archived successfully"),
        (status = 404, description = "Agent not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn delete_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state.service.delete(agent_id).await.map_err(|e| {
        tracing::error!("Failed to delete agent: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// GET /v1/agents/{agent_id}/export - Export agent in Markdown format with YAML front matter
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/export",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "Agent exported as Markdown", content_type = "text/markdown"),
        (status = 404, description = "Agent not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn export_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
) -> Result<Response, StatusCode> {
    let agent = state
        .service
        .get(agent_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent for export: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

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
        (status = 400, description = "Invalid format"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn import_agent(
    State(state): State<AppState>,
    body: String,
) -> Result<(StatusCode, Json<Agent>), (StatusCode, String)> {
    let agent_file = parse_agent_content(&body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid format: {}", e)))?;

    // Generate date-based name if not provided
    let name = agent_file
        .name
        .unwrap_or_else(|| format!("agent-{}", Utc::now().format("%Y%m%d-%H%M%S")));

    // System prompt is required (either from body or front matter)
    let system_prompt = agent_file.system_prompt.unwrap_or_default();
    if system_prompt.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "System prompt is required (provide in front matter or as markdown body)".to_string(),
        ));
    }

    let request = CreateAgentRequest {
        name,
        description: agent_file.description,
        system_prompt,
        default_model_id: agent_file.default_model_id,
        tags: agent_file.tags,
        capabilities: agent_file
            .capabilities
            .into_iter()
            .map(CapabilityId::from)
            .collect(),
    };

    let agent = state.service.create(request).await.map_err(|e| {
        tracing::error!("Failed to import agent: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error".to_string(),
        )
    })?;

    Ok((StatusCode::CREATED, Json(agent)))
}

/// Convert agent to Markdown format with YAML front matter
fn agent_to_markdown(agent: &Agent) -> String {
    let mut front_matter = AgentFile {
        name: Some(agent.name.clone()),
        description: agent.description.clone(),
        system_prompt: None, // System prompt goes in body
        default_model_id: agent.default_model_id,
        tags: agent.tags.clone(),
        capabilities: agent.capabilities.iter().map(|c| c.to_string()).collect(),
    };

    // Don't include empty arrays in front matter
    if front_matter.tags.is_empty() {
        front_matter.tags = vec![];
    }
    if front_matter.capabilities.is_empty() {
        front_matter.capabilities = vec![];
    }

    // Build YAML front matter (skip empty/default fields)
    let mut yaml_lines = vec![];
    yaml_lines.push(format!("name: \"{}\"", agent.name.replace('"', "\\\"")));

    if let Some(ref desc) = front_matter.description {
        yaml_lines.push(format!("description: \"{}\"", desc.replace('"', "\\\"")));
    }

    if let Some(model_id) = front_matter.default_model_id {
        yaml_lines.push(format!("default_model_id: \"{}\"", model_id));
    }

    if !front_matter.tags.is_empty() {
        yaml_lines.push("tags:".to_string());
        for tag in &front_matter.tags {
            yaml_lines.push(format!("  - \"{}\"", tag.replace('"', "\\\"")));
        }
    }

    if !front_matter.capabilities.is_empty() {
        yaml_lines.push("capabilities:".to_string());
        for cap in &front_matter.capabilities {
            yaml_lines.push(format!("  - {}", cap));
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
    if content.starts_with("---") {
        if let Ok(agent) = parse_markdown_frontmatter(content) {
            return Ok(agent);
        }
    }

    // Try JSON (if starts with {)
    if content.starts_with('{') {
        if let Ok(agent) = serde_json::from_str::<AgentFile>(content) {
            return Ok(agent);
        }
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
