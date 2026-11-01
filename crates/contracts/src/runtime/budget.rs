// Budget domain types
//
// Extensible budgeting system for controlling resource consumption.
// Supports multiple currencies (USD, tokens, credits), pluggable meters,
// pluggable rules, and soft enforcement (pause/warn/stop).
//
// See knowledge/security/budgeting.md for the full specification.

use serde::{Deserialize, Serialize};

use crate::runtime::typed_id::BudgetId;
use crate::runtime::user_facing_error::UserFacingErrorFields;

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
    /// Bound to a single agent trigger — the ingress a session arrived
    /// through when a trigger started it (EVE-1138). Successor to the
    /// `AppChannel` subject: migration 138 moved App webhooks onto
    /// `agent_triggers` and deleted the endpoint rows their budgets had been
    /// keyed on, leaving those caps with no structural subject until
    /// migration 153 re-keyed them onto the trigger's own id.
    AgentTrigger,
    /// Bound to a single agent channel — the exposure a session arrived
    /// through (EVE-1004). Successor to `AppChannel` for every ingress except
    /// the webhook triggers migration 138 moved off endpoints.
    #[serde(alias = "agent_endpoint")]
    AgentChannel,
    /// Bound to one session schedule (`sched_` id): the turns its fires
    /// start. Created with every schedule an agent makes (2026-11-01).
    SessionSchedule,
    /// An org's daily cap on unattended turns: schedule fires and sessions an
    /// agent trigger started. Keyed by the org's public id; one per org.
    OrgUnattended,
}

impl BudgetSubjectType {
    /// Wire string used in storage and the API.
    pub fn as_wire(&self) -> &'static str {
        match self {
            BudgetSubjectType::Session => "session",
            BudgetSubjectType::Agent => "agent",
            BudgetSubjectType::User => "user",
            BudgetSubjectType::Organization => "org",
            BudgetSubjectType::AgentTrigger => "agent_trigger",
            BudgetSubjectType::AgentChannel => "agent_channel",
            BudgetSubjectType::SessionSchedule => "session_schedule",
            BudgetSubjectType::OrgUnattended => "org_unattended",
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
            "agent_trigger" => BudgetSubjectType::AgentTrigger,
            "agent_channel" | "agent_endpoint" => BudgetSubjectType::AgentChannel,
            "session_schedule" => BudgetSubjectType::SessionSchedule,
            "org_unattended" => BudgetSubjectType::OrgUnattended,
            // Unknown wire strings, including the retired "app" and
            // "app_channel", fall back to the narrowest subject rather than the
            // widest. Migrations 151 and 153 left no such rows, so this is
            // unreachable for stored data, and mislabelling one as a session
            // budget would still bind it more tightly than mislabelling it as
            // an org budget.
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
// Usage policy (what a budget row enforces)
// ============================================================================

/// What a usage policy does when its spend crosses a step.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PolicyStepAction {
    /// Tell org owners and admins, once per window. The turn keeps going.
    Notify,
    /// Stop before the next model call until the person using the agent
    /// approves going further in this session. Unattended work has no one
    /// to ask and stops.
    Ask,
    /// Stop before the next model call.
    Stop,
}

/// One step of a usage policy, at a percent of the limit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PolicyStep {
    /// Percent of the limit at which the step applies (1 to 100).
    pub at_percent: f64,
    /// What happens when usage reaches the step.
    pub action: PolicyStepAction,
}

/// Which usage a policy counts. Absent counts everything.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PolicySource {
    #[default]
    All,
    /// Only models the host manages and bills (built-in models).
    Managed,
    /// Only the org's own provider keys.
    OwnKeys,
}

/// Name, steps and filter of a usage policy, stored on `budgets.policy`.
///
/// Decision: steps are data on the existing budget row rather than a new
/// table, so the hierarchy lookup, period rollover and gate keep working
/// unchanged. Empty steps mean the legacy shape (see [`UsagePolicy::effective_steps`]).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UsagePolicy {
    /// Display name of the policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Steps by percent of the limit. Empty: notify at 80%, ask at the soft
    /// limit, stop at the limit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<PolicyStep>,
    /// Which usage the policy counts.
    #[serde(default, skip_serializing_if = "is_all_sources")]
    pub source: PolicySource,
}

fn is_all_sources(source: &PolicySource) -> bool {
    *source == PolicySource::All
}

impl UsagePolicy {
    /// Steps in force, sorted by threshold. A policy without explicit steps
    /// keeps the behavior budgets always had: notify when 20% remains, ask
    /// at the soft limit when one is set, stop at the limit.
    pub fn effective_steps(&self, limit: f64, soft_limit: Option<f64>) -> Vec<PolicyStep> {
        let mut steps = if self.steps.is_empty() {
            let mut legacy = vec![PolicyStep {
                at_percent: 80.0,
                action: PolicyStepAction::Notify,
            }];
            if let Some(soft) = soft_limit.filter(|_| limit > 0.0) {
                legacy.push(PolicyStep {
                    at_percent: (soft / limit * 100.0).clamp(0.0, 100.0),
                    action: PolicyStepAction::Ask,
                });
            }
            legacy.push(PolicyStep {
                at_percent: 100.0,
                action: PolicyStepAction::Stop,
            });
            legacy
        } else {
            self.steps.clone()
        };
        steps.sort_by(|a, b| {
            a.at_percent
                .partial_cmp(&b.at_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        steps
    }

    /// Reject steps a person could not have meant.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(name) = &self.name
            && name.chars().count() > 120
        {
            return Err("Policy name must be at most 120 characters".into());
        }
        if self.steps.len() > 8 {
            return Err("A usage policy has at most 8 steps".into());
        }
        for step in &self.steps {
            if !(step.at_percent > 0.0 && step.at_percent <= 100.0) {
                return Err("Each step's at_percent must be above 0 and at most 100".into());
            }
        }
        Ok(())
    }

    /// The most restrictive step reached at `spent` of `limit`.
    pub fn reached(&self, limit: f64, soft_limit: Option<f64>, spent: f64) -> Option<PolicyStep> {
        if limit <= 0.0 {
            return Some(PolicyStep {
                at_percent: 100.0,
                action: PolicyStepAction::Stop,
            });
        }
        self.effective_steps(limit, soft_limit)
            .into_iter()
            .filter(|step| spent >= limit * step.at_percent / 100.0)
            .max_by(|a, b| a.action.cmp(&b.action))
    }
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
    /// Public budget id (`bdgt_...`), when the checker knows it. Budget
    /// events and the budget error fields carry it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_id: Option<String>,
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
