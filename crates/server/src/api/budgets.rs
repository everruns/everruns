// Budget HTTP routes. Every command is served by the generic `#[command(http = ..)]`
// handler; only the policy config endpoint is hand-written.
// Routes use ResolvedOrg: org derived from auth context (API key or cookie)

use crate::auth::{AuthState, ResolvedOrg};
pub use crate::domains::budgets::types::{CreateBudgetRequest, TopUpRequest, UpdateBudgetRequest};
use crate::domains::budgets::{
    BUDGET_MANAGE, BUDGET_VIEW, BudgetService, CheckBudget, CheckSessionBudgets, CreateBudget,
    DeleteBudget, GetBudget, ListBudgetLedger, ListBudgets, ListSessionBudgets,
    ResumeSessionBudgets, TopUpBudget, UpdateBudgetCmd,
};
use crate::domains::common::Ctx;
use crate::storage::StorageBackend;
use axum::{Json, Router, extract::State, routing::get};
use everruns_core::{Caller, ResourceConfigResponse, evaluate_policies_with};
use std::sync::Arc;

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
