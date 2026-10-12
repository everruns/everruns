// User connection rows. `UpdateOAuthConnectionTokens` also rotates virtual user
// connections (`virtual_user_connections`); it sits with the user rows, which
// it was written for.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// User connection row from database
#[derive(Debug, Clone, FromRow)]
pub struct UserConnectionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub provider: String,
    pub connection_type: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    /// Encrypted OAuth token (NULL for GitHub App connections)
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    /// GitHub App installation ID (tokens minted on demand)
    pub installation_id: Option<i64>,
    /// Provider-specific metadata (e.g. Deno org slug for personal tokens)
    pub provider_metadata: Option<sqlx::types::JsonValue>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Input for creating a user connection
#[derive(Debug, Clone)]
pub struct CreateUserConnectionRow {
    pub user_id: Uuid,
    pub provider: String,
    pub connection_type: String,
    pub provider_user_id: Option<String>,
    pub provider_username: Option<String>,
    /// Encrypted OAuth token (None for GitHub App connections)
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    /// GitHub App installation ID (tokens minted on demand)
    pub installation_id: Option<i64>,
    /// Provider-specific metadata (e.g. Deno org slug for personal tokens)
    pub provider_metadata: Option<serde_json::Value>,
}

/// Atomic rotation of a persistent OAuth connection.
#[derive(Debug, Clone)]
pub struct UpdateOAuthConnectionTokens {
    pub connection_id: Uuid,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Vec<u8>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Option<String>,
}
