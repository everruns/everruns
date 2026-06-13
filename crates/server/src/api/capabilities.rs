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
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{Caller, CapabilityInfo, ResourceConfigResponse, evaluate_policies_with};
use serde::Deserialize;

use super::common::{
    ApiResult, ErrorResponse, ListResponse, PaginatedResponse, WithUrls, impl_auth_state,
};
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
        .route(
            "/v1/capabilities",
            get(list_capabilities).post(create_declarative_capability),
        )
        .route(
            "/v1/capabilities/declarative",
            get(list_declarative_capabilities),
        )
        .route(
            "/v1/capabilities/declarative/config",
            get(declarative_capabilities_config),
        )
        .route(
            "/v1/capabilities/declarative/{capability_id}",
            get(get_declarative_capability)
                .patch(update_declarative_capability)
                .delete(delete_declarative_capability),
        )
        .route(
            "/v1/capabilities/declarative/{capability_id}/delete",
            post(destroy_declarative_capability),
        )
        .route(
            "/v1/capabilities/guardrails/dry-run",
            post(dry_run_guardrails),
        )
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
        (status = 400, description = "Invalid query parameters", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
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
        (status = 404, description = "Capability not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse),
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

/// GET /v1/capabilities/declarative/config
#[utoipa::path(
    get,
    path = "/v1/capabilities/declarative/config",
    responses(
        (status = 200, description = "Resource config for declarative capabilities", body = ResourceConfigResponse),
    ),
    tag = "capabilities"
)]
pub async fn declarative_capabilities_config(
    State(state): State<AppState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        state.auth.permission_resolver.as_ref(),
        &caller,
        &[
            &crate::domains::capabilities::CAPABILITY_VIEW,
            &crate::domains::capabilities::CAPABILITY_MANAGE,
            &crate::domains::capabilities::CAPABILITY_DANGEROUS,
        ],
    );
    Json(ResourceConfigResponse { policies })
}

/// POST /v1/capabilities - Create a persisted declarative capability.
#[utoipa::path(
    post,
    path = "/v1/capabilities",
    request_body = crate::domains::capabilities::types::CreateDeclarativeCapabilityRequest,
    responses(
        (status = 201, description = "Declarative capability created. Response id uses cap_<32-hex>; capability_id uses declarative:<unique_name>.", body = WithUrls<crate::domains::capabilities::types::DeclarativeCapability>),
        (status = 400, description = "Invalid declarative capability definition, name, limits, file mount, skill, or MCP server configuration.", body = ErrorResponse),
        (status = 409, description = "A declarative capability with the same unique name already exists.", body = ErrorResponse),
    ),
    tag = "capabilities"
)]
pub async fn create_declarative_capability(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<crate::domains::capabilities::types::CreateDeclarativeCapabilityRequest>,
) -> Result<
    (
        StatusCode,
        Json<WithUrls<crate::domains::capabilities::types::DeclarativeCapability>>,
    ),
    (StatusCode, Json<ErrorResponse>),
> {
    state
        .dispatcher(&org)
        .run_created_with_urls(crate::domains::capabilities::CreateDeclarativeCapability(
            req,
        ))
        .await
}

/// GET /v1/capabilities/declarative - List persisted declarative resources.
#[utoipa::path(
    get,
    path = "/v1/capabilities/declarative",
    params(
        ("search" = Option<String>, Query, description = "Search by unique name, display name, or description"),
        ("include_archived" = Option<bool>, Query, description = "Include archived declarative capabilities")
    ),
    responses(
        (status = 200, description = "Persisted declarative capability resources", body = ListResponse<WithUrls<crate::domains::capabilities::types::DeclarativeCapability>>)
    ),
    tag = "capabilities"
)]
pub async fn list_declarative_capabilities(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListDeclarativeCapabilitiesQuery>,
) -> ApiResult<ListResponse<WithUrls<crate::domains::capabilities::types::DeclarativeCapability>>> {
    state
        .dispatcher(&org)
        .run_list_with_urls(crate::domains::capabilities::ListDeclarativeCapabilities {
            search: query.search,
            include_archived: query.include_archived.unwrap_or(false),
        })
        .await
}

