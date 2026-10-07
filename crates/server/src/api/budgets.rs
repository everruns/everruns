// Budget HTTP routes. Every command is served by the generic `#[command(http = ..)]`
// handler; only the policy config endpoint is hand-written.
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::budgets::{
    BUDGET_MANAGE, BUDGET_VIEW, BudgetService, CheckBudget, CheckSessionBudgets, CreateBudget,
    DeleteBudget, GetBudget, ListBudgetLedger, ListBudgets, ListSessionBudgets,
    ResumeSessionBudgets, TopUpBudget, UpdateBudgetCmd,
};
use crate::domains::common::Ctx;
use crate::storage::StorageBackend;
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::budget::BudgetPeriod;
use everruns_core::{Caller, ResourceConfigResponse, evaluate_policies_with};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;

use super::command_http::CommandRouterExt;
use super::common::impl_auth_state;
use super::dispatch::impl_dispatchable;

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
        // Seed org-effective flags so the `channel_budgets` gate honors org opt-out
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
#[derive(Debug, Clone, Deserialize, ToSchema, serde::Serialize)]
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

// ============================================================================
// Routes
// ============================================================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .command::<CreateBudget>()
        .command::<ListBudgets>()
        .route("/v1/budgets/config", get(budget_config))
        .command::<GetBudget>()
        .command::<UpdateBudgetCmd>()
        .command::<DeleteBudget>()
        .command::<TopUpBudget>()
        .command::<ListBudgetLedger>()
        .command::<CheckBudget>()
        .command::<ListSessionBudgets>()
        .command::<CheckSessionBudgets>()
        .command::<ResumeSessionBudgets>()
        .with_state(state)
}

async fn budget_config(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        state.auth.permission_resolver.as_ref(),
        &caller,
        &[&BUDGET_VIEW, &BUDGET_MANAGE],
    );
    Json(ResourceConfigResponse { policies })
}
