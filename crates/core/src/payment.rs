//! Machine payment DTOs and internal execution contract.
//!
//! Design decision: payment is an internal authority consumed by capabilities,
//! not a generic model-facing paid HTTP tool. Domain tools such as
//! `parallel_search` build typed requests and let the platform resolve wallets,
//! enforce policy, sign, settle, and record receipts.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::{PaymentAccountId, PaymentAttemptId, PaymentPolicyId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Payment rail used to settle a machine payment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PaymentRail {
    MppTempo,
    X402Base,
}

impl PaymentRail {
    pub fn as_wire(&self) -> &'static str {
        match self {
            PaymentRail::MppTempo => "mpp_tempo",
            PaymentRail::X402Base => "x402_base",
        }
    }
}

impl std::fmt::Display for PaymentRail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl std::str::FromStr for PaymentRail {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "mpp_tempo" => Ok(PaymentRail::MppTempo),
            "x402_base" => Ok(PaymentRail::X402Base),
            _ => Err(format!("Invalid payment rail: {value}")),
        }
    }
}

impl From<&str> for PaymentRail {
    fn from(value: &str) -> Self {
        match value {
            "x402_base" => PaymentRail::X402Base,
            _ => PaymentRail::MppTempo,
        }
    }
}

/// Principal class that owns a payment account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PaymentOwnerType {
    User,
    AgentIdentity,
    Organization,
}

impl PaymentOwnerType {
    pub fn as_wire(&self) -> &'static str {
        match self {
            PaymentOwnerType::User => "user",
            PaymentOwnerType::AgentIdentity => "agent_identity",
            PaymentOwnerType::Organization => "organization",
        }
    }
}

