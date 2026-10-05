//! Outbound MCP Events webhook subscriptions (EVE-1121).

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Clone, Debug, FromRow)]
pub struct McpEventSubscriptionRow {
    pub id: Uuid,
    /// Hash of principal, callback URL, event name and canonical arguments:
    /// the subscription id returned to the client.
    pub subscription_key: String,
    pub org_id: i64,
    pub user_id: Uuid,
    pub event_name: String,
    pub arguments: serde_json::Value,
    pub callback_url: String,
    pub secret_encrypted: Vec<u8>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct UpsertMcpEventSubscription {
    pub subscription_key: String,
    pub org_id: i64,
    pub user_id: Uuid,
    pub event_name: String,
    pub arguments: serde_json::Value,
    pub callback_url: String,
    pub secret_encrypted: Vec<u8>,
    pub expires_at: DateTime<Utc>,
}
