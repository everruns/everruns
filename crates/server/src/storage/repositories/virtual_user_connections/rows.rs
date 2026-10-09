// Virtual user (agent identity) and organization connection rows.

use crate::kernel_imports::contracts::typed_id::VirtualUserId;
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Agent identity connection row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct VirtualUserConnectionRow {
    pub id: Uuid,
    pub virtual_user_id: VirtualUserId,
    pub provider: String,
    pub owner_scope: String,
    pub name: Option<String>,
    pub connection_type: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub installation_id: Option<i64>,
    /// Provider-specific metadata (e.g. Deno org slug for personal tokens)
    pub provider_metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating an virtual user connection
#[derive(Debug, Clone)]
pub struct CreateVirtualUserConnectionRow {
    pub virtual_user_id: VirtualUserId,
    pub provider: String,
    pub connection_type: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub installation_id: Option<i64>,
    /// Provider-specific metadata (e.g. Deno org slug for personal tokens)
    pub provider_metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct CreateOrganizationConnectionRow {
    pub org_id: i64,
    pub name: String,
    pub provider: String,
    pub access_token_encrypted: Vec<u8>,
    pub provider_username: Option<String>,
    pub provider_metadata: Option<serde_json::Value>,
}
