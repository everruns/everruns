// Budget CRUD HTTP routes
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::BudgetService;
use crate::storage::StorageBackend;
use crate::storage::models::*;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::budget::{Budget, BudgetCheckResult, BudgetPeriod, LedgerEntry};
use everruns_core::typed_id::BudgetId;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::{IntoParams, ToSchema};

use super::common::{ErrorResponse, impl_auth_state};

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
        .with_state(state)
}

// ============================================================================
// Helpers
// ============================================================================

fn err(status: StatusCode, msg: &str) -> ApiError {
    (status, Json(ErrorResponse::new(msg)))
}

fn internal(e: anyhow::Error) -> ApiError {
    err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}

fn not_found(entity: &str) -> ApiError {
    err(StatusCode::NOT_FOUND, &format!("{entity} not found"))
}

fn parse_budget_id(s: &str) -> Result<uuid::Uuid, ApiError> {
    if let Ok(id) = BudgetId::parse(s) {
        Ok(id.uuid())
    } else if let Ok(id) = uuid::Uuid::parse_str(s) {
        Ok(id)
    } else {
        Err(err(StatusCode::BAD_REQUEST, "Invalid budget ID format"))
    }
}

// ============================================================================
// Handlers
// ============================================================================

async fn create_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateBudgetRequest>,
) -> Result<(StatusCode, Json<Budget>), ApiError> {
    if !["session", "agent", "user", "org"].contains(&req.subject_type.as_str()) {
        return Err(err(StatusCode::BAD_REQUEST, "Invalid subject_type"));
    }
    let input = CreateBudgetRow {
        org_id: org.org_id,
        subject_type: req.subject_type,
        subject_id: req.subject_id,
        currency: req.currency,
        limit: req.limit,
        soft_limit: req.soft_limit,
        period: req
            .period
            .map(|p| serde_json::to_value(p).unwrap_or_default()),
        metadata: req.metadata,
    };
    let row = state.db.create_budget(input).await.map_err(internal)?;
    Ok((
        StatusCode::CREATED,
        Json(BudgetService::row_to_budget(&row)),
    ))
}

async fn get_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
) -> Result<Json<Budget>, ApiError> {
    let id = parse_budget_id(&budget_id)?;
    let row = state
        .db
        .get_budget(org.org_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("Budget"))?;
    Ok(Json(BudgetService::row_to_budget(&row)))
}

async fn list_budgets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListBudgetsQuery>,
) -> Result<Json<Vec<Budget>>, ApiError> {
    let rows = state
        .db
        .list_budgets(
            org.org_id,
            query.subject_type.as_deref(),
            query.subject_id.as_deref(),
        )
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.iter().map(BudgetService::row_to_budget).collect(),
    ))
}

async fn update_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
    Json(req): Json<UpdateBudgetRequest>,
) -> Result<Json<Budget>, ApiError> {
    let id = parse_budget_id(&budget_id)?;
    let input = UpdateBudgetRow {
        limit: req.limit,
        soft_limit: req.soft_limit,
        status: req.status,
        metadata: req.metadata,
    };
    let row = state
        .db
        .update_budget(org.org_id, id, input)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("Budget"))?;
    Ok(Json(BudgetService::row_to_budget(&row)))
}

async fn delete_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = parse_budget_id(&budget_id)?;
    let deleted = state
        .db
        .delete_budget(org.org_id, id)
        .await
        .map_err(internal)?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(not_found("Budget"))
    }
}

async fn top_up(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
    Json(req): Json<TopUpRequest>,
) -> Result<Json<Budget>, ApiError> {
    let id = parse_budget_id(&budget_id)?;
    let _budget = state
        .db
        .get_budget(org.org_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("Budget"))?;

    // Negative amount = credit (adds to balance)
    let input = CreateBudgetLedgerRow {
        budget_id: id,
        amount: -req.amount,
        meter_source: "manual".into(),
        ref_type: Some("top_up".into()),
        ref_id: None,
        session_id: None,
        description: req.description,
    };
    let (_entry, updated) = state
        .db
        .create_budget_ledger_entry(input)
        .await
        .map_err(internal)?;

    // If budget was paused/exhausted and now has balance, reactivate
    if updated.balance > 0.0 && (updated.status == "paused" || updated.status == "exhausted") {
        let _ = state.db.set_budget_status(id, "active").await;
    }

    let row = state
        .db
        .get_budget(org.org_id, id)
        .await
        .map_err(internal)?
        .unwrap();
    Ok(Json(BudgetService::row_to_budget(&row)))
}

async fn list_ledger(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<Vec<LedgerEntry>>, ApiError> {
    let id = parse_budget_id(&budget_id)?;
    let _budget = state
        .db
        .get_budget(org.org_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("Budget"))?;
    let rows = state
        .db
        .list_budget_ledger(id, query.limit, query.offset)
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.iter()
            .map(BudgetService::row_to_ledger_entry)
            .collect(),
    ))
}

async fn check_budget(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(budget_id): Path<String>,
) -> Result<Json<BudgetCheckResult>, ApiError> {
    let id = parse_budget_id(&budget_id)?;
    let budget = state
        .db
        .get_budget(org.org_id, id)
        .await
        .map_err(internal)?
        .ok_or_else(|| not_found("Budget"))?;
    let result = state
        .budget_service
        .check_budgets_for_session(org.org_id, &budget.subject_id, None)
        .await;
    Ok(Json(result))
}

// ============================================================================
// Session shortcuts
// ============================================================================

async fn list_session_budgets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<Vec<Budget>>, ApiError> {
    let rows = state
        .db
        .list_budgets(org.org_id, Some("session"), Some(&session_id))
        .await
        .map_err(internal)?;
    Ok(Json(
        rows.iter().map(BudgetService::row_to_budget).collect(),
    ))
}

async fn check_session_budgets(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<BudgetCheckResult>, ApiError> {
    let result = state
        .budget_service
        .check_budgets_for_session(org.org_id, &session_id, None)
        .await;
    Ok(Json(result))
}

async fn resume_session(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let budgets = state
        .db
        .list_budgets(org.org_id, Some("session"), Some(&session_id))
        .await
        .map_err(internal)?;

    let mut resumed = 0;
    for budget in &budgets {
        if budget.status == "paused"
            && let Ok(Some(_)) = state.db.set_budget_status(budget.id, "active").await
        {
            resumed += 1;
        }
    }

    Ok(Json(serde_json::json!({
        "resumed_budgets": resumed,
        "session_id": session_id,
    })))
}
