// Capability HTTP routes
// Routes are org-scoped: /v1/orgs/:org/capabilities/...
//
// Design Decision: Capabilities are defined in everruns-core via the Capability trait.
// This module provides HTTP endpoints that expose capability information from the
// CapabilityRegistry in everruns-core.
//
// Agent capabilities are managed through the agents API (POST/PATCH /v1/orgs/{org}/agents).

use crate::auth::{AuthState, OrgContext, middleware::FromRef};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::{CapabilityId, CapabilityInfo};

use super::common::ListResponse;
use std::sync::Arc;

use crate::services::CapabilityService;

/// App state for capability routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(service: Arc<CapabilityService>, auth: AuthState) -> Self {
        Self { service, auth }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create capability routes (org-scoped)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/orgs/:org/capabilities", get(list_capabilities))
        .route(
            "/v1/orgs/:org/capabilities/:capability_id",
            get(get_capability),
        )
        .with_state(state)
}

/// GET /v1/orgs/{org}/capabilities - List all available capabilities
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/capabilities",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    responses(
        (status = 200, description = "List of available capabilities", body = ListResponse<CapabilityInfo>),
    ),
    tag = "capabilities"
)]
pub async fn list_capabilities(
    _org: OrgContext,
    State(state): State<AppState>,
) -> Result<Json<ListResponse<CapabilityInfo>>, StatusCode> {
    let capabilities = state.service.list_all().await.map_err(|e| {
        tracing::error!("Failed to list capabilities: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(ListResponse::new(capabilities)))
}

/// GET /v1/orgs/{org}/capabilities/{capability_id} - Get a specific capability
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/capabilities/{capability_id}",
    params(
        ("org" = String, Path, description = "Organization public ID"),
        ("capability_id" = String, Path, description = "Capability ID")
    ),
    responses(
        (status = 200, description = "Capability found", body = CapabilityInfo),
        (status = 404, description = "Capability not found"),
    ),
    tag = "capabilities"
)]
pub async fn get_capability(
    _org: OrgContext,
    State(state): State<AppState>,
    Path((_org_path, capability_id)): Path<(String, String)>,
) -> Result<Json<CapabilityInfo>, StatusCode> {
    let cap_id = CapabilityId::new(&capability_id);

    let capability = state
        .service
        .get(&cap_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get capability: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(capability))
}
