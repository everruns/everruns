use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::Caller;
use everruns_core::reporting::{DatasetCatalog, ReportQuery, ReportResult};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use super::common::{ErrorResponse, impl_auth_state};
use super::dispatch::impl_dispatchable;
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::reporting::types::ProjectorRunResult;
use crate::domains::reporting::{
    GetReportCatalog, ReportingService, RunReportQuery, RunReportingProjector,
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

#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ProjectorRunQuery {
    #[serde(default = "default_projector_limit")]
    pub limit: i64,
}

fn default_projector_limit() -> i64 {
    100
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/reports/catalog", get(get_catalog))
        .route("/v1/reports/query", post(run_query))
        .route("/v1/reports/projector/run", post(run_projector))
        .with_state(state)
}

#[utoipa::path(
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
    post,
    path = "/v1/reports/projector/run",
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
