use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreatePaymentAccountRequest {
    pub owner_type: String,
    pub owner_id: String,
    pub rail: String,
    pub label: String,
    #[serde(default)]
    pub public_address: Option<String>,
    #[serde(default)]
    pub private_key: Option<String>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdatePaymentAccountRequest {
    pub label: Option<String>,
    pub public_address: Option<Option<String>>,
    pub private_key: Option<String>,
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListPaymentAccountsQuery {
    pub owner_type: Option<String>,
    pub owner_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreatePaymentPolicyRequest {
    pub payment_account_id: String,
    pub subject_type: String,
    pub subject_id: String,
    #[serde(default)]
    pub allowed_capabilities: Vec<String>,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub rail_preference: Vec<String>,
    pub max_amount_usd_per_request: Option<f64>,
    pub max_amount_usd_per_turn: Option<f64>,
    pub max_amount_usd_per_day: Option<f64>,
    pub require_approval_above_usd: Option<f64>,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdatePaymentPolicyRequest {
    pub allowed_capabilities: Option<Vec<String>>,
    pub allowed_hosts: Option<Vec<String>>,
    pub rail_preference: Option<Vec<String>>,
    pub max_amount_usd_per_request: Option<Option<f64>>,
    pub max_amount_usd_per_turn: Option<Option<f64>>,
    pub max_amount_usd_per_day: Option<Option<f64>>,
    pub require_approval_above_usd: Option<Option<f64>>,
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListPaymentPoliciesQuery {
    pub payment_account_id: Option<String>,
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
pub struct ListPaymentAttemptsQuery {
    pub session_id: Option<String>,
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
