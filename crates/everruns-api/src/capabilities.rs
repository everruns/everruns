// Capability HTTP routes
//
// Design Decision: Capabilities are defined in everruns-core via the Capability trait.
// This module provides HTTP endpoints that expose capability information from the
// CapabilityRegistry in everruns-core.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use everruns_core::{AgentCapability, CapabilityId, CapabilityInfo};
use everruns_storage::Database;

use crate::common::{ListResponse, UpdateAgentCapabilitiesRequest};
use std::sync::Arc;
use uuid::Uuid;

use crate::services::CapabilityService;

/// App state for capability routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<CapabilityService>,
}

impl AppState {
    pub fn new(db: Arc<Database>) -> Self {
        Self {
            service: Arc::new(CapabilityService::new(db)),
        }
    }
}

/// Create capability routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/capabilities", get(list_capabilities))
        .route("/v1/capabilities/:capability_id", get(get_capability))
        .route(
            "/v1/agents/:agent_id/capabilities",
            get(get_agent_capabilities).put(set_agent_capabilities),
        )
        .with_state(state)
}

/// GET /v1/capabilities - List all available capabilities
#[utoipa::path(
    get,
    path = "/v1/capabilities",
    responses(
        (status = 200, description = "List of available capabilities", body = ListResponse<CapabilityInfo>),
    ),
    tag = "capabilities"
)]
pub async fn list_capabilities(State(state): State<AppState>) -> Json<ListResponse<CapabilityInfo>> {
    let capabilities = state.service.list_all();
    Json(ListResponse::new(capabilities))
}

/// GET /v1/capabilities/{capability_id} - Get a specific capability
#[utoipa::path(
    get,
    path = "/v1/capabilities/{capability_id}",
    params(
        ("capability_id" = String, Path, description = "Capability ID")
    ),
    responses(
        (status = 200, description = "Capability found", body = CapabilityInfo),
        (status = 404, description = "Capability not found"),
    ),
    tag = "capabilities"
)]
pub async fn get_capability(
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
) -> Result<Json<CapabilityInfo>, StatusCode> {
    let cap_id = CapabilityId::new(&capability_id);

    let capability = state.service.get(&cap_id).ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(capability))
}

/// GET /v1/agents/{agent_id}/capabilities - Get capabilities for an agent
#[utoipa::path(
    get,
    path = "/v1/agents/{agent_id}/capabilities",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    responses(
        (status = 200, description = "Agent capabilities", body = ListResponse<AgentCapability>),
        (status = 500, description = "Internal server error"),
    ),
    tag = "capabilities"
)]
pub async fn get_agent_capabilities(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
) -> Result<Json<ListResponse<AgentCapability>>, StatusCode> {
    let capabilities = state
        .service
        .get_agent_capabilities(agent_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get agent capabilities: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(ListResponse::new(capabilities)))
}

/// PUT /v1/agents/{agent_id}/capabilities - Set capabilities for an agent
#[utoipa::path(
    put,
    path = "/v1/agents/{agent_id}/capabilities",
    params(
        ("agent_id" = Uuid, Path, description = "Agent ID")
    ),
    request_body = UpdateAgentCapabilitiesRequest,
    responses(
        (status = 200, description = "Agent capabilities updated", body = ListResponse<AgentCapability>),
        (status = 400, description = "Invalid capability ID"),
        (status = 500, description = "Internal server error"),
    ),
    tag = "capabilities"
)]
pub async fn set_agent_capabilities(
    State(state): State<AppState>,
    Path(agent_id): Path<Uuid>,
    Json(req): Json<UpdateAgentCapabilitiesRequest>,
) -> Result<Json<ListResponse<AgentCapability>>, StatusCode> {
    // Validate all capability IDs exist in the registry
    for cap_id in &req.capabilities {
        if !state.service.has(cap_id) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }

    let capabilities = state
        .service
        .set_agent_capabilities(agent_id, req.capabilities)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set agent capabilities: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(ListResponse::new(capabilities)))
}
