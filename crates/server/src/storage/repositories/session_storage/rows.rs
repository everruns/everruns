// Session key/value, session secret (encrypted) and session-scoped MCP OAuth
// credential rows.

use crate::kernel_imports::contracts::typed_id::{SessionId, VirtualUserId};
use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Session key/value row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct SessionKeyValueRow {
    pub id: Uuid,
    pub session_id: SessionId,
    pub key: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating/updating a session key/value
#[derive(Debug, Clone)]
pub struct UpsertSessionKeyValue {
    pub session_id: SessionId,
    pub key: String,
    pub value: String,
}

/// Lightweight key info for listing (without value)
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct SessionKeyInfoRow {
    pub key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Session secret row from database
#[derive(Debug, Clone, FromRow)]
pub struct SessionSecretRow {
    #[sqlx(default)]
    pub virtual_user_id: Option<VirtualUserId>,
    pub id: Uuid,
    pub session_id: SessionId,
    pub name: String,
    pub value_encrypted: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating/updating a session secret
#[derive(Debug, Clone)]
pub struct UpsertSessionSecret {
    pub session_id: SessionId,
    pub name: String,
    pub value_encrypted: Vec<u8>,
}

/// Lightweight secret info for listing (without encrypted value)
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct SessionSecretInfoRow {
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Encrypted MCP OAuth credential bundle stored in session secrets.
#[derive(Debug, Clone)]
pub struct McpOAuthSessionCredentialsRow {
    pub virtual_user_id: Option<VirtualUserId>,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub expires_at_encrypted: Option<Vec<u8>>,
}

/// Atomic replacement for a session-scoped MCP OAuth grant.
#[derive(Debug, Clone)]
pub struct UpsertMcpOAuthSessionCredentials {
    pub virtual_user_id: Option<VirtualUserId>,
    pub session_id: SessionId,
    pub server_id: Uuid,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub expires_at_encrypted: Option<Vec<u8>>,
}
