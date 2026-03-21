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

use crate::typed_id::{AgentId, AgentIdentityId, AppId, HarnessId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// App lifecycle status.
/// - `draft`: App is configured but not accepting requests
/// - `published`: App is live, accepting incoming requests
/// - `archived`: App is hidden from listings and cannot be modified or assigned
/// - `deleted`: App is a tombstone kept only for historical references
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
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
    /// Organization ID. Internal only, not exposed in API.
    #[serde(skip, default)]
    pub org_id: i64,
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
    /// Optional virtual identity that represents the app in unattended/channel execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "identity_01933b5a00007000800000000000001"))]
    pub agent_identity_id: Option<AgentIdentityId>,
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
    /// Timestamp when the app was archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when the app was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Session strategy for incoming messages (how messages map to sessions).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
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

/// How replies are delivered back to Slack.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SlackReplyMode {
    /// Forward completed assistant messages directly to Slack.
    #[default]
    AllMessages,
    /// Only send deterministic updates emitted via `report_progress`.
    ReportProgressOnly,
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
    /// How replies are delivered back to Slack.
    #[serde(default)]
    pub reply_mode: SlackReplyMode,
    /// Set when Slack successfully verifies the webhook URL (url_verification challenge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_verified_at: Option<DateTime<Utc>>,
    /// Set when the first real message is received from Slack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_message_received_at: Option<DateTime<Utc>>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_status_display() {
        assert_eq!(AppStatus::Draft.to_string(), "draft");
        assert_eq!(AppStatus::Published.to_string(), "published");
        assert_eq!(AppStatus::Archived.to_string(), "archived");
        assert_eq!(AppStatus::Deleted.to_string(), "deleted");
    }

    #[test]
    fn test_app_status_from_str() {
        assert_eq!(AppStatus::from("draft"), AppStatus::Draft);
        assert_eq!(AppStatus::from("published"), AppStatus::Published);
        assert_eq!(AppStatus::from("archived"), AppStatus::Archived);
        assert_eq!(AppStatus::from("deleted"), AppStatus::Deleted);
        assert_eq!(AppStatus::from("unknown"), AppStatus::Draft);
        assert_eq!(AppStatus::from(""), AppStatus::Draft);
    }

    #[test]
    fn test_app_status_serde_roundtrip() {
        let json = serde_json::to_string(&AppStatus::Published).unwrap();
        assert_eq!(json, r#""published""#);
        let parsed: AppStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, AppStatus::Published);
    }

    #[test]
    fn test_channel_type_display() {
        assert_eq!(ChannelType::Slack.to_string(), "slack");
    }

    #[test]
    fn test_channel_type_from_str_opt() {
        assert_eq!(ChannelType::from_str_opt("slack"), Some(ChannelType::Slack));
        assert_eq!(ChannelType::from_str_opt("unknown"), None);
        assert_eq!(ChannelType::from_str_opt(""), None);
    }

    #[test]
    fn test_channel_type_serde_roundtrip() {
        let json = serde_json::to_string(&ChannelType::Slack).unwrap();
        assert_eq!(json, r#""slack""#);
        let parsed: ChannelType = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, ChannelType::Slack);
    }

    #[test]
    fn test_session_strategy_default() {
        assert_eq!(SessionStrategy::default(), SessionStrategy::PerThread);
    }

    #[test]
    fn test_session_strategy_serde() {
        let json = serde_json::to_string(&SessionStrategy::PerChannel).unwrap();
        assert_eq!(json, r#""per_channel""#);
        let parsed: SessionStrategy = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SessionStrategy::PerChannel);

        let json = serde_json::to_string(&SessionStrategy::PerUser).unwrap();
        assert_eq!(json, r#""per_user""#);
    }

    #[test]
    fn test_slack_channel_config_full() {
        let json = r#"{
            "signing_secret": "sec123",
            "bot_token": "xoxb-tok",
            "channel_id": "C123",
            "team_id": "T123",
            "session_strategy": "per_channel",
            "reply_mode": "report_progress_only"
        }"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.signing_secret, "sec123");
        assert_eq!(config.bot_token, "xoxb-tok");
        assert_eq!(config.channel_id.as_deref(), Some("C123"));
        assert_eq!(config.team_id.as_deref(), Some("T123"));
        assert_eq!(config.session_strategy, SessionStrategy::PerChannel);
        assert_eq!(config.reply_mode, SlackReplyMode::ReportProgressOnly);
    }

    #[test]
    fn test_slack_channel_config_minimal() {
        let json = r#"{"signing_secret": "s", "bot_token": "t"}"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert!(config.channel_id.is_none());
        assert!(config.team_id.is_none());
        assert_eq!(config.session_strategy, SessionStrategy::PerThread);
        assert_eq!(config.reply_mode, SlackReplyMode::AllMessages);
        assert!(config.webhook_verified_at.is_none());
        assert!(config.first_message_received_at.is_none());
    }

    #[test]
    fn test_slack_channel_config_with_verification_timestamps() {
        let json = r#"{
            "signing_secret": "s",
            "bot_token": "t",
            "webhook_verified_at": "2025-01-01T00:00:00Z",
            "first_message_received_at": "2025-01-01T01:00:00Z"
        }"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert!(config.webhook_verified_at.is_some());
        assert!(config.first_message_received_at.is_some());

        // Round-trip: timestamps should be preserved
        let serialized = serde_json::to_value(&config).unwrap();
        assert!(serialized.get("webhook_verified_at").is_some());
        assert!(serialized.get("first_message_received_at").is_some());
    }

    #[test]
    fn test_slack_channel_config_timestamps_skipped_when_none() {
        let config = SlackChannelConfig {
            signing_secret: "s".into(),
            bot_token: "t".into(),
            channel_id: None,
            team_id: None,
            session_strategy: SessionStrategy::PerThread,
            reply_mode: SlackReplyMode::AllMessages,
            webhook_verified_at: None,
            first_message_received_at: None,
        };
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("webhook_verified_at").is_none());
        assert!(json.get("first_message_received_at").is_none());
    }

    #[test]
    fn test_slack_reply_mode_serde_roundtrip() {
        let json = serde_json::to_string(&SlackReplyMode::ReportProgressOnly).unwrap();
        assert_eq!(json, r#""report_progress_only""#);
        let parsed: SlackReplyMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SlackReplyMode::ReportProgressOnly);
    }

    #[test]
    fn test_slack_channel_config_missing_required_field() {
        let json = r#"{"signing_secret": "s"}"#;
        assert!(serde_json::from_str::<SlackChannelConfig>(json).is_err());
    }

    #[test]
    fn test_app_slack_config_valid() {
        let config_json = serde_json::json!({
            "signing_secret": "sec",
            "bot_token": "tok"
        });
        let app = App {
            public_id: AppId::from_uuid(Uuid::nil()),
            internal_id: Uuid::nil(),
            org_id: 1,
            name: "test".into(),
            description: None,
            harness_id: HarnessId::from_uuid(Uuid::nil()),
            agent_id: AgentId::from_uuid(Uuid::nil()),
            agent_identity_id: None,
            channel_type: ChannelType::Slack,
            channel_config: config_json,
            status: AppStatus::Draft,
            published_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        };
        let config = app.slack_config().unwrap();
        assert_eq!(config.signing_secret, "sec");
    }

    #[test]
    fn test_app_slack_config_invalid_json() {
        let app = App {
            public_id: AppId::from_uuid(Uuid::nil()),
            internal_id: Uuid::nil(),
            org_id: 1,
            name: "test".into(),
            description: None,
            harness_id: HarnessId::from_uuid(Uuid::nil()),
            agent_id: AgentId::from_uuid(Uuid::nil()),
            agent_identity_id: None,
            channel_type: ChannelType::Slack,
            channel_config: serde_json::json!({"bad": "data"}),
            status: AppStatus::Draft,
            published_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        };
        assert!(app.slack_config().is_none());
    }

    #[test]
    fn test_app_serde_skips_internal_fields() {
        let app = App {
            public_id: AppId::from_uuid(Uuid::nil()),
            internal_id: Uuid::nil(),
            org_id: 42,
            name: "test".into(),
            description: None,
            harness_id: HarnessId::from_uuid(Uuid::nil()),
            agent_id: AgentId::from_uuid(Uuid::nil()),
            agent_identity_id: None,
            channel_type: ChannelType::Slack,
            channel_config: serde_json::json!({}),
            status: AppStatus::Draft,
            published_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        };
        let json = serde_json::to_value(&app).unwrap();
        assert!(json.get("id").is_some()); // public_id serialized as "id"
        assert!(json.get("internal_id").is_none()); // skipped
        assert!(json.get("org_id").is_none()); // skipped
        assert!(json.get("published_at").is_none()); // None skipped
    }
}
