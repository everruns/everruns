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
    /// `config_token` or `manager_app`; see [`SlackGrantKind`].
    pub grant_kind: String,
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
    pub grant_kind: SlackGrantKind,
}

/// Where a connection's tokens came from. It decides how they are refreshed,
/// and nothing else: both kinds create apps through the same manifest API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlackGrantKind {
    /// An app configuration token an admin generated and pasted. Rotates
    /// through `tooling.tokens.rotate`.
    ConfigToken,
    /// A user token from the deployment's Slack manager app OAuth consent.
    /// Refreshes through `oauth.v2.access` with the manager app's client pair.
    ManagerApp,
}

impl SlackGrantKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfigToken => "config_token",
            Self::ManagerApp => "manager_app",
        }
    }
}

impl OrgSlackConnectionRow {
    pub fn grant_kind(&self) -> SlackGrantKind {
        if self.grant_kind == SlackGrantKind::ManagerApp.as_str() {
            SlackGrantKind::ManagerApp
        } else {
            SlackGrantKind::ConfigToken
        }
    }
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
