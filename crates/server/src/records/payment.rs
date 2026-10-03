// Payment accounting records (platform domain aggregates).
//
// Moved out of `everruns-core` in EVE-838. These are the durable accounting
// entities — payment accounts, policies, and settlement attempts — plus the
// value enums they embed (`PaymentOwnerType`, `PaymentStatus`).
//
// The capability-internal execution contract stays in `everruns-core`:
// `PaymentRail`, `PaymentMethod`, `MachinePaymentRequest`, and
// `MachinePaymentResponse` are referenced by core's `PaymentAuthority` trait
// (`crates/core/src/tool_execution.rs`) and its `ToolContext`, so they cannot move
// without a forbidden `core -> platform` edge. These records embed
// `everruns_core::payment::PaymentRail` (direction: platform -> core), and the
// execution-contract types are re-exported below for a unified payment surface.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use everruns_contracts::typed_id::{PaymentAccountId, PaymentAttemptId, PaymentPolicyId};

// Re-export the capability-internal execution contract that remains in core, so
// consumers reach the whole payment surface through `crate::records::payment`.
// `PaymentRail` is also used by the record structs below.
pub use everruns_core::payment::{
    MachinePaymentRequest, MachinePaymentResponse, PaymentMethod, PaymentRail,
};

use utoipa::ToSchema;

/// Principal class that owns a payment account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PaymentOwnerType {
    User,
    VirtualUser,
    Organization,
}

impl PaymentOwnerType {
    pub fn as_wire(&self) -> &'static str {
        match self {
            PaymentOwnerType::User => "user",
            PaymentOwnerType::VirtualUser => "virtual_user",
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
            "virtual_user" => PaymentOwnerType::VirtualUser,
            "organization" => PaymentOwnerType::Organization,
            _ => PaymentOwnerType::User,
        }
    }
}

/// Lifecycle state of a payment account, policy, or attempt. The shared
/// vocabulary keeps account/policy admin and attempt settlement on the
/// same status taxonomy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
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

/// A payment account — the org-scoped source of funds for paid agent calls.
/// Each account binds an owning principal (user, virtual user, or org)
/// to one settlement rail and tracks its provisioning lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PaymentAccount {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: PaymentAccountId,
    /// Owning organization's prefixed public identifier.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "org_01933b5a000070008000000000000001")
    )]
    pub organization_id: String,
    /// Principal class that owns this account (user, virtual user, or organization).
    pub owner_type: PaymentOwnerType,
    /// Prefixed identifier of the owning principal (e.g. `user_…`, `agent_…`, `org_…`).
    #[cfg_attr(
        feature = "openapi",
        schema(example = "agent_01933b5a000070008000000000000001")
    )]
    pub owner_id: String,
    /// Settlement rail this account operates on.
    pub rail: PaymentRail,
    /// Human-readable label for this account. Safe to render in user-facing messages.
    #[schema(example = "Production USDC ops wallet")]
    pub label: String,
    /// Public address on the rail (chain address, account number, etc.). Optional; `None` until provisioning completes.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8")
    )]
    pub public_address: Option<String>,
    /// Current lifecycle status of this account.
    pub status: PaymentStatus,
    /// Free-form metadata attached to this account (caller-defined; opaque to the platform).
    #[schema(example = json!({"team": "finance"}))]
    pub metadata: serde_json::Value,
    /// Timestamp when this account was created (RFC 3339).
    #[schema(example = "2026-04-01T10:00:00Z")]
    pub created_at: DateTime<Utc>,
    /// Timestamp when this account was last updated (RFC 3339).
    #[schema(example = "2026-05-20T14:00:00Z")]
    pub updated_at: DateTime<Utc>,
}

