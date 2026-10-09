// Rows the budgets repository reads and writes.

use crate::records::{Budget, LedgerEntry};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{BudgetId, SessionId};
use everruns_core::budget::{BudgetStatus, BudgetSubjectType};
use uuid::Uuid;

/// Budget row (maps to `budgets` table).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct BudgetRow {
    pub id: Uuid,
    pub org_id: i64,
    pub subject_type: String,
    pub subject_id: String,
    pub currency: String,
    pub limit: f64,
    pub soft_limit: Option<f64>,
    pub balance: f64,
    pub period: Option<sqlx::types::JsonValue>,
    /// When the current `period` window started. NULL for one-shot budgets.
    /// Set on insert when `period` is provided, refreshed when the period
    /// rolls over.
    #[sqlx(default)]
    pub period_started_at: Option<DateTime<Utc>>,
    pub metadata: Option<sqlx::types::JsonValue>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a budget.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateBudgetRow {
    pub org_id: i64,
    pub subject_type: String,
    pub subject_id: String,
    pub currency: String,
    pub limit: f64,
    pub soft_limit: Option<f64>,
    pub period: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
}

/// Input for updating a budget.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UpdateBudgetRow {
    pub limit: Option<f64>,
    pub soft_limit: Option<Option<f64>>,
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Immutable activity journal row (maps to `usage_journal` table).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct UsageJournalRow {
    pub id: Uuid,
    pub org_id: i64,
    pub kind: String,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub event_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub turn_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub principal_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub measures: sqlx::types::JsonValue,
    pub metadata: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a journal row.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateUsageJournalRow {
    pub org_id: i64,
    pub kind: String,
    pub source_type: Option<String>,
    pub source_id: Option<String>,
    pub event_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub turn_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub principal_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub measures: serde_json::Value,
    pub metadata: serde_json::Value,
}

/// Rated ledger row (maps to `usage_ledger` table).
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct UsageLedgerRow {
    pub id: Uuid,
    pub journal_id: Uuid,
    pub budget_id: Option<Uuid>,
    pub org_id: i64,
    pub session_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub principal_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub currency: String,
    pub amount: f64,
    pub meter_source: String,
    pub ref_type: Option<String>,
    pub ref_id: Option<Uuid>,
    pub description: Option<String>,
    pub rating_metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a rated ledger row.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateUsageLedgerRow {
    pub journal_id: Uuid,
    pub budget_id: Option<Uuid>,
    pub org_id: i64,
    pub session_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub principal_id: Option<Uuid>,
    pub agent_id: Option<Uuid>,
    pub harness_id: Option<Uuid>,
    pub currency: String,
    pub amount: f64,
    pub meter_source: String,
    pub ref_type: Option<String>,
    pub ref_id: Option<Uuid>,
    pub description: Option<String>,
    pub rating_metadata: Option<serde_json::Value>,
}

/// Compatibility input for callers that still think in budget-specific postings.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreateBudgetLedgerRow {
    pub budget_id: Uuid,
    pub amount: f64,
    pub meter_source: String,
    pub ref_type: Option<String>,
    pub ref_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub description: Option<String>,
}

/// Compatibility alias for budget-scoped ledger reads.
pub type BudgetLedgerRow = UsageLedgerRow;

// The one row -> record mapping for budgets and their ledger entries.

impl From<&BudgetRow> for Budget {
    fn from(row: &BudgetRow) -> Self {
        Budget {
            id: BudgetId::from_uuid(row.id),
            organization_id: everruns_core::org_public_id_from_internal(row.org_id),
            subject_type: BudgetSubjectType::from(row.subject_type.as_str()),
            subject_id: row.subject_id.clone(),
            currency: row.currency.clone(),
            limit: row.limit,
            soft_limit: row.soft_limit,
            balance: row.balance,
            period: row
                .period
                .as_ref()
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            period_started_at: row.period_started_at,
            metadata: row.metadata.clone(),
            status: BudgetStatus::from(row.status.as_str()),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl From<&BudgetLedgerRow> for LedgerEntry {
    /// Budget APIs only return ledger rows attributed to a budget.
    fn from(row: &BudgetLedgerRow) -> Self {
        LedgerEntry {
            id: format!("ledger_{}", row.id.to_string().replace('-', "")),
            budget_id: BudgetId::from_uuid(
                row.budget_id
                    .expect("budget ledger rows returned from budget APIs always have a budget_id"),
            ),
            amount: row.amount,
            meter_source: row.meter_source.clone(),
            ref_type: row.ref_type.clone(),
            ref_id: row.ref_id.map(|id| id.to_string()),
            session_id: row.session_id.map(SessionId::from_uuid),
            description: row.description.clone(),
            created_at: row.created_at,
        }
    }
}
