use super::*;
use everruns_server_macros::Columns;

#[derive(Debug, Clone, FromRow)]
pub struct VirtualUserRow {
    pub usage: String,
    pub id: VirtualUserId,
    pub org_id: i64,
    pub name: String,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub archived_at: Option<DateTime<Utc>>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct CreateVirtualUserRow {
    pub usage: String,
    pub org_id: i64,
    pub id: VirtualUserId,
    pub name: String,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub locale: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateVirtualUser {
    pub name: Option<String>,
    pub description: UpdateField<String>,
    pub avatar_url: UpdateField<String>,
    pub locale: UpdateField<String>,
    pub timezone: UpdateField<String>,
    pub status: Option<String>,
}

impl From<VirtualUserConnectionRow> for UserConnectionRow {
    fn from(r: VirtualUserConnectionRow) -> Self {
        Self {
            id: r.id,
            user_id: r.virtual_user_id.uuid(),
            provider: r.provider,
            connection_type: r.connection_type,
            provider_user_id: r.provider_user_id,
            provider_username: r.provider_username,
            access_token_encrypted: r.access_token_encrypted,
            refresh_token_encrypted: r.refresh_token_encrypted,
            scopes: r.scopes,
            expires_at: r.expires_at,
            installation_id: r.installation_id,
            provider_metadata: r.provider_metadata,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

#[derive(Debug, Clone, FromRow, serde::Serialize, Columns)]
pub struct VirtualUserPreferenceRow {
    pub id: Uuid,
    pub virtual_user_id: VirtualUserId,
    pub key: String,
    pub value: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
