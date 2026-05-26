use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::Caller;
use everruns_core::reporting::{DatasetCatalog, ReportQuery, ReportResult};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

use super::common::{ErrorResponse, ListResponse, impl_auth_state};
use super::dispatch::impl_dispatchable;
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::reporting::types::{
    CreateSavedReportRequest, ExportReportQueryRequest, ExportSavedReportRequest,
    ProjectorRunResult, ReportExport, ReportingBackfillRequest, ReportingBackfillResult,
    ReportingDiagnostics, SavedReport, UpdateSavedReportRequest,
};
use crate::domains::reporting::{
    BackfillReporting, CreateSavedReport, DeleteSavedReport, ExportReportQuery, ExportSavedReport,
    GetReportCatalog, GetReportingDiagnostics, GetSavedReport, ListSavedReports, ReportingService,
    RunReportQuery, RunReportingProjector, RunSavedReport, UpdateSavedReport,
};
use crate::storage::StorageBackend;

type ApiError = (StatusCode, Json<ErrorResponse>);

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub reporting_service: Arc<ReportingService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        let reporting_service = Arc::new(ReportingService::new(db.clone()));
        Self {
            db,
            reporting_service,
            auth,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_reporting_service(self.reporting_service.clone())
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

/// Query parameters for a manual `POST /v1/reports/projector/run` invocation —
/// the cap on how many outbox rows one run is allowed to claim.
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ProjectorRunQuery {
    #[serde(default = "default_projector_limit")]
    /// Maximum number of items returned in this page.
    pub limit: i64,
}

fn default_projector_limit() -> i64 {
    100
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/reports/catalog", get(get_catalog))
        .route("/v1/reports/query", post(run_query))
        .route("/v1/reports/query/export", post(export_query))
        .route(
            "/v1/reports/saved",
            get(list_saved_reports).post(create_saved_report),
        )
        .route(
            "/v1/reports/saved/{report_id}",
            get(get_saved_report)
                .patch(update_saved_report)
                .delete(delete_saved_report),
        )
        .route("/v1/reports/saved/{report_id}/run", post(run_saved_report))
        .route(
            "/v1/reports/saved/{report_id}/export",
            post(export_saved_report),
        )
        .route("/v1/reports/admin/diagnostics", get(get_diagnostics))
        .route("/v1/reports/admin/backfill", post(backfill_reporting))
        .route("/v1/reports/projector/run", post(run_projector))
        .with_state(state)
}

#[utoipa::path(
    description = "Return the available reporting datasets and their schemas.",
    get,
    path = "/v1/reports/catalog",
    responses(
        (status = 200, description = "Reporting semantic catalog", body = DatasetCatalog),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn get_catalog(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<DatasetCatalog>, ApiError> {
    Ok(Json(GetReportCatalog.run(&state.ctx(&org)).await?))
}

#[utoipa::path(
    description = "Run an ad-hoc reporting query against a registered dataset.",
    post,
    path = "/v1/reports/query",
    request_body = ReportQuery,
    responses(
        (status = 200, description = "Reporting query result", body = ReportResult),
        (status = 400, description = "Invalid semantic query", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn run_query(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(query): Json<ReportQuery>,
) -> Result<Json<ReportResult>, ApiError> {
    Ok(Json(RunReportQuery(query).run(&state.ctx(&org)).await?))
}

#[utoipa::path(
    description = "Run an ad-hoc reporting query and stream the result as CSV.",
    post,
    path = "/v1/reports/query/export",
    request_body = ExportReportQueryRequest,
    responses(
        (status = 200, description = "Exported report content", body = ReportExport),
        (status = 400, description = "Invalid semantic query", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn export_query(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(request): Json<ExportReportQueryRequest>,
) -> Result<Json<ReportExport>, ApiError> {
    Ok(Json(
        ExportReportQuery(request).run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(
    description = "List saved reports.",
    get,
    path = "/v1/reports/saved",
    responses(
        (status = 200, description = "Saved reports", body = ListResponse<SavedReport>),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn list_saved_reports(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<ListResponse<SavedReport>>, ApiError> {
    Ok(Json(ListResponse::new(
        ListSavedReports.run(&state.ctx(&org)).await?,
    )))
}

#[utoipa::path(
    description = "Create a new saved report.",
    post,
    path = "/v1/reports/saved",
    request_body = CreateSavedReportRequest,
    responses(
        (status = 201, description = "Saved report", body = SavedReport),
        (status = 400, description = "Invalid saved report", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn create_saved_report(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(request): Json<CreateSavedReportRequest>,
) -> Result<(StatusCode, Json<SavedReport>), ApiError> {
    Ok((
        StatusCode::CREATED,
        Json(CreateSavedReport(request).run(&state.ctx(&org)).await?),
    ))
}

#[utoipa::path(
    description = "Get a saved report by ID.",
    get,
    path = "/v1/reports/saved/{report_id}",
    params(("report_id" = Uuid, Path, description = "Saved report ID")),
    responses(
        (status = 200, description = "Saved report", body = SavedReport),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Saved report not found", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn get_saved_report(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(report_id): Path<Uuid>,
) -> Result<Json<SavedReport>, ApiError> {
    Ok(Json(
        GetSavedReport { report_id }.run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(
    description = "Update a saved report. Only provided fields are modified.",
    patch,
    path = "/v1/reports/saved/{report_id}",
    params(("report_id" = Uuid, Path, description = "Saved report ID")),
    request_body = UpdateSavedReportRequest,
    responses(
        (status = 200, description = "Saved report", body = SavedReport),
        (status = 400, description = "Invalid saved report", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Saved report not found", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn update_saved_report(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(report_id): Path<Uuid>,
    Json(request): Json<UpdateSavedReportRequest>,
) -> Result<Json<SavedReport>, ApiError> {
    Ok(Json(
        UpdateSavedReport { report_id, request }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Delete a saved report.",
    delete,
    path = "/v1/reports/saved/{report_id}",
    params(("report_id" = Uuid, Path, description = "Saved report ID")),
    responses(
        (status = 204, description = "Saved report deleted"),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Saved report not found", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn delete_saved_report(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(report_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    DeleteSavedReport { report_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    description = "Run a previously saved reporting query and return the result.",
    post,
    path = "/v1/reports/saved/{report_id}/run",
    params(("report_id" = Uuid, Path, description = "Saved report ID")),
    responses(
        (status = 200, description = "Reporting query result", body = ReportResult),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Saved report not found", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn run_saved_report(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(report_id): Path<Uuid>,
) -> Result<Json<ReportResult>, ApiError> {
    Ok(Json(
        RunSavedReport { report_id }.run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(
    description = "Run a previously saved reporting query and stream the result as CSV.",
    post,
    path = "/v1/reports/saved/{report_id}/export",
    params(("report_id" = Uuid, Path, description = "Saved report ID")),
    request_body = ExportSavedReportRequest,
    responses(
        (status = 200, description = "Exported report content", body = ReportExport),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Saved report not found", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn export_saved_report(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(report_id): Path<Uuid>,
    Json(request): Json<ExportSavedReportRequest>,
) -> Result<Json<ReportExport>, ApiError> {
    Ok(Json(
        ExportSavedReport { report_id, request }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Return diagnostics for the reporting subsystem (operator view).",
    get,
    path = "/v1/reports/admin/diagnostics",
    responses(
        (status = 200, description = "Reporting diagnostics", body = ReportingDiagnostics),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn get_diagnostics(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Result<Json<ReportingDiagnostics>, ApiError> {
    Ok(Json(GetReportingDiagnostics.run(&state.ctx(&org)).await?))
}

#[utoipa::path(
    description = "Run a reporting projector to refresh derived data (operator action).",
    post,
    path = "/v1/reports/admin/backfill",
    request_body = ReportingBackfillRequest,
    responses(
        (status = 200, description = "Reporting backfill enqueue result", body = ReportingBackfillResult),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn backfill_reporting(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(request): Json<ReportingBackfillRequest>,
) -> Result<Json<ReportingBackfillResult>, ApiError> {
    Ok(Json(
        BackfillReporting(request).run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/v1/reports/projector/run",
    description = "Run a reporting projector to refresh derived data (operator action).",
    params(ProjectorRunQuery),
    responses(
        (status = 200, description = "Reporting projector run result", body = ProjectorRunResult),
        (status = 403, description = "Forbidden", body = ErrorResponse)
    ),
    tag = "reporting"
)]
pub async fn run_projector(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ProjectorRunQuery>,
) -> Result<Json<ProjectorRunResult>, ApiError> {
    Ok(Json(
        RunReportingProjector { limit: query.limit }
            .run(&state.ctx(&org))
            .await?,
    ))
}