/// GET /v1/capabilities/declarative/{capability_id} - Get a declarative resource.
#[utoipa::path(
    get,
    path = "/v1/capabilities/declarative/{capability_id}",
    params(("capability_id" = String, Path, description = "Public declarative capability resource ID, e.g. cap_01933b5a000070008000000000000001")),
    responses(
        (status = 200, description = "Declarative capability resource", body = WithUrls<crate::domains::capabilities::types::DeclarativeCapability>),
        (status = 404, description = "Declarative capability not found", body = ErrorResponse)
    ),
    tag = "capabilities"
)]
pub async fn get_declarative_capability(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
) -> ApiResult<WithUrls<crate::domains::capabilities::types::DeclarativeCapability>> {
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::capabilities::GetDeclarativeCapability { id: capability_id })
        .await
}

/// PATCH /v1/capabilities/declarative/{capability_id} - Update a declarative resource.
#[utoipa::path(
    patch,
    path = "/v1/capabilities/declarative/{capability_id}",
    params(("capability_id" = String, Path, description = "Public declarative capability resource ID")),
    request_body = crate::domains::capabilities::types::UpdateDeclarativeCapabilityRequest,
    responses(
        (status = 200, description = "Declarative capability updated", body = WithUrls<crate::domains::capabilities::types::DeclarativeCapability>),
        (status = 400, description = "Invalid update payload or declarative capability definition", body = ErrorResponse),
        (status = 409, description = "A declarative capability with the same unique name already exists", body = ErrorResponse)
    ),
    tag = "capabilities"
)]
pub async fn update_declarative_capability(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
    Json(req): Json<crate::domains::capabilities::types::UpdateDeclarativeCapabilityRequest>,
) -> ApiResult<WithUrls<crate::domains::capabilities::types::DeclarativeCapability>> {
    state
        .dispatcher(&org)
        .run_with_urls(
            crate::domains::capabilities::UpdateDeclarativeCapabilityCmd {
                id: capability_id,
                req,
            },
        )
        .await
}

/// DELETE /v1/capabilities/declarative/{capability_id} - Archive a declarative resource.
#[utoipa::path(
    delete,
    path = "/v1/capabilities/declarative/{capability_id}",
    params(("capability_id" = String, Path, description = "Public declarative capability resource ID")),
    responses(
        (status = 200, description = "Declarative capability archived"),
        (status = 404, description = "Declarative capability not found", body = ErrorResponse)
    ),
    tag = "capabilities"
)]
pub async fn delete_declarative_capability(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
) -> ApiResult<serde_json::Value> {
    state
        .dispatcher(&org)
        .run(crate::domains::capabilities::DeleteDeclarativeCapability { id: capability_id })
        .await
}

/// POST /v1/capabilities/declarative/{capability_id}/delete - Permanently delete archived resource.
#[utoipa::path(
    post,
    path = "/v1/capabilities/declarative/{capability_id}/delete",
    params(("capability_id" = String, Path, description = "Public declarative capability resource ID")),
    responses(
        (status = 200, description = "Archived declarative capability permanently deleted"),
        (status = 404, description = "Declarative capability not found or not archived", body = ErrorResponse)
    ),
    tag = "capabilities"
)]
pub async fn destroy_declarative_capability(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(capability_id): Path<String>,
) -> ApiResult<serde_json::Value> {
    state
        .dispatcher(&org)
        .run(crate::domains::capabilities::DestroyDeclarativeCapability { id: capability_id })
        .await
}

/// POST /v1/capabilities/guardrails/dry-run - Evaluate guardrail checks against sample text.
#[utoipa::path(
    post,
    path = "/v1/capabilities/guardrails/dry-run",
    request_body = crate::domains::capabilities::types::GuardrailsDryRunRequest,
    responses(
        (status = 200, description = "Triggered checks for the sample content", body = crate::domains::capabilities::types::GuardrailsDryRunResponse),
        (status = 400, description = "Invalid guardrails config, stage, or oversized text", body = ErrorResponse),
    ),
    tag = "capabilities"
)]
pub async fn dry_run_guardrails(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<crate::domains::capabilities::types::GuardrailsDryRunRequest>,
) -> ApiResult<crate::domains::capabilities::types::GuardrailsDryRunResponse> {
    state
        .dispatcher(&org)
        .run(crate::domains::capabilities::DryRunGuardrails(req))
        .await
}

#[derive(Debug, Default, Deserialize)]
pub struct ListDeclarativeCapabilitiesQuery {
    pub search: Option<String>,
    pub include_archived: Option<bool>,
}
