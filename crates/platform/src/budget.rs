//! Hosted budget persistence/API records.

use chrono::{DateTime, Utc};
use everruns_core::budget::{BudgetPeriod, BudgetStatus, BudgetSubjectType};
use everruns_provider::typed_id::{BudgetId, SessionId};
use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Budget — a stored spending cap for a platform subject.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Budget {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "bdgt_01933b5a00007000800000000000001"))]
    pub id: BudgetId,
    pub organization_id: String,
    pub subject_type: BudgetSubjectType,
    /// Public ID of the subject entity.
    pub subject_id: String,
    /// Currency: "usd", "tokens", "credits", or custom.
    pub currency: String,
    /// Hard limit — budget ceiling.
    pub limit: f64,
    /// Soft limit — triggers pause/warn when balance drops below this.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_limit: Option<f64>,
    /// Current remaining balance (limit minus consumed).
    pub balance: f64,
    /// Optional period for recurring budgets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<BudgetPeriod>,
    /// When the current period started (used to detect period rollover for
    /// `Duration` / `Rolling` periods, and to display "resets at" in the UI).
    /// `None` for budgets without a period.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_started_at: Option<DateTime<Utc>>,
    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub status: BudgetStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Immutable platform ledger record for resource consumption or credit.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LedgerEntry {
    pub id: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub budget_id: BudgetId,
    /// Positive = debit (consumption), negative = credit (top-up/refund).
    pub amount: f64,
    /// Which meter produced this: "llm_tokens", "tool_calls", etc.
    pub meter_source: String,
    /// Reference entity type: "llm_generation", "tool_execution", "manual".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_type: Option<String>,
    /// Reference entity ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    /// Session context for this entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}
