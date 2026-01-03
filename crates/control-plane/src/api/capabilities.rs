// Capability HTTP routes
//
// Design Decision: Capabilities are defined in everruns-core via the Capability trait.
// This module provides HTTP endpoints that expose capability information from the
// CapabilityRegistry in everruns-core.
//
// Agent capabilities are managed through the agents API (POST/PATCH /v1/agents).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use everruns_core::{CapabilityId, CapabilityInfo};

use super::common::ListResponse;
use std::sync::Arc;

use crate::services::CapabilityService;

/// App state for capability routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<CapabilityService>,
}

impl AppState {
    pub fn new(service: Arc<CapabilityService>) -> Self {
        Self { service }
    }
}

/// Create capability routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/capabilities", get(list_capabilities))
        .route("/v1/capabilities/:capability_id", get(get_capability))
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
pub async fn list_capabilities(
    State(state): State<AppState>,
) -> Json<ListResponse<CapabilityInfo>> {
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
