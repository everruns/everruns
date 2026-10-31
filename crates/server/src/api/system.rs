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
use crate::domains::system::RunSystemDoctor;
use crate::domains::system::doctor::DoctorReport;

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/system/doctor", get(run_system_doctor))
        .with_state(state)
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
