use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Request body for the `create_payment_account` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreatePaymentAccountRequest {
    /// Principal class that owns the account (user, agent identity, or organization).
    pub owner_type: String,
    /// Prefixed identifier of the owning principal (e.g. `user_…`, `agent_…`, `org_…`).
    pub owner_id: String,
    /// Settlement rail this account operates on (e.g. `mpp_tempo`, `x402_base`).
    pub rail: String,
    /// Human-readable label. Safe to render in user-facing messages.
    pub label: String,
    /// Public address on the rail (chain address, account number, etc.). Optional; can be filled in later.
    #[serde(default)]
    pub public_address: Option<String>,
    /// Private key material for the rail. Stored encrypted; never returned in responses.
    #[serde(default)]
    pub private_key: Option<String>,
    /// Free-form metadata attached to this account (caller-defined; opaque to the platform).
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Request body for the `update_payment_account` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdatePaymentAccountRequest {
    /// New label, if changing.
    pub label: Option<String>,
    /// New public address. The outer `Option` indicates whether to update; the inner allows clearing the field.
    pub public_address: Option<Option<String>>,
    /// New private key material. Set to `Some(...)` to rotate; omit to leave unchanged.
    pub private_key: Option<String>,
    /// New lifecycle status. Valid values: `active`, `disabled`.
    pub status: Option<String>,
    /// New free-form metadata. Replaces the existing metadata blob entirely when set.
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListPaymentAccountsQuery {
    /// Filter to a single owner class (user, agent identity, or organization).
    pub owner_type: Option<String>,
    /// Filter to a specific owner principal id.
    pub owner_id: Option<String>,
}

/// Request body for the `create_payment_policy` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreatePaymentPolicyRequest {
    /// Payment account this policy authorizes spending from.
    pub payment_account_id: String,
    /// Class of subject this policy binds to (e.g. `agent_identity`, `session`).
    pub subject_type: String,
    /// Prefixed identifier of the bound subject.
    pub subject_id: String,
    /// Capability IDs this policy permits paid calls for. Empty list means no capability gating.
    #[serde(default)]
    pub allowed_capabilities: Vec<String>,
    /// HTTP host allowlist for paid outbound calls. Empty list means no host gating.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Preferred settlement rails in priority order; the authority picks the first available.
    #[serde(default)]
    pub rail_preference: Vec<String>,
    /// Maximum amount (USD) any single paid request may settle for. **Enforced** by the payment authority at policy selection. `None` means no per-request cap.
    pub max_amount_usd_per_request: Option<f64>,
    /// Maximum cumulative amount (USD) per agent turn. **Advisory only — not yet enforced.** Stored for forward compatibility; the authority currently checks only `max_amount_usd_per_request`. `None` means no per-turn cap.
    pub max_amount_usd_per_turn: Option<f64>,
    /// Maximum cumulative amount (USD) per UTC day. **Advisory only — not yet enforced.** Stored for forward compatibility; the authority currently checks only `max_amount_usd_per_request`. `None` means no per-day cap.
    pub max_amount_usd_per_day: Option<f64>,
    /// Threshold (USD) above which a request would require explicit human approval. **Advisory only — not yet enforced.** Stored for forward compatibility; no approval gate is wired up yet. `None` disables the (future) gate.
    pub require_approval_above_usd: Option<f64>,
    /// Free-form metadata attached to this policy.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Request body for the `update_payment_policy` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdatePaymentPolicyRequest {
    /// New capability allowlist. Outer `None` leaves the field unchanged.
    pub allowed_capabilities: Option<Vec<String>>,
    /// New host allowlist. Outer `None` leaves the field unchanged.
    pub allowed_hosts: Option<Vec<String>>,
    /// New rail preference order. Outer `None` leaves the field unchanged.
    pub rail_preference: Option<Vec<String>>,
    /// New per-request cap (USD). **Enforced** by the payment authority. Outer `None` leaves the field unchanged; inner `None` clears the cap.
    pub max_amount_usd_per_request: Option<Option<f64>>,
    /// New per-turn cap (USD). **Advisory only — not yet enforced.** Outer `None` leaves the field unchanged; inner `None` clears the cap.
    pub max_amount_usd_per_turn: Option<Option<f64>>,
    /// New per-day cap (USD). **Advisory only — not yet enforced.** Outer `None` leaves the field unchanged; inner `None` clears the cap.
    pub max_amount_usd_per_day: Option<Option<f64>>,
    /// New approval threshold (USD). **Advisory only — not yet enforced.** Outer `None` leaves the field unchanged; inner `None` disables the (future) gate.
    pub require_approval_above_usd: Option<Option<f64>>,
    /// New lifecycle status. Valid values: `active`, `disabled`.
    pub status: Option<String>,
    /// New free-form metadata. Replaces the existing metadata blob entirely when set.
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListPaymentPoliciesQuery {
    /// Filter to policies that authorize a specific payment account.
    pub payment_account_id: Option<String>,
    /// Filter to a single subject class.
    pub subject_type: Option<String>,
    /// Filter to a specific subject principal id.
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListPaymentAttemptsQuery {
    /// Filter to attempts originating from a specific session.
    pub session_id: Option<String>,
    /// Maximum number of attempts returned. Defaults to 50.
    #[serde(default = "default_attempt_limit")]
    pub limit: i64,
}

fn default_attempt_limit() -> i64 {
    50
}

#[derive(Debug, Serialize)]
pub struct PaymentDeleteResult {
    pub disabled: bool,
}
