// Harness CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
// Policy enforcement happens at the service layer via #[policy] macro.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::Command;
use crate::domains::harnesses::types::{
    CheckNameQuery, CheckNameResponse, CreateHarnessRequest, HarnessPreviewResponse,
    PreviewHarnessRequest, UpdateHarnessRequest,
};
use crate::domains::harnesses::{HARNESS_DANGEROUS, HARNESS_MANAGE, HARNESS_VIEW};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::{Caller, Harness, ResourceConfigResponse, evaluate_policies_with};

use super::common::{
    ApiResult, ErrorResponse, ListResponse, UrlBuilder, WithUrls, impl_auth_state,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::IntoParams;

use crate::services::CapabilityService;

/// Query parameters for listing harnesses.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListHarnessesQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived harnesses. Deleted harnesses never appear in lists.
    pub include_archived: Option<bool>,
}

/// App state for harness routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            capability_service,
            auth,
        }
    }

    /// Build a domain Ctx from this AppState for the given org.
    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            None,
        )
    }
}

impl_auth_state!(AppState);

/// Create harness routes (no import/export)
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/harnesses", post(create_harness).get(list_harnesses))
        .route("/v1/harnesses/check-name", get(check_harness_name))
        .route("/v1/harnesses/config", get(harness_config))
        .route("/v1/harnesses/preview", post(preview_harness))
        .route(
            "/v1/harnesses/{harness_id}",
            get(get_harness)
                .patch(update_harness)
                .delete(delete_harness),
        )
        .route("/v1/harnesses/{harness_id}/delete", post(destroy_harness))
        .route("/v1/harnesses/{harness_id}/copy", post(copy_harness))
        .with_state(state)
}

/// GET /v1/harnesses/config
///
/// Returns which harness policies the caller satisfies.
/// UI uses this to show/hide controls (e.g. delete button).
#[utoipa::path(
    get,
    path = "/v1/harnesses/config",
    responses(
        (status = 200, description = "Resource config for harnesses", body = ResourceConfigResponse),
    ),
    tag = "harnesses"
)]
pub async fn harness_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&HARNESS_VIEW, &HARNESS_MANAGE, &HARNESS_DANGEROUS],
    );
    Json(ResourceConfigResponse { policies })
}

/// GET /v1/harnesses/check-name
///
/// Returns whether a harness name is available for use. Optionally excludes
/// a specific harness ID (for edit forms where the harness's own name is valid).
#[utoipa::path(
    get,
    path = "/v1/harnesses/check-name",
    params(CheckNameQuery),
    responses(
        (status = 200, description = "Name availability result", body = CheckNameResponse),
        (status = 400, description = "Invalid exclude_id", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
    ),
    tag = "harnesses"
)]
pub async fn check_harness_name(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<CheckNameQuery>,
) -> Result<Json<CheckNameResponse>, (StatusCode, Json<ErrorResponse>)> {
    let result = crate::domains::harnesses::CheckHarnessName {
        name: query.name,
        exclude_id: query.exclude_id,
    }
    .execute(&state.ctx(&org))
    .await?;
    Ok(Json(CheckNameResponse {
        available: result.available,
    }))
}

/// POST /v1/harnesses
#[utoipa::path(
    post,
    path = "/v1/harnesses",
    request_body = CreateHarnessRequest,
    responses(
        (status = 201, description = "Harness created", body = WithUrls<Harness>),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "harnesses"
)]
pub async fn create_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateHarnessRequest>,
) -> Result<(StatusCode, Json<WithUrls<Harness>>), (StatusCode, Json<ErrorResponse>)> {
    let harness = crate::domains::harnesses::CreateHarness(req)
        .execute(&state.ctx(&org))
        .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(urls.wrap(harness))))
}

/// GET /v1/harnesses
#[utoipa::path(
    get,
    path = "/v1/harnesses",
    responses(
        (status = 200, description = "List of harnesses", body = ListResponse<WithUrls<Harness>>),
        (status = 403, description = "Forbidden"),
        (status = 500, description = "Internal server error")
    ),
    params(ListHarnessesQuery),
    tag = "harnesses"
)]
pub async fn list_harnesses(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListHarnessesQuery>,
) -> ApiResult<ListResponse<WithUrls<Harness>>> {
    let harnesses = crate::domains::harnesses::ListHarnesses {
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
    }
    .execute(&state.ctx(&org))
    .await?;

    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(ListResponse::new(harnesses).with_urls(&urls)))
}

