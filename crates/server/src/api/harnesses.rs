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
use everruns_core::{
    AgentCapabilityConfig, Caller, DeploymentGrade, ResourceConfigResponse, evaluate_policies_with,
};
use everruns_host::HostComposition;
use everruns_platform::Harness;

use super::common::{
    ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse, ListResponse, ResourceStatsResponse,
    ResourceWithCounts, UrlBuilder, WithUrls, impl_auth_state,
};
use super::dispatch::{Dispatchable, impl_dispatchable};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use utoipa::{IntoParams, ToSchema};

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
    pub grade: DeploymentGrade,
    pub host_composition: Arc<HostComposition>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
        grade: DeploymentGrade,
        host_composition: Arc<HostComposition>,
    ) -> Self {
        Self {
            db,
            capability_service,
            auth,
            grade,
            host_composition,
        }
    }

    /// Build a domain Ctx from this AppState for the given org.
    pub fn ctx(&self, org: &ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

async fn add_harness_counts(
    db: &StorageBackend,
    org_id: i64,
    harness: Harness,
) -> Result<ResourceWithCounts<Harness>, (StatusCode, Json<ErrorResponse>)> {
    let session_count = async {
        db.count_sessions_for_harness(org_id, harness.id)
            .await
            .log_internal_error_json("count harness sessions")
    };
    let app_count = async {
        db.count_apps_for_harness(org_id, harness.id)
            .await
            .log_internal_error_json("count harness apps")
    };
    let (session_count, app_count) = tokio::try_join!(session_count, app_count)?;

    Ok(ResourceWithCounts {
        session_count,
        app_count,
        inner: harness,
    })
}

async fn add_harnesses_counts(
    db: &StorageBackend,
    org_id: i64,
    harnesses: Vec<Harness>,
) -> Result<Vec<ResourceWithCounts<Harness>>, (StatusCode, Json<ErrorResponse>)> {
    let harness_ids = harnesses
        .iter()
        .map(|harness| harness.id)
        .collect::<Vec<_>>();
    let session_counts = async {
        db.count_sessions_for_harnesses(org_id, &harness_ids)
            .await
            .log_internal_error_json("count harness sessions")
    };
    let app_counts = async {
        db.count_apps_for_harnesses(org_id, &harness_ids)
            .await
            .log_internal_error_json("count harness apps")
    };
    let (session_counts, app_counts) = tokio::try_join!(session_counts, app_counts)?;
    let session_counts = session_counts.into_iter().collect::<HashMap<_, _>>();
    let app_counts = app_counts.into_iter().collect::<HashMap<_, _>>();

    Ok(harnesses
        .into_iter()
        .map(|harness| ResourceWithCounts {
            session_count: session_counts.get(&harness.id).copied().unwrap_or(0) as u64,
            app_count: app_counts.get(&harness.id).copied().unwrap_or(0) as u64,
            inner: harness,
        })
        .collect())
}

/// Create harness routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/harnesses", post(create_harness).get(list_harnesses))
        .route("/v1/harnesses/check-name", get(check_harness_name))
        .route("/v1/harnesses/config", get(harness_config))
        .route("/v1/harnesses/preview", post(preview_harness))
        .route("/v1/harnesses/import", post(import_harness))
        .route(
            "/v1/harnesses/{harness_id}",
            get(get_harness)
                .patch(update_harness)
                .delete(delete_harness),
        )
        .route("/v1/harnesses/{harness_id}/stats", get(get_harness_stats))
        .route("/v1/harnesses/{harness_id}/delete", post(destroy_harness))
        .route("/v1/harnesses/{harness_id}/copy", post(copy_harness))
        .with_state(state)
}

/// Query parameters for `POST /v1/harnesses/import`.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ImportHarnessQuery {
    /// Import from a built-in harness example by name (e.g. `data-analyst`).
    /// Required: the only currently supported import mode is from-example.
    #[serde(rename = "from-example")]
    pub from_example: String,
}

