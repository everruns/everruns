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
    pub subject_id: String,
    pub currency: String,
    pub limit: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_limit: Option<f64>,
    pub balance: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<BudgetPeriod>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period_started_at: Option<DateTime<Utc>>,
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
    pub amount: f64,
    pub meter_source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ref_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}
