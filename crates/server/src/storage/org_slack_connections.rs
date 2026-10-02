use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

/// One Slack workspace an organization has connected for app provisioning.
///
/// An organization may connect several. `team_id` identifies the workspace and
/// is `None` only for rows created before workspaces were recorded; the
/// rotation sweep fills it from Slack's rotate response.
#[derive(Clone, Debug, FromRow)]
pub struct OrgSlackConnectionRow {
    pub id: Uuid,
    pub org_id: i64,
    pub team_id: Option<String>,
    pub team_name: Option<String>,
    pub access_token_encrypted: Option<Vec<u8>>,
    pub refresh_token_encrypted: Option<Vec<u8>>,
    pub access_token_expires_at: Option<DateTime<Utc>>,
    pub state: String,
    pub token_generation: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Connect a workspace, or replace the tokens of one already connected.
#[derive(Clone, Debug)]
pub struct UpsertOrgSlackConnection {
    pub org_id: i64,
    pub team_id: String,
    pub team_name: Option<String>,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Vec<u8>,
    pub access_token_expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct RotateOrgSlackConnection {
    pub id: Uuid,
    pub expected_generation: i64,
    /// Fills `team_id` when the row predates workspace identity. Never
    /// overwrites one already recorded.
    pub team_id: Option<String>,
    pub access_token_encrypted: Vec<u8>,
    pub refresh_token_encrypted: Vec<u8>,
    pub access_token_expires_at: DateTime<Utc>,
}
