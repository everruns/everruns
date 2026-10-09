// Rows the payments repository reads and writes.

use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct PaymentAccountRow {
    pub id: Uuid,
    pub org_id: i64,
    pub owner_type: String,
    pub owner_id: String,
    pub rail: String,
    pub label: String,
    pub public_address: Option<String>,
    pub credential_encrypted: Option<Vec<u8>>,
    pub status: String,
    pub metadata: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreatePaymentAccountRow {
    pub owner_type: String,
    pub owner_id: String,
    pub rail: String,
    pub label: String,
    pub public_address: Option<String>,
    pub credential_encrypted: Option<Vec<u8>>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UpdatePaymentAccountRow {
    pub label: Option<String>,
    pub public_address: Option<Option<String>>,
    pub credential_encrypted: Option<Vec<u8>>,
    pub status: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct PaymentPolicyRow {
    pub id: Uuid,
    pub org_id: i64,
    pub payment_account_id: Uuid,
    pub subject_type: String,
    pub subject_id: String,
    pub allowed_capabilities: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub rail_preference: Vec<String>,
    pub max_amount_usd_per_request: Option<f64>,
    pub max_amount_usd_per_turn: Option<f64>,
    pub max_amount_usd_per_day: Option<f64>,
    pub require_approval_above_usd: Option<f64>,
    pub status: String,
    pub metadata: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreatePaymentPolicyRow {
    pub payment_account_id: Uuid,
    pub subject_type: String,
    pub subject_id: String,
    pub allowed_capabilities: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub rail_preference: Vec<String>,
    pub max_amount_usd_per_request: Option<f64>,
    pub max_amount_usd_per_turn: Option<f64>,
    pub max_amount_usd_per_day: Option<f64>,
    pub require_approval_above_usd: Option<f64>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct UpdatePaymentPolicyRow {
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

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct PaymentAttemptRow {
    pub id: Uuid,
    pub org_id: i64,
    pub payment_account_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub capability: String,
    pub operation: String,
    pub rail: Option<String>,
    pub amount_usd: f64,
    pub currency: String,
    pub target_url: String,
    pub request_hash: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub receipt: sqlx::types::JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CreatePaymentAttemptRow {
    pub payment_account_id: Option<Uuid>,
    pub session_id: Option<Uuid>,
    pub capability: String,
    pub operation: String,
    pub rail: Option<String>,
    pub amount_usd: f64,
    pub currency: String,
    pub target_url: String,
    pub request_hash: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub receipt: serde_json::Value,
}