/// POST /v1/harnesses/import - Adopt a harness example as a regular org-owned harness.
///
/// Creates a normal harness (`is_built_in = false`) from a code-defined
/// example. The new harness inherits from the org's `generic` harness by
/// name — no UUIDs are hardcoded. Capabilities and other config flow through
/// the same validation paths as `POST /v1/harnesses`.
///
/// Returns 201 on creation. If the example name collides with an existing
/// harness in the org, a random alphanumeric suffix is appended (parity with
/// agent example import).
#[utoipa::path(
    post,
    path = "/v1/harnesses/import",
    params(ImportHarnessQuery),
    responses(
        (status = 201, description = "Harness adopted from example", body = WithUrls<Harness>),
        (status = 400, description = "Invalid input or missing capabilities", body = ErrorResponse),
        (status = 404, description = "Example not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "harnesses"
)]
pub async fn import_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ImportHarnessQuery>,
) -> Result<(StatusCode, Json<WithUrls<Harness>>), (StatusCode, Json<ErrorResponse>)> {
    let name = query.from_example.trim().to_string();
    if name.is_empty() {
        return Err(
            ErrorResponse::new("`from-example` query parameter must not be empty")
                .into_response(StatusCode::BAD_REQUEST),
        );
    }

    let example = crate::harnesses::find_harness_example(&name)
        .ok_or_else(|| ErrorResponse::not_found(&format!("harness example '{name}'")))?;

    if example.dev_only && !state.grade.experimental_features_enabled() {
        return Err(ErrorResponse::not_found(&format!(
            "harness example '{name}'"
        )));
    }

    let missing: Vec<&str> = example
        .definition
        .capabilities
        .iter()
        .map(|c| c.capability_id())
        .filter(|id| !state.host_composition.capability_registry().has(id))
        .collect();
    if !missing.is_empty() {
        return Err(ErrorResponse::new(format!(
            "Example requires unregistered capabilities: {missing:?}"
        ))
        .into_response(StatusCode::BAD_REQUEST));
    }

    // Resolve parent harness ID by name within this org. Examples are required
    // to inherit by name (no hardcoded UUIDs).
    let parent_harness_id = if let Some(parent_name) = example.definition.parent_name.as_deref() {
        let parent_row = state
            .db
            .get_harness_by_name(org.org_id, parent_name)
            .await
            .log_internal_error_json("resolve parent harness for example import")?
            .ok_or_else(|| {
                ErrorResponse::new(format!(
                    "Required parent harness `{parent_name}` is not provisioned for this org"
                ))
                .into_response(StatusCode::INTERNAL_SERVER_ERROR)
            })?;
        Some(parent_row.id)
    } else {
        None
    };

    let capabilities: Vec<AgentCapabilityConfig> = example
        .definition
        .capabilities
        .iter()
        .map(|cap| {
            AgentCapabilityConfig::with_config(cap.typed_id().clone(), cap.config_value().clone())
        })
        .collect();

    // Pick a non-colliding name with random suffix retry — same strategy as
    // agent example import. Concurrency-safe enough for a manual UI action.
    let unique_name = {
        use rand::RngExt;
        let base = example.definition.name.as_str();
        let mut selected = None;

        for attempt in 0..10 {
            let candidate = if attempt == 0 {
                base.to_string()
            } else {
                let suffix: String = rand::rng()
                    .sample_iter(&rand::distr::Alphanumeric)
                    .take(5)
                    .map(|c| (c as char).to_ascii_lowercase())
                    .collect();
                format!("{base}-{suffix}")
            };

            let result = crate::domains::harnesses::CheckHarnessName {
                name: candidate.clone(),
                exclude_id: None,
            }
            .run(&state.ctx(&org))
            .await?;

            if result.available {
                selected = Some(candidate);
                break;
            }
        }

        selected.ok_or_else(ErrorResponse::internal_error)?
    };

    let req = CreateHarnessRequest {
        name: unique_name,
        display_name: Some(example.definition.display_name.clone()),
        description: Some(example.definition.description.clone()),
        system_prompt: Some(example.definition.system_prompt.clone()),
        parent_harness_id,
        default_model_id: None,
        tags: example.definition.tags.clone(),
        capabilities,
        initial_files: vec![],
        mcp_servers: Default::default(),
        network_access: None,
        embedder_metadata: Default::default(),
    };

    let harness = crate::domains::harnesses::CreateHarness(req)
        .run(&state.ctx(&org))
        .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(urls.wrap(harness))))
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
    .run(&state.ctx(&org))
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
    state
        .dispatcher(&org)
        .run_created_with_urls(crate::domains::harnesses::CreateHarness(req))
        .await
}

