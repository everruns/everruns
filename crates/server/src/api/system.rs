// System doctor route.
//
// Decision: platform users only (the `PlatformUser` extractor, plus the
// command's `system.view` policy for MCP). The report covers every server,
// worker and datastore of the install, not one organization.

use axum::{
    Router,
    extract::{Query, State},
    routing::get,
};

use super::common::{ApiResult, ErrorResponse};
use super::state::ApiState;
use crate::auth::PlatformUser;
use crate::domains::common::Command;
use crate::domains::health_issues::types::PlatformDetectionList;
use crate::domains::system::doctor::DoctorReport;
use crate::domains::system::{ListPlatformDetections, RunSystemDoctor};

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/system/doctor", get(run_system_doctor))
        .route("/v1/system/detections", get(list_platform_detections))
        .with_state(state.clone())
        // The control plane's /dp/v1 routes and org tokens ride along:
        // app_builder sits at the file-size ratchet.
        .merge(super::data_plane::routes(state))
}

/// GET /v1/system/doctor - Check the whole install
#[utoipa::path(
    get,
    path = "/v1/system/doctor",
    params(
        ("refresh" = Option<bool>, Query, description = "Run every check now instead of returning the report from the last minute")
    ),
    responses(
        (status = 200, description = "Health of every part of the install", body = DoctorReport),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Platform user access is required", body = ErrorResponse),
        (status = 503, description = "The system doctor is not available", body = ErrorResponse)
    ),
    tag = "system"
)]
pub async fn run_system_doctor(
    auth: PlatformUser,
    State(state): State<ApiState>,
    Query(query): Query<RunSystemDoctor>,
) -> ApiResult<DoctorReport> {
    Ok(axum::Json(query.run(&state.platform_ctx(&auth)).await?))
}

/// GET /v1/system/detections - Open abuse detections in every organization
#[utoipa::path(
    get,
    path = "/v1/system/detections",
    params(
        ("offset" = Option<i64>, Query, description = "Number of detections to skip"),
        ("limit" = Option<i64>, Query, description = "Page size from 1 through 100; defaults to 20")
    ),
    responses(
        (status = 200, description = "Open detections, newest first", body = PlatformDetectionList),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Platform user access is required", body = ErrorResponse)
    ),
    tag = "system"
)]
pub async fn list_platform_detections(
    auth: PlatformUser,
    State(state): State<ApiState>,
    Query(query): Query<ListPlatformDetections>,
) -> ApiResult<PlatformDetectionList> {
    Ok(axum::Json(query.run(&state.platform_ctx(&auth)).await?))
}
