// Auth session, personal access token, refresh token, CLI auth and MCP OAuth 2.1 rows.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Auth session row (legacy, kept for backwards compatibility)
#[derive(Debug, Clone, FromRow)]
pub struct AuthSessionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Personal access token row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct PersonalAccessTokenRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub scopes: sqlx::types::JsonValue,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub metadata: sqlx::types::JsonValue,
}

/// Refresh token row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct RefreshTokenRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an auth session (legacy)
#[derive(Debug, Clone)]
pub struct CreateAuthSessionRow {
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Input for creating a personal access token
#[derive(Debug, Clone)]
pub struct CreatePersonalAccessTokenRow {
    pub user_id: Uuid,
    pub name: String,
    pub token_hash: String,
    pub token_prefix: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: serde_json::Value,
}

/// CLI auth session row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct CliAuthSessionRow {
    pub id: Uuid,
    pub state: String,
    pub exchange_code: String,
    pub user_id: Option<Uuid>,
    pub redirect_port: i32,
    pub completed: bool,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating a CLI auth session
#[derive(Debug, Clone)]
pub struct CreateCliAuthSessionRow {
    pub state: String,
    pub exchange_code: String,
    pub redirect_port: i32,
    pub expires_at: DateTime<Utc>,
}

/// Input for creating a refresh token
#[derive(Debug, Clone)]
pub struct CreateRefreshTokenRow {
    pub user_id: Uuid,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}

/// OAuth client row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct OAuthClientRow {
    pub id: Uuid,
    pub client_id: String,
    pub client_secret_hash: String,
    pub client_name: String,
    pub redirect_uris: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an OAuth client
#[derive(Debug, Clone)]
pub struct CreateOAuthClientRow {
    pub client_id: String,
    pub client_secret_hash: String,
    pub client_name: String,
    pub redirect_uris: serde_json::Value,
}

/// OAuth authorization code row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct OAuthAuthorizationCodeRow {
    pub id: Uuid,
    pub code_hash: String,
    pub client_id: String,
    pub user_id: Uuid,
    pub org_id: i64,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub scope: String,
    pub consumed: bool,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an OAuth authorization code
#[derive(Debug, Clone)]
pub struct CreateOAuthAuthorizationCodeRow {
    pub code_hash: String,
    pub client_id: String,
    pub user_id: Uuid,
    pub org_id: i64,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
}

/// OAuth refresh token row from database
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct OAuthRefreshTokenRow {
    pub id: Uuid,
    pub token_hash: String,
    pub client_id: String,
    pub user_id: Uuid,
    pub org_id: i64,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

/// Input for creating an OAuth refresh token
#[derive(Debug, Clone)]
pub struct CreateOAuthRefreshTokenRow {
    pub token_hash: String,
    pub client_id: String,
    pub user_id: Uuid,
    pub org_id: i64,
    pub scope: String,
    pub expires_at: DateTime<Utc>,
}
