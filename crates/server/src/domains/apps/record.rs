// Frozen App records retained for archival API responses and attribution.
use crate::domains::agent_channels::record::{AgentChannel, ChannelType};
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::{
    AgentChannelId, AgentId, AppId, HarnessId, PrincipalId, VirtualUserId,
};
use everruns_core::principal::PrincipalSummary;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// App lifecycle status.
/// - `draft`: App is configured but not accepting requests
/// - `published`: App is live, accepting incoming requests
/// - `archived`: App is hidden from listings and cannot be modified or assigned
/// - `deleted`: App is a tombstone kept only for historical references
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[schema(example = "published")]
#[serde(rename_all = "lowercase")]
pub enum AppStatus {
    Draft,
    Published,
    Archived,
    Deleted,
}

impl std::fmt::Display for AppStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppStatus::Draft => write!(f, "draft"),
            AppStatus::Published => write!(f, "published"),
            AppStatus::Archived => write!(f, "archived"),
            AppStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for AppStatus {
    fn from(s: &str) -> Self {
        match s {
            "published" => AppStatus::Published,
            "archived" => AppStatus::Archived,
            "deleted" => AppStatus::Deleted,
            _ => AppStatus::Draft,
        }
    }
}

/// App configuration for deploying agents to channels.
/// An app binds a harness and optional agent to distribution channels with a
/// publish lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct App {
    /// External identifier (app_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[schema(value_type = String, example = "app_01933b5a000070008000000000000001")]
    pub public_id: AppId,
    /// Internal UUID primary key. Used for FK references. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Organization ID. Internal only, not exposed in API.
    #[serde(skip, default)]
    pub org_id: i64,
    /// Display name of the app.
    pub name: String,
    /// Human-readable description of what the app does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// ID of the harness to use (format: harness_{32-hex}).
    #[schema(value_type = String, example = "harness_01933b5a00007000800000000000001")]
    pub harness_id: HarnessId,
    /// Optional ID of the agent to use (format: agent_{32-hex}).
    #[schema(value_type = Option<String>, example = "agent_01933b5a00007000800000000000001")]
    pub agent_id: Option<AgentId>,
    /// Optional virtual identity that represents the app in unattended/channel execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001")]
    pub virtual_user_id: Option<VirtualUserId>,
    /// Owning principal for this app.
    #[schema(value_type = String, example = "principal_01933b5a000070008000000000000001")]
    pub owner_principal_id: PrincipalId,
    /// Denormalized effective human owner of the owning principal lineage.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_owner_user_id: Option<Uuid>,
    /// Owning principal summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<PrincipalSummary>,
    /// Effective human owner summary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_owner: Option<PrincipalSummary>,
    /// Distribution channels attached to this app.
    #[serde(default)]
    pub channels: Vec<AgentChannel>,
    /// Current lifecycle status.
    pub status: AppStatus,
    /// Timestamp when the app was last published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    /// Timestamp when the app was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the app was last updated.
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the app was archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when the app was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

impl App {
    /// Find the first Slack channel on this app.
    pub fn slack_channel(&self) -> Option<&AgentChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::Slack && ch.enabled)
    }

    /// Find the first enabled AG-UI channel on this app.
    pub fn ag_ui_channel(&self) -> Option<&AgentChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::AgUi && ch.enabled)
    }

    /// Find the first enabled schedule channel on this app.
    pub fn schedule_channel(&self) -> Option<&AgentChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::Schedule && ch.enabled)
    }

    /// Find the first enabled webhook channel on this app.
    pub fn webhook_channel(&self) -> Option<&AgentChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::Webhook && ch.enabled)
    }

    /// Find the first enabled FCP channel on this app.
    pub fn fcp_channel(&self) -> Option<&AgentChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::Fcp && ch.enabled)
    }

    /// Find the first enabled A2A channel on this app.
    pub fn a2a_channel(&self) -> Option<&AgentChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::A2a && ch.enabled)
    }

    /// Find the first enabled api_endpoint channel on this app.
    pub fn api_endpoint_channel(&self) -> Option<&AgentChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::ApiEndpoint && ch.enabled)
    }

    /// Find the first enabled Public Chat channel on this app.
    pub fn public_chat_channel(&self) -> Option<&AgentChannel> {
        self.channels
            .iter()
            .find(|ch| ch.channel_type == ChannelType::PublicChat && ch.enabled)
    }

    /// Find a channel by its public ID.
    pub fn channel_by_id(&self, id: &AgentChannelId) -> Option<&AgentChannel> {
        self.channels.iter().find(|ch| ch.public_id == *id)
    }
}
