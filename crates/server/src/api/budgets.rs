// Budget CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::budgets::BudgetService;
use crate::domains::budgets::{
    CheckBudget, CheckSessionBudgets, CreateBudget, DeleteBudget, GetBudget, ListAppBudgets,
    ListBudgetLedger, ListBudgets, ListSessionBudgets, ResumeSessionBudgets, TopUpBudget,
    UpdateBudgetCmd,
};
use crate::domains::common::{Command, Ctx};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::Caller;
use everruns_core::budget::{Budget, BudgetCheckResult, BudgetPeriod, LedgerEntry};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

use super::common::{ErrorResponse, UrlBuilder, WithUrls, impl_auth_state};

type ApiError = (StatusCode, Json<ErrorResponse>);

// ============================================================================
// AppState
// ============================================================================

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub budget_service: Arc<BudgetService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        budget_service: Arc<BudgetService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            budget_service,
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
    }
}

impl_auth_state!(AppState);

// ============================================================================
// Request/Response types
// ============================================================================

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateBudgetRequest {
    pub subject_type: String,
    pub subject_id: String,
    pub currency: String,
    pub limit: f64,
    #[serde(default)]
    pub soft_limit: Option<f64>,
    #[serde(default)]
    pub period: Option<BudgetPeriod>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateBudgetRequest {
    pub limit: Option<f64>,
    pub soft_limit: Option<Option<f64>>,
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct TopUpRequest {
    pub amount: f64,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListBudgetsQuery {
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct LedgerQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

// ============================================================================
// Routes
// ============================================================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/budgets", post(create_budget).get(list_budgets))
        .route(
            "/v1/budgets/{budget_id}",
            get(get_budget).patch(update_budget).delete(delete_budget),
        )
        .route("/v1/budgets/{budget_id}/top-up", post(top_up))
        .route("/v1/budgets/{budget_id}/ledger", get(list_ledger))
        .route("/v1/budgets/{budget_id}/check", get(check_budget))
        .route(
            "/v1/sessions/{session_id}/budgets",
            get(list_session_budgets),
        )
        .route(
            "/v1/sessions/{session_id}/budget-check",
            get(check_session_budgets),
        )
        .route("/v1/sessions/{session_id}/resume", post(resume_session))
        .route("/v1/apps/{app_id}/budgets", get(list_app_budgets))
        .with_state(state)
}

async fn create_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateBudgetRequest>,
) -> Result<(StatusCode, Json<WithUrls<Budget>>), ApiError> {
    let row = CreateBudget(req).run(&state.ctx(&org)).await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok((StatusCode::CREATED, Json(urls.wrap(row))))
}

async fn get_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
) -> Result<Json<WithUrls<Budget>>, ApiError> {
    let row = GetBudget { budget_id }.run(&state.ctx(&org)).await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(row)))
}

async fn list_budgets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListBudgetsQuery>,
) -> Result<Json<Vec<WithUrls<Budget>>>, ApiError> {
    let budgets = ListBudgets {
        subject_type: query.subject_type,
        subject_id: query.subject_id,
    }
    .run(&state.ctx(&org))
    .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap_vec(budgets)))
}

async fn update_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
    Json(req): Json<UpdateBudgetRequest>,
) -> Result<Json<WithUrls<Budget>>, ApiError> {
    let row = UpdateBudgetCmd {
        budget_id,
        limit: req.limit,
        soft_limit: req.soft_limit,
        status: req.status,
        metadata: req.metadata,
    }
    .run(&state.ctx(&org))
    .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(row)))
}

async fn delete_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    DeleteBudget { budget_id }.run(&state.ctx(&org)).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn top_up(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
    Json(req): Json<TopUpRequest>,
) -> Result<Json<WithUrls<Budget>>, ApiError> {
    let row = TopUpBudget {
        budget_id,
        amount: req.amount,
        description: req.description,
    }
    .run(&state.ctx(&org))
    .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap(row)))
}

async fn list_ledger(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<Vec<LedgerEntry>>, ApiError> {
    Ok(Json(
        ListBudgetLedger {
            budget_id,
            limit: query.limit,
            offset: query.offset,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

async fn check_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
) -> Result<Json<BudgetCheckResult>, ApiError> {
    Ok(Json(CheckBudget { budget_id }.run(&state.ctx(&org)).await?))
}

// ============================================================================
// Session shortcuts
// ============================================================================

async fn list_session_budgets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<WithUrls<Budget>>>, ApiError> {
    let budgets = ListSessionBudgets { session_id }
        .run(&state.ctx(&org))
        .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap_vec(budgets)))
}

async fn check_session_budgets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<BudgetCheckResult>, ApiError> {
    Ok(Json(
        CheckSessionBudgets { session_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListAppBudgetsQuery {
    #[serde(default = "default_include_channels")]
    pub include_channels: bool,
}

fn default_include_channels() -> bool {
    true
}

async fn list_app_budgets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(app_id): Path<String>,
    Query(query): Query<ListAppBudgetsQuery>,
) -> Result<Json<Vec<WithUrls<Budget>>>, ApiError> {
    let budgets = ListAppBudgets {
        app_id,
        include_channels: query.include_channels,
    }
    .run(&state.ctx(&org))
    .await?;
    let urls = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(urls.wrap_vec(budgets)))
}

async fn resume_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        serde_json::to_value(
            ResumeSessionBudgets { session_id }
                .run(&state.ctx(&org))
                .await?,
        )
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(e.to_string())),
            )
        })?,
    ))
}
