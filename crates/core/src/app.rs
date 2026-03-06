// App domain types
//
// Design Decision: Dual-ID pattern (see specs/id-schema.md)
// - public_id: AppId (external, API-facing, client-supplied or auto-generated)
// - internal_id: Uuid (internal PK, used for FK references, never exposed in API)
//
// An App binds a Harness + Agent to a distribution channel (Slack, etc.)
// with a publish/unpublish lifecycle.
//
// Design Decision: Slack ingestion is app-scoped (POST /v1/apps/{app_id}/slack/events)
// because the App defines the agent, harness, signing secret, and session strategy.
// Webhooks are unauthenticated — security comes from Slack signing secret verification.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::typed_id::{AgentId, AppId, HarnessId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// App lifecycle status.
/// - `draft`: App is configured but not accepting requests
/// - `published`: App is live, accepting incoming requests
/// - `archived`: App is soft-deleted and hidden from listings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum AppStatus {
    Draft,
    Published,
    Archived,
}

impl std::fmt::Display for AppStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppStatus::Draft => write!(f, "draft"),
            AppStatus::Published => write!(f, "published"),
            AppStatus::Archived => write!(f, "archived"),
        }
    }
}

impl From<&str> for AppStatus {
    fn from(s: &str) -> Self {
        match s {
            "published" => AppStatus::Published,
            "archived" => AppStatus::Archived,
            _ => AppStatus::Draft,
        }
    }
}

/// Supported channel types for app distribution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Slack,
}

impl std::fmt::Display for ChannelType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChannelType::Slack => write!(f, "slack"),
        }
    }
}

impl ChannelType {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "slack" => Some(ChannelType::Slack),
            _ => None,
        }
    }
}

/// App configuration for deploying agents to channels.
/// An app binds a harness and agent to a distribution channel with publish lifecycle.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct App {
    /// External identifier (app_<32-hex>). Shown as "id" in API.
    #[serde(rename = "id")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "app_01933b5a000070008000000000000001"))]
    pub public_id: AppId,
    /// Internal UUID primary key. Used for FK references. Never exposed in API.
    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,
    /// Display name of the app.
    pub name: String,
    /// Human-readable description of what the app does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// ID of the harness to use (format: harness_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "harness_01933b5a00007000800000000000001"))]
    pub harness_id: HarnessId,
    /// ID of the agent to use (format: agent_{32-hex}).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "agent_01933b5a00007000800000000000001"))]
    pub agent_id: AgentId,
    /// Distribution channel type.
    pub channel_type: ChannelType,
    /// Channel-specific configuration (validated per channel type).
    #[serde(default)]
    pub channel_config: serde_json::Value,
    /// Current lifecycle status.
    pub status: AppStatus,
    /// Timestamp when the app was last published.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<DateTime<Utc>>,
    /// Timestamp when the app was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the app was last updated.
    pub updated_at: DateTime<Utc>,
}

/// Session strategy for incoming messages (how messages map to sessions).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SessionStrategy {
    /// Each Slack thread gets its own session (default).
    #[default]
    PerThread,
    /// One session per channel.
    PerChannel,
    /// One session per user.
    PerUser,
}

/// Typed Slack channel configuration.
/// Parsed from the `channel_config` JSON field on App.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SlackChannelConfig {
    /// Slack signing secret for verifying webhook requests.
    pub signing_secret: String,
    /// Slack Bot OAuth token for sending responses.
    pub bot_token: String,
    /// Slack channel ID to listen on (e.g., "C0123456789").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Slack team/workspace ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// How incoming messages map to sessions.
    #[serde(default)]
    pub session_strategy: SessionStrategy,
}

impl App {
    /// Parse channel_config as SlackChannelConfig. Returns None if not a Slack app
    /// or if the config is invalid.
    pub fn slack_config(&self) -> Option<SlackChannelConfig> {
        if self.channel_type != ChannelType::Slack {
            return None;
        }
        serde_json::from_value(self.channel_config.clone()).ok()
    }
}
