// Agent CRUD HTTP routes (M2)

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_contracts::{Agent, CreateAgentRequest, ListResponse, UpdateAgentRequest};
use everruns_storage::{
    models::{CreateAgent, UpdateAgent},
    Database,
};
use std::sync::Arc;
use uuid::Uuid;

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
            "/v1/agents/{agent_id}",
            get(get_agent).patch(update_agent).delete(delete_agent),
        )
        .route("/v1/agents/slug/{slug}", get(get_agent_by_slug))
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
    let input = CreateAgent {
        slug: req.slug,
        name: req.name,
        description: req.description,
        system_prompt: req.system_prompt,
        default_model_id: req.default_model_id,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        tools: req.tools,
        tags: req.tags,
    };

    let agent = state.service.create(input).await.map_err(|e| {
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

/// GET /v1/agents/slug/{slug} - Get agent by slug
#[utoipa::path(
    get,
    path = "/v1/agents/slug/{slug}",
    params(
        ("slug" = String, Path, description = "Agent slug")
    ),
    responses(
        (status = 200, description = "Agent found", body = Agent),
        (status = 404, description = "Agent not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn get_agent_by_slug(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Agent>, StatusCode> {
    let agent = state
        .service
        .get_by_slug(&slug)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent by slug: {}", e);
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
    let input = UpdateAgent {
        slug: req.slug,
        name: req.name,
        description: req.description,
        system_prompt: req.system_prompt,
        default_model_id: req.default_model_id,
        temperature: req.temperature,
        max_tokens: req.max_tokens,
        tools: req.tools,
        tags: req.tags,
        status: req.status.map(|s| s.to_string()),
    };

    let agent = state
        .service
        .update(agent_id, input)
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