/// A payment policy — the binding between a paying account and a subject
/// (virtual user, session) that controls which paid calls are
/// authorized and at what spend caps.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PaymentPolicy {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: PaymentPolicyId,
    /// Owning organization's prefixed public identifier.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "org_01933b5a000070008000000000000001")
    )]
    pub organization_id: String,
    /// Payment account this policy authorizes spending from.
    pub payment_account_id: PaymentAccountId,
    /// Class of subject this policy binds to (e.g. `virtual_user`, `session`).
    #[schema(example = "virtual_user")]
    pub subject_type: String,
    /// Prefixed identifier of the bound subject.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "identity_01933b5a000070008000000000000001")
    )]
    pub subject_id: String,
    /// Capability IDs this policy permits paid calls for. Empty list means no capability gating.
    #[schema(example = json!(["paid_search", "paid_image_gen"]))]
    pub allowed_capabilities: Vec<String>,
    /// HTTP host allowlist for paid outbound calls. Empty list means no host gating.
    #[schema(example = json!(["api.openai.com", "api.anthropic.com"]))]
    pub allowed_hosts: Vec<String>,
    /// Preferred settlement rails in priority order; the authority picks the first available.
    pub rail_preference: Vec<PaymentRail>,
    /// Maximum amount (USD) any single paid request may settle for. **Enforced** by the payment authority at policy selection. `None` means no per-request cap.
    #[schema(example = 2.5)]
    pub max_amount_usd_per_request: Option<f64>,
    /// Maximum cumulative amount (USD) per agent turn. **Advisory only — not yet enforced.** Stored on the policy for forward compatibility; the payment authority currently checks only `max_amount_usd_per_request`. `None` means no per-turn cap.
    #[schema(example = 5.0)]
    pub max_amount_usd_per_turn: Option<f64>,
    /// Maximum cumulative amount (USD) per UTC day. **Advisory only — not yet enforced.** Stored on the policy for forward compatibility; the payment authority currently checks only `max_amount_usd_per_request`. `None` means no per-day cap.
    #[schema(example = 50.0)]
    pub max_amount_usd_per_day: Option<f64>,
    /// Threshold (USD) above which a request would require explicit human approval. **Advisory only — not yet enforced.** Stored on the policy for forward compatibility; no approval gate is wired up yet. `None` disables the (future) gate.
    #[schema(example = 10.0)]
    pub require_approval_above_usd: Option<f64>,
    /// Current lifecycle status of this policy.
    pub status: PaymentStatus,
    /// Free-form metadata attached to this policy.
    #[schema(example = json!({"created_by": "alex@acme.example"}))]
    pub metadata: serde_json::Value,
    /// Timestamp when this policy was created (RFC 3339).
    #[schema(example = "2026-04-01T10:00:00Z")]
    pub created_at: DateTime<Utc>,
    /// Timestamp when this policy was last updated (RFC 3339).
    #[schema(example = "2026-05-20T14:00:00Z")]
    pub updated_at: DateTime<Utc>,
}

/// A single paid-call settlement attempt — the durable record of one
/// authorization+settlement cycle issued through the payment authority.
/// Persisted regardless of outcome so failed attempts remain auditable.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PaymentAttempt {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    pub id: PaymentAttemptId,
    /// Owning organization's prefixed public identifier.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "org_01933b5a000070008000000000000001")
    )]
    pub organization_id: String,
    /// Payment account that settled (or attempted to settle) this attempt. `None` if no account could be resolved.
    pub payment_account_id: Option<PaymentAccountId>,
    /// Session that initiated the paid call, if any.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "session_01933b5a000070008000000000000001")
    )]
    pub session_id: Option<String>,
    /// Capability ID that originated this paid call.
    #[schema(example = "paid_search")]
    pub capability: String,
    /// Capability-specific operation name that originated this paid call.
    #[schema(example = "search.query")]
    pub operation: String,
    /// Settlement rail actually used. `None` if the attempt failed before rail selection.
    pub rail: Option<PaymentRail>,
    /// Amount actually charged (USD).
    #[schema(example = 0.014)]
    pub amount_usd: f64,
    /// ISO 4217 currency code for the charge (typically `USD`).
    #[schema(example = "USD")]
    pub currency: String,
    /// Destination URL of the paid outbound call.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "https://api.example.com/v1/search")
    )]
    pub target_url: String,
    /// Stable hash of the outbound request used to detect replays. `None` when not applicable.
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "sha256:9f1e2a4c3d5b6e8a0b2c4d6e8f0a1b3c5d7e9f0a1b2c4d6e8f0a1b2c4d6e8f0a"
        )
    )]
    pub request_hash: Option<String>,
    /// Current lifecycle status of this attempt.
    pub status: PaymentStatus,
    /// Human-readable error message when `status` is `failed`; `None` otherwise.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "rail.insufficient_funds: settled balance below minimum")
    )]
    pub error_message: Option<String>,
    /// Rail-specific receipt payload (transaction id, block reference, signature, etc.).
    #[schema(example = json!({"tx_hash": "0x4a1c2b3d", "block": 18234567}))]
    pub receipt: serde_json::Value,
    /// Timestamp when this attempt was created (RFC 3339).
    #[schema(example = "2026-05-25T10:14:00Z")]
    pub created_at: DateTime<Utc>,
    /// Timestamp when this attempt was last updated (RFC 3339).
    #[schema(example = "2026-05-25T10:14:02Z")]
    pub updated_at: DateTime<Utc>,
}
