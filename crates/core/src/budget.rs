// Budget domain types
//
// Extensible budgeting system for controlling resource consumption.
// Supports multiple currencies (USD, tokens, credits), pluggable meters,
// pluggable rules, and soft enforcement (pause/warn/stop).
//
// See knowledge/security/budgeting.md for the full specification.

use serde::{Deserialize, Serialize};

use crate::typed_id::BudgetId;
use crate::user_facing_error::UserFacingErrorFields;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

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
    /// Bound to an `App` (every session created for the app counts).
    App,
    /// Bound to a single `AppChannel` (only sessions for that channel count).
    AppChannel,
}

impl BudgetSubjectType {
    /// Wire string used in storage and the API.
    pub fn as_wire(&self) -> &'static str {
        match self {
            BudgetSubjectType::Session => "session",
            BudgetSubjectType::Agent => "agent",
            BudgetSubjectType::User => "user",
            BudgetSubjectType::Organization => "org",
            BudgetSubjectType::App => "app",
            BudgetSubjectType::AppChannel => "app_channel",
        }
    }
}

impl std::fmt::Display for BudgetSubjectType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl From<&str> for BudgetSubjectType {
    fn from(s: &str) -> Self {
        match s {
            "session" => BudgetSubjectType::Session,
            "agent" => BudgetSubjectType::Agent,
            "user" => BudgetSubjectType::User,
            "org" | "organization" => BudgetSubjectType::Organization,
            "app" => BudgetSubjectType::App,
            "app_channel" => BudgetSubjectType::AppChannel,
            _ => BudgetSubjectType::Session,
        }
    }
}

/// Budget period configuration for recurring budgets.
///
/// Periods drive automatic balance reset:
/// - `Duration` is a fixed-length sliding window (e.g. last 5 hours, last 30 days)
///   measured from `Budget::period_started_at`. When the window elapses the
///   balance is reset to `limit` and the window restarts.
/// - `Calendar` aligns to a calendar boundary (`hour | day | week | month | year`)
///   in UTC. The balance resets when the next boundary is crossed.
/// - `Rolling` is preserved for backwards compatibility and parses common
///   shorthand (`24h`, `5h`, `7d`, `30d`) into a `Duration`-equivalent reset
///   policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BudgetPeriod {
    /// Sliding window of a configurable number of seconds.
    Duration { seconds: u64 },
    /// Rolling window described as a humanized string ("5h", "24h", "30d").
    Rolling { window: String },
    /// Calendar-aligned (`hour`, `day`, `week`, `month`, `year`).
    Calendar { unit: String },
}

impl BudgetPeriod {
    /// Length of the period in seconds, if it can be expressed as a fixed
    /// duration. Calendar periods return `None` (handled separately).
    pub fn duration_seconds(&self) -> Option<u64> {
        match self {
            BudgetPeriod::Duration { seconds } => Some(*seconds),
            BudgetPeriod::Rolling { window } => parse_rolling_window(window),
            BudgetPeriod::Calendar { .. } => None,
        }
    }
}

/// Parse a rolling window shorthand like "5h", "30m", "7d" into seconds.
fn parse_rolling_window(window: &str) -> Option<u64> {
    let trimmed = window.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (digits, suffix) = trimmed.split_at(
        trimmed
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(trimmed.len()),
    );
    let value: u64 = digits.parse().ok()?;
    let multiplier: u64 = match suffix.trim().to_ascii_lowercase().as_str() {
        "" | "s" | "sec" | "secs" | "second" | "seconds" => 1,
        "m" | "min" | "mins" | "minute" | "minutes" => 60,
        "h" | "hr" | "hrs" | "hour" | "hours" => 3_600,
        "d" | "day" | "days" => 86_400,
        "w" | "wk" | "wks" | "week" | "weeks" => 604_800,
        _ => return None,
    };
    value.checked_mul(multiplier)
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
    /// Stable error code for user-facing budget failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// Structured interpolation fields for localized error rendering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub error_fields: Option<UserFacingErrorFields>,
}

impl BudgetCheckResult {
    pub fn ok() -> Self {
        Self {
            action: "continue".into(),
            message: None,
            budget_id: None,
            balance: None,
            currency: None,
            error_code: None,
            error_fields: None,
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
