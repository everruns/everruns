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
    /// The approval this token belongs to (`oauth_grants`). `None` only for a
    /// token minted before grants existed or by a pre-grant replica.
    pub grant_id: Option<Uuid>,
    /// When this token was exchanged for its successor. A rotated token is kept
    /// so a replay can be told apart from an unknown token.
    pub rotated_at: Option<DateTime<Utc>>,
    /// Whether the one replay allowed inside the reuse grace window was used.
    pub grace_used: bool,
    /// The rotation chain this token belongs to. `None` for a chain's first
    /// token, whose own `id` then names the family.
    pub family_id: Option<Uuid>,
}

impl OAuthRefreshTokenRow {
    /// The rotation chain this token belongs to; its successor carries it.
    pub fn family(&self) -> Uuid {
        self.family_id.unwrap_or(self.id)
    }
}

/// What presenting an MCP OAuth refresh token resolved to.
#[derive(Debug, Clone)]
pub enum OAuthRefreshUse {
    /// Issue the successor: a first use, or the one replay inside the grace
    /// window.
    Mint(OAuthRefreshTokenRow),
    /// Unknown, expired, or a further replay inside the window. Nothing revoked.
    Refused,
    /// A rotated token replayed after the window: its whole family was deleted.
    ReuseDetected,
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
    pub grant_id: Uuid,
    /// The rotation chain: `None` starts a new one.
    pub family_id: Option<Uuid>,
}
