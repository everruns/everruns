// User rows.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// Canonical form of an email address used as a user identity (EVE-704).
///
/// Email is the account identity key across register / login / OAuth linking /
/// password recovery, so it must be treated case-insensitively: `John@x.com`
/// and `john@x.com` are the same mailbox and must resolve to one account. We
/// canonicalize by trimming surrounding whitespace and lowercasing, matching
/// the normalization the rate limiters and org-invitation matching already use
/// (`req.email.trim().to_lowercase()`). Applied at the storage trust boundary
/// (both backends' `create_user*` and `get_user_by_email`) so every caller —
/// register, login, forgot/resend, verify, oauth_callback linking, admin
/// bootstrap — shares one identity notion, backed by a case-insensitive unique
/// index on `users(lower(email))`.
pub fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

/// User row from database
#[derive(Debug, Clone, FromRow, serde::Serialize, everruns_server_macros::Columns)]
pub struct UserRow {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: sqlx::types::JsonValue,
    pub password_hash: Option<String>,
    pub email_verified: bool,
    pub auth_provider: Option<String>,
    pub auth_provider_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// External identity provider ID (e.g., PropelAuth user ID). NULL for OSS.
    #[sqlx(default)]
    pub external_id: Option<String>,
}

/// Input for creating a new user
#[derive(Debug, Clone)]
pub struct CreateUserRow {
    pub email: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub roles: Vec<String>,
    pub password_hash: Option<String>,
    pub email_verified: bool,
    pub auth_provider: Option<String>,
    pub auth_provider_id: Option<String>,
    /// External identity provider ID (e.g., PropelAuth user ID). NULL for OSS.
    pub external_id: Option<String>,
}

/// Input for updating a user
#[derive(Debug, Clone, Default)]
pub struct UpdateUser {
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub roles: Option<Vec<String>>,
    pub password_hash: Option<String>,
    pub email_verified: Option<bool>,
}
