// Agent CRUD HTTP routes (M2)

use crate::storage::Database;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_core::{Agent, AgentStatus, CapabilityId};

use crate::common::ListResponse;
use serde::Deserialize;
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
        .route(
            "/v1/agents/:agent_id",
            get(get_agent).patch(update_agent).delete(delete_agent),
        )
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
