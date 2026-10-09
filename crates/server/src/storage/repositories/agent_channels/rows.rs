// Rows the agent channels repository reads and writes.

use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// `agent_channels` row as read through its archival `app_id` link.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentChannelRow {
    pub id: Uuid,
    pub app_id: Uuid,
    pub public_id: String,
    pub channel_type: String,
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub auth: Option<serde_json::Value>,
    pub auth_encrypted: Option<Vec<u8>>,
    pub durable_schedule_id: Option<Uuid>,
    pub enabled: bool,
    /// Per-endpoint lifecycle; authoritative for ingress (EVE-1007).
    #[sqlx(default)]
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an endpoint under a legacy App alias (agent derived from the App).
#[derive(Debug, Clone)]
pub struct CreateLegacyAliasChannelRow {
    pub public_id: String,
    pub channel_type: String,
    pub channel_config: serde_json::Value,
    pub channel_config_encrypted: Option<Vec<u8>>,
    pub auth: Option<serde_json::Value>,
    pub auth_encrypted: Option<Vec<u8>>,
    pub durable_schedule_id: Option<Uuid>,
    pub enabled: bool,
}

/// Input for updating an endpoint by its internal row id.
#[derive(Debug, Clone, Default)]
pub struct UpdateChannelByIdRow {
    pub channel_type: Option<String>,
    pub channel_config: Option<serde_json::Value>,
    pub channel_config_encrypted: UpdateField<Vec<u8>>,
    pub auth: UpdateField<serde_json::Value>,
    pub auth_encrypted: UpdateField<Vec<u8>>,
    pub durable_schedule_id: UpdateField<Uuid>,
    pub enabled: Option<bool>,
    /// Set the endpoint lifecycle directly. When `None`, an `enabled` change
    /// still moves `status` between `disabled` and the App's publish state so
    /// the two cannot drift while the App API is still the everyday control.
    pub status: Option<String>,
}
