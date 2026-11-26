// Rows the agent channels repository reads and writes.

use crate::storage::UpdateField;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Stored agent channel configuration.
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct AgentChannelRow {
    pub id: Uuid,
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
    /// keeps lifecycle status consistent.
    pub status: Option<String>,
}
