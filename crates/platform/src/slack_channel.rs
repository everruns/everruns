//! Typed configuration for a Slack channel.
//!
//! Split out of `app.rs` (EVE-1069): that file is on the size ratchet's debt
//! list, and one-click install adds to this type rather than to the rest of it.

use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

use chrono::{DateTime, Utc};

use crate::app::{
    SessionBinding, SlackReplyMode, default_ag_ui_generic_tool_text,
    is_default_ag_ui_generic_tool_text,
};
use crate::exposure::PublicToolVisibility;

/// Typed Slack channel configuration.
/// Parsed from the `channel_config` JSON field on App.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SlackChannelConfig {
    /// Slack signing secret for verifying webhook requests.
    ///
    /// May be empty while the channel is being configured. An empty value
    /// causes all incoming requests to fail signature verification.
    #[serde(default)]
    pub signing_secret: String,
    /// Slack Bot OAuth token for sending responses.
    ///
    /// May be empty while the channel is being configured.
    #[serde(default)]
    pub bot_token: String,
    /// Set when this deployment created the channel's Slack app (EVE-1069).
    /// Absent on a hand-configured channel.
    ///
    /// Never reaches an API response: `redact_channel_config` drops the whole
    /// object and surfaces `slack_app_provisioned: true` instead, so it carries
    /// no schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub provisioned_app: Option<crate::slack_provisioning::ProvisionedSlackApp>,
    /// Slack channel ID to listen on (e.g., "C0123456789").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    /// Slack team/workspace ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// What identity keys the session for incoming messages.
    #[serde(default)]
    pub session_strategy: SessionBinding,
    /// How replies are delivered back to Slack.
    #[serde(default)]
    pub reply_mode: SlackReplyMode,
    /// Set when Slack successfully verifies the webhook URL (url_verification challenge).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_verified_at: Option<DateTime<Utc>>,
    /// Set when the first real message is received from Slack.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_message_received_at: Option<DateTime<Utc>>,
    /// Whether this app also serves Slack's agent surface (the assistant pane).
    ///
    /// One boolean, not a mode: enabling Slack's Agents feature does not replace
    /// the channel bot, it adds an assistant container alongside it. The same app
    /// answers `@mentions` in channels *and* messages in the pane, and which
    /// surface an event belongs to is read from the event at runtime rather than
    /// from config (EVE-973).
    #[serde(default)]
    pub agent_surface_enabled: bool,
    /// Tool activity visibility for the agent pane's live status line.
    ///
    /// The pane is a user-facing surface like a published AG-UI endpoint, so it
    /// answers to the same policy rather than a second, divergent one (EVE-975).
    #[serde(default)]
    pub tool_visibility: PublicToolVisibility,
    /// Status text shown while a tool runs, when `tool_visibility` is `generic`.
    #[serde(
        default = "default_ag_ui_generic_tool_text",
        skip_serializing_if = "is_default_ag_ui_generic_tool_text"
    )]
    pub generic_tool_text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::DEFAULT_AG_UI_GENERIC_TOOL_TEXT;
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
        assert_eq!(config.session_strategy, SessionBinding::Conversation);
        assert_eq!(config.reply_mode, SlackReplyMode::ReportProgressOnly);
    }

    #[test]
    fn test_slack_channel_config_minimal() {
        let json = r#"{"signing_secret": "s", "bot_token": "t"}"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert!(config.channel_id.is_none());
        assert!(config.team_id.is_none());
        assert_eq!(config.session_strategy, SessionBinding::Thread);
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

    /// Channels stored before the agent surface existed have no such key, and
    /// must keep parsing with it off (EVE-973).
    #[test]
    fn test_slack_channel_config_defaults_agent_surface_off() {
        let json = r#"{"signing_secret":"s","bot_token":"t"}"#;
        let config: SlackChannelConfig = serde_json::from_str(json).unwrap();
        assert!(!config.agent_surface_enabled);
    }

    #[test]
    fn test_slack_channel_config_timestamps_skipped_when_none() {
        let config = SlackChannelConfig {
            signing_secret: "s".into(),
            bot_token: "t".into(),
            provisioned_app: None,
            channel_id: None,
            team_id: None,
            session_strategy: SessionBinding::Thread,
            reply_mode: SlackReplyMode::AllMessages,
            webhook_verified_at: None,
            first_message_received_at: None,
            agent_surface_enabled: false,
            tool_visibility: PublicToolVisibility::default(),
            generic_tool_text: DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string(),
        };
        let json = serde_json::to_value(&config).unwrap();
        assert!(json.get("webhook_verified_at").is_none());
        // The default text is a knob nobody turned; serialising it into every
        // stored Slack config would be noise, same as the AG-UI config.
        assert!(json.get("generic_tool_text").is_none());
        assert!(json.get("first_message_received_at").is_none());
        // Absent on a hand-configured channel (EVE-1069).
        assert!(json.get("provisioned_app").is_none());
    }

    #[test]
    fn test_slack_reply_mode_serde_roundtrip() {
        let json = serde_json::to_string(&SlackReplyMode::ReportProgressOnly).unwrap();
        assert_eq!(json, r#""report_progress_only""#);
        let parsed: SlackReplyMode = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, SlackReplyMode::ReportProgressOnly);
    }

    #[test]
    fn test_slack_channel_config_defaults_omitted_credentials() {
        let config: SlackChannelConfig = serde_json::from_str("{}").unwrap();
        assert!(config.signing_secret.is_empty());
        assert!(config.bot_token.is_empty());
    }
}
