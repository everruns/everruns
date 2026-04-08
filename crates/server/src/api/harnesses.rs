// Harness CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)
// Policy enforcement happens at the service layer via #[policy] macro.

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::harness::{
    HARNESS_DANGEROUS, HARNESS_MANAGE, HARNESS_VIEW, merge_preview_layer, resolve_effective_harness,
};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::typed_id::{HarnessId, ModelId};
use everruns_core::{
    AgentCapabilityConfig, Caller, Harness, HarnessStatus, InitialFile, ResourceConfigResponse,
    ToolDefinition, evaluate_policies_with,
};

use super::common::{
    ApiOptionExt, ApiPolicyResultExt, ApiResult, ErrorResponse, ListResponse, impl_auth_state,
};
use super::validation::{
    validate_create_agent_input, validate_harness_name_strict, validate_update_agent_input,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

use crate::services::{CapabilityService, HarnessService};

/// Request to create a new harness
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateHarnessRequest {
    /// Name, unique per org. Lowercase alphanumeric and hyphens.
    #[schema(example = "deep-research")]
    pub name: String,
    /// Human-readable display name shown in UI.
    #[schema(example = "Deep Research")]
    pub display_name: String,
    /// Description of what the harness does.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Research harness with planning and web capabilities")]
    pub description: Option<String>,
    /// The system prompt defining the harness's base behavior.
    #[schema(example = "You are a research assistant with deep analytical capabilities.")]
    pub system_prompt: String,
    /// Optional parent harness to inherit from.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000602")]
    pub parent_harness_id: Option<HarnessId>,
    /// Default LLM model ID for this harness. Lowest priority in model chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<ModelId>,
    /// Tags for organizing harnesses.
    #[serde(default)]
    #[schema(example = json!(["research", "planning"]))]
    pub tags: Vec<String>,
    /// Capabilities to enable with per-harness configuration.
    #[serde(default)]
    #[schema(example = json!([{"ref": "current_time", "config": {}}, {"ref": "web_fetch", "config": {}}]))]
    pub capabilities: Vec<AgentCapabilityConfig>,
    /// Starter files copied into each new session for this harness.
    #[serde(default)]
    pub initial_files: Vec<InitialFile>,
    /// Network access list controlling which hosts/URLs sessions can reach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
}

/// Request to update a harness. Only provided fields will be updated.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateHarnessRequest {
    /// Name, unique per org.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "updated-research")]
    pub name: Option<String>,
    /// Human-readable display name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Updated Research Harness")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub parent_harness_id: Option<Option<HarnessId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub default_model_id: Option<ModelId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<AgentCapabilityConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_files: Option<Vec<InitialFile>>,
    /// Network access list. Send `{}` (empty object) to clear. Omit to leave unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<everruns_core::network_access::NetworkAccessList>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<HarnessStatus>,
}

/// Request to preview harness shape with capabilities applied
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PreviewHarnessRequest {
    #[schema(example = "You are a research assistant.")]
    pub system_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000602")]
    pub parent_harness_id: Option<HarnessId>,
    #[serde(default)]
    pub capabilities: Vec<AgentCapabilityConfig>,
}

/// Preview response showing merged prompt and tools
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HarnessPreviewResponse {
    pub system_prompt: String,
    #[schema(value_type = Vec<Object>)]
    pub tools: Vec<ToolDefinition>,
}

/// Query parameters for listing harnesses.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListHarnessesQuery {
    /// Search by name or description (case-insensitive substring match).
    pub search: Option<String>,
    /// Include archived harnesses. Deleted harnesses never appear in lists.
    pub include_archived: Option<bool>,
}

/// Query parameters for checking harness name availability.
#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct CheckNameQuery {
    /// The harness name to check.
    pub name: String,
    /// Optional harness ID to exclude (for edit forms where the current harness's own name is valid).
    pub exclude_id: Option<String>,
}

/// Response for name availability check.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CheckNameResponse {
    /// Whether the name is available for use.
    pub available: bool,
}

/// App state for harness routes
#[derive(Clone)]
pub struct AppState {
    pub service: Arc<HarnessService>,
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
            service: Arc::new(HarnessService::new(db)),
            capability_service,
            auth,
        }
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
    // Validate format first — if the name is invalid, it's not "available"
    if validate_harness_name_strict(&query.name).is_err() {
        return Ok(Json(CheckNameResponse { available: false }));
    }

    let exclude_id = query
        .exclude_id
        .as_deref()
        .map(|id| id.parse::<HarnessId>())
        .transpose()
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid exclude_id: {}", e),
                }),
            )
        })?;

    let caller = Caller::from(&org);
    let existing = state
        .service
        .check_name_available(&caller, &query.name, exclude_id)
        .await
        .map_policy_or_internal("check harness name")?;

    Ok(Json(CheckNameResponse {
        available: existing,
    }))
}

