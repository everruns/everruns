// Capability HTTP routes
// Routes: /v1/capabilities/...
//
// Design Decision: Capabilities are defined in everruns-core via the Capability trait.
// This module provides HTTP endpoints that expose capability information from the
// CapabilityRegistry in everruns-core.
//
// Agent capabilities are managed through the agents API (POST/PATCH /v1/agents).

use crate::auth::{AuthState, ResolvedOrg};
use crate::storage::StorageBackend;
use axum::{
    Router,
    extract::{Path, Query, State},
    routing::get,
};
use everruns_core::{Caller, CapabilityInfo};
use serde::Deserialize;

use super::common::{ApiResult, PaginatedResponse, WithUrls, impl_auth_state};
use super::dispatch::{Dispatchable, impl_dispatchable};
use std::sync::Arc;

use crate::services::CapabilityService;

/// App state for capability routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, service: Arc<CapabilityService>, auth: AuthState) -> Self {
        Self { db, service, auth }
    }

    /// Build a domain Ctx from this AppState for the given org.
    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.service.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

/// Create capability routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/capabilities", get(list_capabilities))
        .route("/v1/capabilities/{capability_id}", get(get_capability))
        .with_state(state)
}

#[derive(Debug, Default, Deserialize)]
pub struct ListCapabilitiesQuery {
    pub search: Option<String>,
    pub offset: Option<u32>,
    pub limit: Option<u32>,
}

/// GET /v1/capabilities - List available capabilities with pagination
#[utoipa::path(
    get,
    path = "/v1/capabilities",
    params(
        ("search" = Option<String>, Query, description = "Search by name/description"),
        ("offset" = Option<u32>, Query, description = "Pagination offset (default: 0)"),
        ("limit" = Option<u32>, Query, description = "Page size (default: 20, max: 100)"),
    ),
    responses(
        (status = 200, description = "Paginated list of capabilities", body = PaginatedResponse<WithUrls<CapabilityInfo>>),
    ),
    tag = "capabilities"
)]
pub async fn list_capabilities(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListCapabilitiesQuery>,
) -> ApiResult<PaginatedResponse<WithUrls<CapabilityInfo>>> {
    state
        .dispatcher(&org)
        .run_paginated_with_urls(crate::domains::capabilities::ListCapabilities {
            search: query.search,
            offset: query.offset,
            limit: query.limit,
        })
        .await
}

/// GET /v1/capabilities/{capability_id} - Get a specific capability
#[utoipa::path(
    get,
    path = "/v1/capabilities/{capability_id}",
    params(
        ("capability_id" = String, Path, description = "Capability ID")
    ),
    responses(
        (status = 200, description = "Capability found", body = WithUrls<CapabilityInfo>),
        (status = 404, description = "Capability not found"),
    ),
    tag = "capabilities"
)]
pub async fn get_capability(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
) -> ApiResult<WithUrls<CapabilityInfo>> {
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::capabilities::GetCapability { id: capability_id })
        .await
}