/// GET /v1/harnesses
#[utoipa::path(
    get,
    path = "/v1/harnesses",
    responses(
        (status = 200, description = "List of harnesses", body = ListResponse<WithUrls<ResourceWithCounts<Harness>>>),
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
) -> ApiResult<ListResponse<WithUrls<ResourceWithCounts<Harness>>>> {
    let harnesses = crate::domains::harnesses::ListHarnesses {
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
    }
    .run(&state.ctx(&org))
    .await?;

    let harnesses = add_harnesses_counts(&state.db, org.org_id, harnesses).await?;
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
        (status = 200, description = "Harness found", body = WithUrls<ResourceWithCounts<Harness>>),
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
) -> ApiResult<WithUrls<ResourceWithCounts<Harness>>> {
    let harness = crate::domains::harnesses::GetHarness {
        id: harness_id_or_name,
    }
    .run(&state.ctx(&org))
    .await?;
    let harness = add_harness_counts(&state.db, org.org_id, harness).await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(harness)))
}

/// GET /v1/harnesses/{harness_id}/stats - Get aggregate usage stats for a harness
#[utoipa::path(
    get,
    path = "/v1/harnesses/{harness_id}/stats",
    params(
        ("harness_id" = String, Path, description = "Harness ID (prefixed) or name")
    ),
    responses(
        (status = 200, description = "Harness aggregate stats", body = ResourceStatsResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Harness not found", body = ErrorResponse),
        (status = 500, description = "Internal server error", body = ErrorResponse)
    ),
    tag = "harnesses"
)]
pub async fn get_harness_stats(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(harness_id_or_name): Path<String>,
) -> ApiResult<ResourceStatsResponse> {
    let harness = crate::domains::harnesses::GetHarness {
        id: harness_id_or_name,
    }
    .run(&state.ctx(&org))
    .await?;
    let row = state
        .db
        .get_harness(org.org_id, harness.id)
        .await
        .log_internal_error_json("get harness for stats")?
        .filter(|row| row.status != "deleted")
        .ok_or_not_found_json("Harness")?;
    let stats = state
        .db
        .session_aggregate_stats(org.org_id, None, Some(row.id))
        .await
        .log_internal_error_json("get harness stats")?;

    Ok(Json(stats.into()))
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
    state
        .dispatcher(&org)
        .run_with_urls(crate::domains::harnesses::UpdateHarnessCmd {
            id: harness_id,
            req,
        })
        .await
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
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::harnesses::DeleteHarness { id: harness_id })
        .await
}

pub async fn destroy_harness(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(harness_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(crate::domains::harnesses::DestroyHarness { id: harness_id })
        .await
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
    state
        .dispatcher(&org)
        .run_created_with_urls(crate::domains::harnesses::CopyHarness { id: harness_id })
        .await
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
        system_prompt: req.system_prompt,
        parent_harness_id: req.parent_harness_id,
        capabilities: req.capabilities,
        mcp_servers: req.mcp_servers,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(HarnessPreviewResponse {
        system_prompt: result.system_prompt,
        tools: result.tools,
    }))
}