/// POST /v1/harnesses
#[utoipa::path(
    post,
    path = "/v1/harnesses",
    request_body = CreateHarnessRequest,
    responses(
        (status = 201, description = "Harness created", body = Harness),
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
) -> Result<(StatusCode, Json<Harness>), (StatusCode, Json<ErrorResponse>)> {
    validate_harness_name_strict(&req.name)?;
    // Reuse agent validation for display_name and other fields
    validate_create_agent_input(
        &req.name,
        Some(&req.display_name),
        req.description.as_deref(),
        &req.system_prompt,
        req.capabilities.len(),
        &req.initial_files,
    )?;

    let caller = Caller::from(&org);
    let harness = state
        .service
        .create(&caller, req)
        .await
        .map_policy_or_internal("create harness")?;

    Ok((StatusCode::CREATED, Json(harness)))
}

/// GET /v1/harnesses
#[utoipa::path(
    get,
    path = "/v1/harnesses",
    responses(
        (status = 200, description = "List of harnesses", body = ListResponse<Harness>),
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
) -> ApiResult<ListResponse<Harness>> {
    let caller = Caller::from(&org);
    let harnesses = state
        .service
        .list(
            &caller,
            query.search.as_deref(),
            query.include_archived.unwrap_or(false),
        )
        .await
        .map_policy_or_internal("list harnesses")?;

    Ok(Json(ListResponse::new(harnesses)))
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
        (status = 200, description = "Harness found", body = Harness),
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
) -> ApiResult<Harness> {
    let caller = Caller::from(&org);

    // Try parsing as a prefixed ID first; fall back to name lookup.
    if let Ok(harness_id) = harness_id_or_name.parse::<HarnessId>() {
        let harness = state
            .service
            .get(&caller, harness_id.uuid())
            .await
            .map_policy_or_internal("get harness")?
            .ok_or_not_found_json("Harness")?;
        return Ok(Json(harness));
    }

    let harness = state
        .service
        .get_by_name_or_alias(&caller, &harness_id_or_name)
        .await
        .map_policy_or_internal("get harness by name")?
        .ok_or_not_found_json("Harness")?;

    Ok(Json(harness))
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
        (status = 200, description = "Harness updated", body = Harness),
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
) -> ApiResult<Harness> {
    let harness_id: HarnessId = harness_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid harness ID: {}", e),
            }),
        )
    })?;

    if let Some(ref name) = req.name {
        validate_harness_name_strict(name)?;
    }
    // Reuse agent validation for display_name and other fields
    validate_update_agent_input(
        req.display_name.as_deref(),
        req.description.as_deref(),
        req.system_prompt.as_deref(),
        req.capabilities.as_ref().map(|c| c.len()),
        req.initial_files.as_deref(),
    )?;

    let caller = Caller::from(&org);
    let harness = state
        .service
        .update(&caller, harness_id.uuid(), req)
        .await
        .map_policy_or_internal("update harness")?
        .ok_or_not_found_json("Harness")?;

    Ok(Json(harness))
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
    let harness_id: HarnessId = harness_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid harness ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .delete(&caller, harness_id.uuid())
        .await
        .map_policy_or_internal("delete harness")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Harness not found".to_string(),
            }),
        ))
    }
}

pub async fn destroy_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(harness_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let harness_id: HarnessId = harness_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid harness ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let deleted = state
        .service
        .destroy(&caller, harness_id.uuid())
        .await
        .map_policy_or_internal("destroy harness")?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Harness not found".to_string(),
            }),
        ))
    }
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
        (status = 201, description = "Harness copied successfully", body = Harness),
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
) -> Result<(StatusCode, Json<Harness>), (StatusCode, Json<ErrorResponse>)> {
    let harness_id: HarnessId = harness_id.parse().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Invalid harness ID: {}", e),
            }),
        )
    })?;

    let caller = Caller::from(&org);
    let harness = state
        .service
        .copy(&caller, harness_id.uuid())
        .await
        .map_policy_or_internal("copy harness")?
        .ok_or_not_found_json("Harness")?;

    Ok((StatusCode::CREATED, Json(harness)))
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
    let parent = match req.parent_harness_id {
        Some(parent_harness_id) => Some(
            resolve_effective_harness(state.service.db().as_ref(), org.org_id, parent_harness_id)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to resolve harness preview parent: {}", e);
                    ErrorResponse::new("Internal server error")
                        .into_response(StatusCode::INTERNAL_SERVER_ERROR)
                })?
                .ok_or_else(|| {
                    ErrorResponse::new("Parent harness not found")
                        .into_response(StatusCode::NOT_FOUND)
                })?,
        ),
        None => None,
    };
    let (system_prompt, capabilities) =
        merge_preview_layer(parent.as_ref(), &req.system_prompt, &req.capabilities);
    let (system_prompt, tools) = state
        .capability_service
        .preview(org.org_id, &system_prompt, &capabilities)
        .await
        .map_err(|e| {
            tracing::error!("Failed to generate harness preview: {}", e);
            ErrorResponse::new("Internal server error")
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?;

    Ok(Json(HarnessPreviewResponse {
        system_prompt,
        tools,
    }))
}
