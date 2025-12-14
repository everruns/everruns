// Agent CRUD HTTP routes

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use everruns_contracts::{Agent, AgentStatus, AgentVersion};
use everruns_storage::{
    models::{CreateAgent, CreateAgentVersion, UpdateAgent},
    Database,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// App state
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Database>,
}

/// Request to create an agent
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentRequest {
    pub name: String,
    pub description: Option<String>,
    pub default_model_id: String,
}

/// Request to update an agent
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAgentRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub default_model_id: Option<String>,
    pub status: Option<AgentStatus>,
}

/// Request to create an agent version
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAgentVersionRequest {
    pub definition: serde_json::Value,
}

/// Create agent routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/agents", post(create_agent).get(list_agents))
        .route("/v1/agents/:agent_id", get(get_agent).patch(update_agent))
        .route(
            "/v1/agents/:agent_id/versions",
            post(create_agent_version).get(list_agent_versions),
        )
        .route(
            "/v1/agents/:agent_id/versions/:version",
            get(get_agent_version),
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
    let input = CreateAgent {
        name: req.name,
        description: req.description,
        default_model_id: req.default_model_id,
    };

    let row = state.db.create_agent(input).await.map_err(|e| {
        tracing::error!("Failed to create agent: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let agent = Agent {
        id: row.id,
        name: row.name,
        description: row.description,
        default_model_id: row.default_model_id,
        status: row.status.parse().unwrap_or(AgentStatus::Active),
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok((StatusCode::CREATED, Json(agent)))
}

/// GET /v1/agents
#[utoipa::path(
    get,
    path = "/v1/agents",
    responses(
        (status = 200, description = "List of agents", body = Vec<Agent>),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn list_agents(State(state): State<AppState>) -> Result<Json<Vec<Agent>>, StatusCode> {
    let rows = state.db.list_agents().await.map_err(|e| {
        tracing::error!("Failed to list agents: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let agents: Vec<Agent> = rows
        .into_iter()
        .map(|row| Agent {
            id: row.id,
            name: row.name,
            description: row.description,
            default_model_id: row.default_model_id,
            status: row.status.parse().unwrap_or(AgentStatus::Active),
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect();

    Ok(Json(agents))
}

/// GET /v1/agents/:agent_id
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
    let row = state
        .db
        .get_agent(agent_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let agent = Agent {
        id: row.id,
        name: row.name,
        description: row.description,
        default_model_id: row.default_model_id,
        status: row.status.parse().unwrap_or(AgentStatus::Active),
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok(Json(agent))
}

/// PATCH /v1/agents/:agent_id
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
        name: req.name,
        description: req.description,
        default_model_id: req.default_model_id,
        status: req.status.map(|s| s.to_string()),
    };

    let row = state
        .db
        .update_agent(agent_id, input)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update agent: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let agent = Agent {
        id: row.id,
        name: row.name,
        description: row.description,
        default_model_id: row.default_model_id,
        status: row.status.parse().unwrap_or(AgentStatus::Active),
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok(Json(agent))
}

/// POST /v1/agents/:agent_id/versions - Publish a new immutable version
#[utoipa::path(
    post,
    path = "/v1/agents/{agent_id}/versions",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    request_body = CreateAgentVersionRequest,
    responses(
        (status = 201, description = "Agent version created successfully", body = AgentVersion),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn create_agent_version(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
    Json(req): Json<CreateAgentVersionRequest>,
) -> Result<(StatusCode, Json<AgentVersion>), StatusCode> {
    let input = CreateAgentVersion {
        agent_id,
        definition: req.definition,
    };

    let row = state.db.create_agent_version(input).await.map_err(|e| {
        tracing::error!("Failed to create agent version: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let version = AgentVersion {
        agent_id: row.agent_id,
        version: row.version,
        definition: row.definition,
        created_at: row.created_at,
    };

    Ok((StatusCode::CREATED, Json(version)))
}

/// GET /v1/agents/:agent_id/versions
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/versions",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "List of agent versions", body = Vec<AgentVersion>),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn list_agent_versions(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
) -> Result<Json<Vec<AgentVersion>>, StatusCode> {
    let rows = state.db.list_agent_versions(agent_id).await.map_err(|e| {
        tracing::error!("Failed to list agent versions: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let versions: Vec<AgentVersion> = rows
        .into_iter()
        .map(|row| AgentVersion {
            agent_id: row.agent_id,
            version: row.version,
            definition: row.definition,
            created_at: row.created_at,
        })
        .collect();

    Ok(Json(versions))
}

/// GET /v1/agents/:agent_id/versions/:version
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/versions/{version}",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID"),
        ("version" = i32, Path, description = "Version number")
    ),
    responses(
        (status = 200, description = "Agent version found", body = AgentVersion),
        (status = 404, description = "Agent version not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "agents"
)]
pub async fn get_agent_version(
    State(state): State<AppState>,
    Path((agent_id, version)): Path<(Uuid, i32)>,
) -> Result<Json<AgentVersion>, StatusCode> {
    let row = state
        .db
        .get_agent_version(agent_id, version)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent version: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let version = AgentVersion {
        agent_id: row.agent_id,
        version: row.version,
        definition: row.definition,
        created_at: row.created_at,
    };

    Ok(Json(version))
}
