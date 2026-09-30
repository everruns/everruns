// Rows for per-agent GitHub Apps (`crate::github_apps`).

use chrono::{DateTime, Utc};
use everruns_provider::typed_id::AgentIdentityId;
use sqlx::FromRow;
use uuid::Uuid;

/// A GitHub App created for one agent identity through the manifest flow.
/// Secrets are stored encrypted; see migration 147.
#[derive(Debug, Clone, FromRow)]
pub struct GitHubAppRow {
    pub id: Uuid,
    pub org_id: i64,
    pub agent_identity_id: AgentIdentityId,
    pub app_id: i64,
    pub slug: String,
    pub name: String,
    pub html_url: String,
    pub owner_login: Option<String>,
    pub client_id: Option<String>,
    pub client_secret_encrypted: Option<Vec<u8>>,
    pub private_key_encrypted: Vec<u8>,
    pub webhook_secret_encrypted: Option<Vec<u8>>,
    pub created_by_user_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct CreateGitHubAppRow {
    pub id: Uuid,
    pub org_id: i64,
    pub agent_identity_id: AgentIdentityId,
    pub app_id: i64,
    pub slug: String,
    pub name: String,
    pub html_url: String,
    pub owner_login: Option<String>,
    pub client_id: Option<String>,
    pub client_secret_encrypted: Option<Vec<u8>>,
    pub private_key_encrypted: Vec<u8>,
    pub webhook_secret_encrypted: Option<Vec<u8>>,
    pub created_by_user_id: Option<Uuid>,
}
