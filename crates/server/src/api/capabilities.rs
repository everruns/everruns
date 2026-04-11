// Capability HTTP routes
// Routes: /v1/capabilities/...
//
// Design Decision: Capabilities are defined in everruns-core via the Capability trait.
// This module provides HTTP endpoints that expose capability information from the
// CapabilityRegistry in everruns-core.
//
// Agent capabilities are managed through the agents API (POST/PATCH /v1/agents).

use crate::auth::{AuthState, ResolvedOrg};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::{CapabilityId, CapabilityInfo};
use serde::Deserialize;

use super::common::{PaginatedResponse, impl_auth_state};
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

impl_auth_state!(AppState);

/// Create capability routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/capabilities", get(list_capabilities))
        .route("/v1/capabilities/{capability_id}", get(get_capability))
        .with_state(state)
}

const DEFAULT_LIMIT: u32 = 20;
const MAX_LIMIT: u32 = 100;

#[derive(Debug, Default, Deserialize)]
pub struct ListCapabilitiesQuery {
    pub search: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
    /// When true, return compact output (id, name, description, status only).
    #[serde(default)]
    pub summary: bool,
}

/// GET /v1/capabilities - List available capabilities with pagination
#[utoipa::path(
    get,
    path = "/v1/capabilities",
    params(
        ("search" = Option<String>, Query, description = "Search by name/description"),
        ("offset" = Option<u32>, Query, description = "Pagination offset (default: 0)"),
        ("limit" = Option<u32>, Query, description = "Page size (default: 20, max: 100)"),
        ("summary" = Option<bool>, Query, description = "Compact output (id, name, description, status only)"),
    ),
    responses(
        (status = 200, description = "Paginated list of capabilities", body = PaginatedResponse<CapabilityInfo>),
    ),
    tag = "capabilities"
)]
pub async fn list_capabilities(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListCapabilitiesQuery>,
) -> Result<Json<PaginatedResponse<CapabilityInfo>>, StatusCode> {
    let mut capabilities = state.service.list_all(org.org_id).await.map_err(|e| {
        tracing::error!("Failed to list capabilities: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Filter by search query
    if let Some(ref search) = query.search {
        capabilities.retain(|c| c.matches_search(search));
    }

    let total = capabilities.len() as u32;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).min(MAX_LIMIT);

    let data: Vec<CapabilityInfo> = capabilities
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|mut c| {
            if query.summary {
                // Strip heavy fields for compact output
                c.system_prompt = None;
                c.tool_definitions = Vec::new();
                c.dependencies = Vec::new();
                c.features = Vec::new();
            }
            c
        })
        .collect();

    Ok(Json(PaginatedResponse::new(data, total, offset, limit)))
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
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
) -> Result<Json<CapabilityInfo>, StatusCode> {
    let cap_id = CapabilityId::new(&capability_id);

    let capability = state
        .service
        .get(org.org_id, &cap_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get capability: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(capability))
}
