// Rows the MCP OAuth grants repository reads and writes.

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// One user's approval of one MCP OAuth client (`oauth_grants`).
#[derive(Debug, Clone, FromRow, everruns_server_macros::Columns)]
pub struct OAuthGrantRow {
    pub id: Uuid,
    pub client_id: String,
    pub user_id: Uuid,
    /// `read_and_run` or `read_only`. Only `read_and_run` is written today.
    pub access: String,
    /// `None` means every organization the user belongs to.
    pub allowed_org_ids: Option<Vec<i64>>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

/// An active grant joined with the client it was given to, for the user's
/// "Connected AI clients" list.
#[derive(Debug, Clone, FromRow)]
pub struct OAuthGrantWithClientRow {
    pub id: Uuid,
    pub client_id: String,
    pub client_name: String,
    pub redirect_uris: serde_json::Value,
    pub access: String,
    pub allowed_org_ids: Option<Vec<i64>>,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}