/// GET /v1/harnesses/{harness_id}
///
/// Accepts either a harness ID (e.g. `harness_01933b5a...`) or a
/// name (e.g. `generic`). The virtual name `default` resolves to the org's
/// configured default harness. Names are resolved within the caller's org.
#[utoipa::path(
    get,
    path = "/v1/harnesses/{harness_id}",
    params(
        ("harness_id" = String, Path, description = "Harness ID (prefixed) or name")
    ),
    responses(
        (status = 200, description = "Harness found", body = WithUrls<Harness>),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Harness not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "harnesses"
)]
pub async fn get_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(harness_id_or_name): Path<String>,
) -> ApiResult<WithUrls<Harness>> {
    let harness = crate::domains::harnesses::GetHarness {
        id: harness_id_or_name,
    }
    .execute(&state.ctx(&org))
    .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(harness)))
}

/// PATCH /v1/harnesses/{harness_id}
#[utoipa::path(
    patch,
    path = "/v1/harnesses/{harness_id}",
    params(
        ("harness_id" = String, Path, description = "Harness ID (prefixed)")
    ),
    request_body = UpdateHarnessRequest,
    responses(
        (status = 200, description = "Harness updated", body = WithUrls<Harness>),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Harness not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "harnesses"
)]
pub async fn update_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(harness_id): Path<String>,
    Json(req): Json<UpdateHarnessRequest>,
) -> ApiResult<WithUrls<Harness>> {
    let harness = crate::domains::harnesses::UpdateHarnessCmd {
        id: harness_id,
        req,
    }
    .execute(&state.ctx(&org))
    .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(harness)))
}

/// DELETE /v1/harnesses/{harness_id}
#[utoipa::path(
    delete,
    path = "/v1/harnesses/{harness_id}",
    params(
        ("harness_id" = String, Path, description = "Harness ID (prefixed)")
    ),
    responses(
        (status = 204, description = "Harness archived"),
        (status = 400, description = "Invalid harness ID"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Harness not found"),
        (status = 500, description = "Internal server error")
    ),
    tag = "harnesses"
)]
pub async fn delete_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(harness_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    crate::domains::harnesses::DeleteHarness { id: harness_id }
        .execute(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn destroy_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(harness_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    crate::domains::harnesses::DestroyHarness { id: harness_id }
        .execute(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/harnesses/{harness_id}/copy - Copy a harness
///
/// Creates a new harness with the same configuration as the source harness.
/// The new harness's name will be "{original name} (copy)".
#[utoipa::path(
    post,
    path = "/v1/harnesses/{harness_id}/copy",
    params(
        ("harness_id" = String, Path, description = "Source harness ID to copy")
    ),
    responses(
        (status = 201, description = "Harness copied successfully", body = WithUrls<Harness>),
        (status = 400, description = "Invalid harness ID", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Source harness not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "harnesses"
)]
pub async fn copy_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(harness_id): Path<String>,
) -> Result<(StatusCode, Json<WithUrls<Harness>>), (StatusCode, Json<ErrorResponse>)> {
    let harness = crate::domains::harnesses::CopyHarness { id: harness_id }
        .execute(&state.ctx(&org))
        .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(urls.wrap(harness))))
}

/// POST /v1/harnesses/preview
#[utoipa::path(
    post,
    path = "/v1/harnesses/preview",
    request_body = PreviewHarnessRequest,
    responses(
        (status = 200, description = "Harness preview generated", body = HarnessPreviewResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "harnesses"
)]
pub async fn preview_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<PreviewHarnessRequest>,
) -> ApiResult<HarnessPreviewResponse> {
    let result = crate::domains::harnesses::PreviewHarness {
        system_prompt: Some(req.system_prompt),
        parent_harness_id: req.parent_harness_id,
        capabilities: req.capabilities,
        mcp_servers: req.mcp_servers,
    }
    .execute(&state.ctx(&org))
    .await?;

    Ok(Json(HarnessPreviewResponse {
        system_prompt: result.system_prompt,
        tools: result.tools,
    }))
}
