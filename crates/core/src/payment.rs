//! Machine payment execution contract.
//!
//! Design decision: payment is an internal authority consumed by capabilities,
//! not a generic model-facing paid HTTP tool. Domain tools such as
//! `parallel_search` build typed requests and let the platform resolve wallets,
//! enforce policy, sign, settle, and record receipts.
//!
//! EVE-838: the durable accounting **records** (`PaymentAccount`,
//! `PaymentPolicy`, `PaymentAttempt`) and their value enums (`PaymentOwnerType`,
//! `PaymentStatus`) moved to the `everruns-platform` crate. The
//! capability-internal execution contract below stays in core because it is
//! bound to the [`PaymentAuthority`](crate::tool_execution::PaymentAuthority) trait and
//! `ToolContext`; `PaymentRail` and `PaymentMethod` are the value types those
//! DTOs embed.

use serde::{Deserialize, Serialize};

use crate::typed_id::PaymentAttemptId;

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
