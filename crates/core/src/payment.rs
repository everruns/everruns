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
    pub id: PaymentAccountId,
    pub organization_id: String,
    pub owner_type: PaymentOwnerType,
    pub owner_id: String,
    pub rail: PaymentRail,
    pub label: String,
    pub public_address: Option<String>,
    pub status: PaymentStatus,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PaymentPolicy {
    pub id: PaymentPolicyId,
    pub organization_id: String,
    pub payment_account_id: PaymentAccountId,
    pub subject_type: String,
    pub subject_id: String,
    pub allowed_capabilities: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub rail_preference: Vec<PaymentRail>,
    pub max_amount_usd_per_request: Option<f64>,
    pub max_amount_usd_per_turn: Option<f64>,
    pub max_amount_usd_per_day: Option<f64>,
    pub require_approval_above_usd: Option<f64>,
    pub status: PaymentStatus,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct PaymentAttempt {
    pub id: PaymentAttemptId,
    pub organization_id: String,
    pub payment_account_id: Option<PaymentAccountId>,
    pub session_id: Option<String>,
    pub capability: String,
    pub operation: String,
    pub rail: Option<PaymentRail>,
    pub amount_usd: f64,
    pub currency: String,
    pub target_url: String,
    pub request_hash: Option<String>,
    pub status: PaymentStatus,
    pub error_message: Option<String>,
    pub receipt: serde_json::Value,
    pub created_at: DateTime<Utc>,
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
