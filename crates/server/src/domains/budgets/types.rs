// Budget domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use everruns_core::budget::BudgetPeriod;
use serde::Deserialize;
use utoipa::ToSchema;

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

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct TopUpRequest {
    /// Amount credited back to the budget balance.
    pub amount: f64,
    #[serde(default)]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
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
