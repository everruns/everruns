// Budget domain types
//
// Extensible budgeting system for controlling resource consumption.
// Supports multiple currencies (USD, tokens, credits), pluggable meters,
// pluggable rules, and soft enforcement (pause/warn/stop).
//
// See specs/budgeting.md for the full specification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::{BudgetId, SessionId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// ============================================================================
// Budget
// ============================================================================

/// Budget status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BudgetStatus {
    Active,
    Paused,
    Exhausted,
    Disabled,
}

impl std::fmt::Display for BudgetStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetStatus::Active => write!(f, "active"),
            BudgetStatus::Paused => write!(f, "paused"),
            BudgetStatus::Exhausted => write!(f, "exhausted"),
            BudgetStatus::Disabled => write!(f, "disabled"),
        }
    }
}

impl From<&str> for BudgetStatus {
    fn from(s: &str) -> Self {
        match s {
            "active" => BudgetStatus::Active,
            "paused" => BudgetStatus::Paused,
            "exhausted" => BudgetStatus::Exhausted,
            "disabled" => BudgetStatus::Disabled,
            _ => BudgetStatus::Active,
        }
    }
}

/// Subject type: what entity this budget constrains.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BudgetSubjectType {
    Session,
    Agent,
    User,
    Organization,
}

impl std::fmt::Display for BudgetSubjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetSubjectType::Session => write!(f, "session"),
            BudgetSubjectType::Agent => write!(f, "agent"),
            BudgetSubjectType::User => write!(f, "user"),
            BudgetSubjectType::Organization => write!(f, "org"),
        }
    }
}

impl From<&str> for BudgetSubjectType {
    fn from(s: &str) -> Self {
        match s {
            "session" => BudgetSubjectType::Session,
            "agent" => BudgetSubjectType::Agent,
            "user" => BudgetSubjectType::User,
            "org" | "organization" => BudgetSubjectType::Organization,
            _ => BudgetSubjectType::Session,
        }
    }
}

/// Budget period configuration for recurring budgets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BudgetPeriod {
    /// Rolling window (e.g. "last 24 hours").
    Rolling { window: String },
    /// Calendar-aligned (e.g. "per month").
    Calendar { unit: String },
}

/// Budget — a spending cap for a subject in a currency.
/// API response DTO.
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
    /// Arbitrary metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    pub status: BudgetStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ============================================================================
// Ledger Entry
// ============================================================================

/// Immutable ledger entry recording resource consumption or credit against a budget.
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

// ============================================================================
// Budget Rule Actions
// ============================================================================

/// Action returned by a budget rule after evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetAction {
    /// No action needed, continue execution.
    Continue,
    /// Emit a warning event but keep running.
    Warn { message: String },
    /// Pause the session — requires user input to resume.
    Pause { message: String },
    /// Hard stop — terminate the current turn.
    Stop { message: String },
}

// ============================================================================
// Budget check result (used by worker to decide what to do)
// ============================================================================

/// Result of checking all budgets for a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BudgetCheckResult {
    /// Most restrictive action across all budgets.
    pub action: String, // "continue", "warn", "pause", "stop"
    /// Human-readable message (set when action != "continue").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Budget that triggered the action.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub budget_id: Option<BudgetId>,
    /// Remaining balance on the most restrictive budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
    /// Currency of the most restrictive budget.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

impl BudgetCheckResult {
    pub fn ok() -> Self {
        Self {
            action: "continue".into(),
            message: None,
            budget_id: None,
            balance: None,
            currency: None,
        }
    }

    pub fn should_stop(&self) -> bool {
        self.action == "stop"
    }

    pub fn should_pause(&self) -> bool {
        self.action == "pause"
    }
}

// ============================================================================
// Budget tool response (returned by check_budget tool)
// ============================================================================

/// Summary of a single budget for the check_budget tool response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetSummary {
    pub currency: String,
    pub limit: f64,
    pub balance: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub soft_limit: Option<f64>,
    pub percent_remaining: f64,
    pub status: String,
}

/// Full response from the check_budget tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetToolResponse {
    /// Overall status: "active", "warning", "paused", "exhausted", "no_budgets"
    pub status: String,
    /// Per-budget summaries
    pub budgets: Vec<BudgetSummary>,
    /// Human-readable hint for the agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}
