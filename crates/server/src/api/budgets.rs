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
use everruns_core::budget::{BudgetCheckResult, BudgetPeriod};
use everruns_platform::{Budget, LedgerEntry};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

use super::common::{ErrorResponse, UrlBuilder, WithUrls, impl_auth_state};
use super::dispatch::{Dispatchable, impl_dispatchable};

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
        // Seed org-effective flags so the `app_budgets` gate honors org opt-out
        // rather than falling back to deployment-level `FeatureFlags::current()`.
        .with_feature_flags(org.feature_flags.clone())
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

// ============================================================================
// Request/Response types
// ============================================================================

/// Request body for creating a spending budget.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateBudgetRequest {
    /// Kind of resource constrained by the budget.
    #[schema(example = "agent")]
    pub subject_type: String,
    /// Public identifier of the constrained resource.
    #[schema(example = "agent_01933b5a00007000800000000000001")]
    pub subject_id: String,
    /// Unit in which usage and the limit are measured.
    #[schema(example = "usd")]
    pub currency: String,
    /// Hard spending ceiling for the budget.
    #[schema(example = 100.0)]
    pub limit: f64,
    /// Optional threshold that triggers a warning or pause before exhaustion.
    #[serde(default)]
    #[schema(example = 20.0)]
    pub soft_limit: Option<f64>,
    /// Optional recurring reset period for the budget balance.
    #[serde(default)]
    pub period: Option<BudgetPeriod>,
    #[serde(default)]
    /// Free-form metadata attached to this resource.
    pub metadata: Option<serde_json::Value>,
}

/// Request body for changing a spending budget.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateBudgetRequest {
    /// Replacement hard spending ceiling.
    #[schema(example = 150.0)]
    pub limit: Option<f64>,
    /// Replacement soft threshold, or null to remove the threshold.
    pub soft_limit: Option<Option<f64>>,
    /// Current lifecycle status.
    pub status: Option<String>,
    /// Free-form metadata attached to this resource.
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct TopUpRequest {
    /// Amount credited back to the budget balance.
    pub amount: f64,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
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
    state
        .dispatcher(&org)
        .run_created_with_urls(CreateBudget(req))
        .await
}

async fn get_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
) -> Result<Json<WithUrls<Budget>>, ApiError> {
    state
        .dispatcher(&org)
        .run_with_urls(GetBudget { budget_id })
        .await
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
    state
        .dispatcher(&org)
        .run_with_urls(UpdateBudgetCmd {
            budget_id,
            limit: req.limit,
            soft_limit: req.soft_limit,
            status: req.status,
            metadata: req.metadata,
        })
        .await
}

async fn delete_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .dispatcher(&org)
        .run_no_content(DeleteBudget { budget_id })
        .await
}

async fn top_up(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
    Json(req): Json<TopUpRequest>,
) -> Result<Json<WithUrls<Budget>>, ApiError> {
    state
        .dispatcher(&org)
        .run_with_urls(TopUpBudget {
            budget_id,
            amount: req.amount,
            description: req.description,
        })
        .await
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
            ErrorResponse::new(e.to_string()).into_response(StatusCode::INTERNAL_SERVER_ERROR)
        })?,
    ))
}