impl std::fmt::Display for PaymentOwnerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl From<&str> for PaymentOwnerType {
    fn from(value: &str) -> Self {
        match value {
            "agent_identity" => PaymentOwnerType::AgentIdentity,
            "organization" => PaymentOwnerType::Organization,
            _ => PaymentOwnerType::User,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PaymentStatus {
    Active,
    Disabled,
    Pending,
    Succeeded,
    Failed,
    Released,
}

impl PaymentStatus {
    pub fn as_wire(&self) -> &'static str {
        match self {
            PaymentStatus::Active => "active",
            PaymentStatus::Disabled => "disabled",
            PaymentStatus::Pending => "pending",
            PaymentStatus::Succeeded => "succeeded",
            PaymentStatus::Failed => "failed",
            PaymentStatus::Released => "released",
        }
    }
}

impl std::fmt::Display for PaymentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl From<&str> for PaymentStatus {
    fn from(value: &str) -> Self {
        match value {
            "disabled" => PaymentStatus::Disabled,
            "pending" => PaymentStatus::Pending,
            "succeeded" => PaymentStatus::Succeeded,
            "failed" => PaymentStatus::Failed,
            "released" => PaymentStatus::Released,
            _ => PaymentStatus::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PaymentAccount {
    /// Prefixed public identifier (see `specs/id-schema.md`).
    pub id: PaymentAccountId,
    /// Owning organization's prefixed public identifier.
    pub organization_id: String,
    /// Principal class that owns this account (user, agent identity, or organization).
    pub owner_type: PaymentOwnerType,
    /// Prefixed identifier of the owning principal (e.g. `user_…`, `agent_…`, `org_…`).
    pub owner_id: String,
    /// Settlement rail this account operates on.
    pub rail: PaymentRail,
    /// Human-readable label for this account. Safe to render in user-facing messages.
    pub label: String,
    /// Public address on the rail (chain address, account number, etc.). Optional; `None` until provisioning completes.
    pub public_address: Option<String>,
    /// Current lifecycle status of this account.
    pub status: PaymentStatus,
    /// Free-form metadata attached to this account (caller-defined; opaque to the platform).
    pub metadata: serde_json::Value,
    /// Timestamp when this account was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this account was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PaymentPolicy {
    /// Prefixed public identifier (see `specs/id-schema.md`).
    pub id: PaymentPolicyId,
    /// Owning organization's prefixed public identifier.
    pub organization_id: String,
    /// Payment account this policy authorizes spending from.
    pub payment_account_id: PaymentAccountId,
    /// Class of subject this policy binds to (e.g. `agent_identity`, `session`).
    pub subject_type: String,
    /// Prefixed identifier of the bound subject.
    pub subject_id: String,
    /// Capability IDs this policy permits paid calls for. Empty list means no capability gating.
    pub allowed_capabilities: Vec<String>,
    /// HTTP host allowlist for paid outbound calls. Empty list means no host gating.
    pub allowed_hosts: Vec<String>,
    /// Preferred settlement rails in priority order; the authority picks the first available.
    pub rail_preference: Vec<PaymentRail>,
    /// Maximum amount (USD) any single paid request may settle for. **Enforced** by the payment authority at policy selection. `None` means no per-request cap.
    pub max_amount_usd_per_request: Option<f64>,
    /// Maximum cumulative amount (USD) per agent turn. **Advisory only — not yet enforced.** Stored on the policy for forward compatibility; the payment authority currently checks only `max_amount_usd_per_request`. `None` means no per-turn cap.
    pub max_amount_usd_per_turn: Option<f64>,
    /// Maximum cumulative amount (USD) per UTC day. **Advisory only — not yet enforced.** Stored on the policy for forward compatibility; the payment authority currently checks only `max_amount_usd_per_request`. `None` means no per-day cap.
    pub max_amount_usd_per_day: Option<f64>,
    /// Threshold (USD) above which a request would require explicit human approval. **Advisory only — not yet enforced.** Stored on the policy for forward compatibility; no approval gate is wired up yet. `None` disables the (future) gate.
    pub require_approval_above_usd: Option<f64>,
    /// Current lifecycle status of this policy.
    pub status: PaymentStatus,
    /// Free-form metadata attached to this policy.
    pub metadata: serde_json::Value,
    /// Timestamp when this policy was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this policy was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PaymentAttempt {
    /// Prefixed public identifier (see `specs/id-schema.md`).
    pub id: PaymentAttemptId,
    /// Owning organization's prefixed public identifier.
    pub organization_id: String,
    /// Payment account that settled (or attempted to settle) this attempt. `None` if no account could be resolved.
    pub payment_account_id: Option<PaymentAccountId>,
    /// Session that initiated the paid call, if any.
    pub session_id: Option<String>,
    /// Capability ID that originated this paid call.
    pub capability: String,
    /// Capability-specific operation name that originated this paid call.
    pub operation: String,
    /// Settlement rail actually used. `None` if the attempt failed before rail selection.
    pub rail: Option<PaymentRail>,
    /// Amount actually charged (USD).
    pub amount_usd: f64,
    /// ISO 4217 currency code for the charge (typically `USD`).
    pub currency: String,
    /// Destination URL of the paid outbound call.
    pub target_url: String,
    /// Stable hash of the outbound request used to detect replays. `None` when not applicable.
    pub request_hash: Option<String>,
    /// Current lifecycle status of this attempt.
    pub status: PaymentStatus,
    /// Human-readable error message when `status` is `failed`; `None` otherwise.
    pub error_message: Option<String>,
    /// Rail-specific receipt payload (transaction id, block reference, signature, etc.).
    pub receipt: serde_json::Value,
    /// Timestamp when this attempt was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this attempt was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// HTTP method for an internal paid request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum PaymentMethod {
    Get,
    Post,
}

impl PaymentMethod {
    pub fn as_wire(&self) -> &'static str {
        match self {
            PaymentMethod::Get => "GET",
            PaymentMethod::Post => "POST",
        }
    }
}

impl std::fmt::Display for PaymentMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_wire())
    }
}

impl std::str::FromStr for PaymentMethod {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "GET" => Ok(PaymentMethod::Get),
            "POST" => Ok(PaymentMethod::Post),
            _ => Err(format!("Invalid payment method: {value}")),
        }
    }
}

/// Internal request from a capability to the payment authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachinePaymentRequest {
    pub capability: String,
    pub operation: String,
    pub method: PaymentMethod,
    pub url: String,
    pub body: Option<serde_json::Value>,
    pub max_amount_usd: f64,
    pub rail_preference: Vec<PaymentRail>,
    pub metadata: serde_json::Value,
}

/// Response returned to the calling capability after payment and execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MachinePaymentResponse {
    pub attempt_id: Option<PaymentAttemptId>,
    pub amount_usd: f64,
    pub rail: Option<PaymentRail>,
    pub response: serde_json::Value,
    pub receipt: serde_json::Value,
}
