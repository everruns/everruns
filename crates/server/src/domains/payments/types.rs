use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Request body for the `create_payment_account` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreatePaymentAccountRequest {
    /// Principal class that owns the account. One of: `user`, `agent_identity`, `organization`.
    /// The prefix on `owner_id` must match this: `user` → `user_…`, `agent_identity` → `identity_…`,
    /// `organization` → `org_…`.
    #[schema(example = "agent_identity")]
    pub owner_type: String,
    /// Prefixed identifier of the owning principal. Must use the prefix that matches `owner_type`
    /// (see above). Example below pairs with `owner_type = "agent_identity"`.
    #[schema(example = "identity_01933b5a00007000800000000000001")]
    pub owner_id: String,
    /// Settlement rail this account operates on. One of: `mpp_tempo`, `x402_base`.
    #[schema(example = "x402_base")]
    pub rail: String,
    /// Human-readable label. Safe to render in user-facing messages.
    #[schema(example = "Refund agent · USDC on Base")]
    pub label: String,
    /// Public address on the rail (chain address, account number, etc.). Optional; can be filled in later.
    #[serde(default)]
    #[schema(example = "0x742d35Cc6634C0532925a3b844Bc454e4438f44e")]
    pub public_address: Option<String>,
    /// Private key material for the rail. Stored encrypted; never returned in responses.
    /// Example shown as an obvious placeholder — supply a real 32-byte hex value at create time.
    #[serde(default)]
    #[schema(example = "0x<your-32-byte-hex-private-key>")]
    pub private_key: Option<String>,
    /// Free-form metadata attached to this account (caller-defined; opaque to the platform).
    /// Example: `{"team": "support", "cost_center": "ops"}`.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Request body for the `update_payment_account` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdatePaymentAccountRequest {
    /// New label, if changing.
    #[schema(example = "Refund agent · USDC on Base (prod)")]
    pub label: Option<String>,
    /// New public address. The outer `Option` indicates whether to update; the inner allows clearing the field.
    pub public_address: Option<Option<String>>,
    /// New private key material. Set to `Some(...)` to rotate; omit to leave unchanged.
    /// Example shown as an obvious placeholder — supply a real 32-byte hex value when rotating.
    #[schema(example = "0x<your-32-byte-hex-private-key>")]
    pub private_key: Option<String>,
    /// New lifecycle status. Valid values: `active`, `disabled`.
    #[schema(example = "disabled")]
    pub status: Option<String>,
    /// New free-form metadata. Replaces the existing metadata blob entirely when set.
    /// Example: `{"team": "support", "cost_center": "ops-2026"}`.
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
    /// Payment account this policy authorizes spending from. Accepts a prefixed `payacct_…`
    /// identifier or a bare UUID.
    #[schema(example = "payacct_01933b5a00007000800000000000001")]
    pub payment_account_id: String,
    /// Class of subject this policy binds to. One of: `user`, `agent_identity`, `agent`, `app`, `session`, `org`.
    /// The prefix on `subject_id` must match: `user`→`user_…`, `agent_identity`→`identity_…`,
    /// `agent`→`agent_…`, `app`→`app_…`, `session`→`session_…`, `org`→`org_…`.
    #[schema(example = "agent_identity")]
    pub subject_type: String,
    /// Prefixed identifier of the bound subject. Must use the prefix matching `subject_type`
    /// (see above). Example below pairs with `subject_type = "agent_identity"`.
    #[schema(example = "identity_01933b5a00007000800000000000001")]
    pub subject_id: String,
    /// Capability IDs this policy permits paid calls for. Empty list means no capability gating.
    #[serde(default)]
    #[schema(example = json!(["weather.lookup", "shipping.quote"]))]
    pub allowed_capabilities: Vec<String>,
    /// HTTP host allowlist for paid outbound calls. Empty list means no host gating.
    #[serde(default)]
    #[schema(example = json!(["api.shippo.com", "api.openweathermap.org"]))]
    pub allowed_hosts: Vec<String>,
    /// Preferred settlement rails in priority order; the authority picks the first available.
    #[serde(default)]
    #[schema(example = json!(["x402_base", "mpp_tempo"]))]
    pub rail_preference: Vec<String>,
    /// Maximum amount (USD) any single paid request may settle for. **Enforced** by the payment authority at policy selection. `None` means no per-request cap.
    #[schema(example = 5.0)]
    pub max_amount_usd_per_request: Option<f64>,
    /// Maximum cumulative amount (USD) per agent turn. **Advisory only — not yet enforced.** Stored for forward compatibility; the authority currently checks only `max_amount_usd_per_request`. `None` means no per-turn cap.
    #[schema(example = 20.0)]
    pub max_amount_usd_per_turn: Option<f64>,
    /// Maximum cumulative amount (USD) per UTC day. **Advisory only — not yet enforced.** Stored for forward compatibility; the authority currently checks only `max_amount_usd_per_request`. `None` means no per-day cap.
    #[schema(example = 100.0)]
    pub max_amount_usd_per_day: Option<f64>,
    /// Threshold (USD) above which a request would require explicit human approval. **Advisory only — not yet enforced.** Stored for forward compatibility; no approval gate is wired up yet. `None` disables the (future) gate.
    #[schema(example = 10.0)]
    pub require_approval_above_usd: Option<f64>,
    /// Free-form metadata attached to this policy.
    /// Example: `{"owner_team": "ops", "ticket": "OPS-1248"}`.
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Request body for the `update_payment_policy` operation.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdatePaymentPolicyRequest {
    /// New capability allowlist. Outer `None` leaves the field unchanged.
    #[schema(example = json!(["weather.lookup", "shipping.quote", "currency.convert"]))]
    pub allowed_capabilities: Option<Vec<String>>,
    /// New host allowlist. Outer `None` leaves the field unchanged.
    #[schema(example = json!(["api.shippo.com", "api.openweathermap.org", "api.exchangerate.host"]))]
    pub allowed_hosts: Option<Vec<String>>,
    /// New rail preference order. Outer `None` leaves the field unchanged.
    #[schema(example = json!(["mpp_tempo", "x402_base"]))]
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
    #[schema(example = "disabled")]
    pub status: Option<String>,
    /// New free-form metadata. Replaces the existing metadata blob entirely when set.
    /// Example: `{"owner_team": "ops", "ticket": "OPS-1248", "review_due": "2026-09-01"}`.
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
