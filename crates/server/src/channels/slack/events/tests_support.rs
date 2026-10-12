//! Fixtures shared by the test modules.

use super::*;
use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use crate::domains::harnesses::record::ConversationStarter;
use everruns_core::channel::SessionBinding;
use hmac::{KeyInit, Mac};
use std::ops::{Deref, DerefMut};

pub(crate) fn make_signature(secret: &str, timestamp: &str, body: &str) -> String {
    let sig_basestring = format!("v0:{}:{}", timestamp, body);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(sig_basestring.as_bytes());
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
}

pub(crate) struct TestIngress {
    context: crate::api::channel_ingress::IngressContext,
    pub(crate) channels: Vec<crate::api::channel_ingress::IngressChannel>,
}

impl Deref for TestIngress {
    type Target = crate::api::channel_ingress::IngressContext;

    fn deref(&self) -> &Self::Target {
        &self.context
    }
}

impl DerefMut for TestIngress {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.context
    }
}

pub(crate) fn test_app() -> TestIngress {
    use crate::domains::agent_channels::record::{ChannelStatus, ChannelType};
    use crate::records::AgentChannelId;

    TestIngress {
        context: crate::api::channel_ingress::IngressContext::for_test("Test App", None),
        channels: vec![crate::api::channel_ingress::IngressChannel {
            public_id: AgentChannelId::from_uuid(uuid::Uuid::nil()),
            internal_id: uuid::Uuid::nil(),
            channel_type: ChannelType::Slack,
            channel_config: serde_json::json!({}),
            auth: None,
            enabled: true,
            status: ChannelStatus::Live,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }],
    }
}

pub(crate) fn test_config(strategy: SessionBinding) -> SlackChannelConfig {
    SlackChannelConfig {
        agent_surface_enabled: false,
        signing_secret: "secret".to_string(),
        bot_token: "xoxb-token".to_string(),
        provisioned_app: None,
        channel_id: None,
        team_id: None,
        session_strategy: strategy,
        response_policy: Default::default(),
        webhook_verified_at: None,
        first_message_received_at: None,
        tool_visibility: Default::default(),
        generic_tool_text: crate::domains::agent_channels::record::DEFAULT_AG_UI_GENERIC_TOOL_TEXT
            .to_string(),
    }
}

pub(crate) fn test_event(channel: &str, ts: Option<&str>, thread_ts: Option<&str>) -> SlackEvent {
    SlackEvent {
        event_type: "message".to_string(),
        user: None,
        text: Some("Hello".to_string()),
        title: None,
        channel: Some(channel.to_string()),
        thread_ts: thread_ts.map(String::from),
        ts: ts.map(String::from),
        bot_id: None,
        subtype: None,
        channel_type: None,
        files: vec![],
        attachments: vec![],
        assistant_thread: None,
    }
}

pub(crate) fn test_slack_file(
    name: &str,
    mimetype: &str,
    filetype: &str,
    url: Option<&str>,
) -> SlackFile {
    SlackFile {
        id: Some("F0123456789".to_string()),
        name: Some(name.to_string()),
        mimetype: Some(mimetype.to_string()),
        filetype: Some(filetype.to_string()),
        url_private: url.map(String::from),
        size: Some(1024),
    }
}

// ==========================================
// Manifest helper tests
// ==========================================

pub(crate) const TEST_INTERACTIVITY_URL: &str =
    "https://api.example.com/v1/channels/appchan_test/slack/interactivity";
pub(crate) const TEST_REQUEST_URL: &str =
    "https://example.com/api/v1/apps/app_test123/slack/events";
pub(crate) const TEST_REDIRECT_URL: &str =
    "https://api.example.com/v1/channels/appchan_test/slack/oauth/callback";

pub(crate) fn test_starter(text: &str) -> ConversationStarter {
    ConversationStarter {
        icon: None,
        text: text.to_string(),
    }
}

pub(crate) fn test_slack_attachment() -> SlackAttachment {
    SlackAttachment {
        title: None,
        title_link: None,
        text: None,
        fallback: None,
        pretext: None,
        image_url: None,
        thumb_url: None,
        author_name: None,
        service_name: None,
        original_url: None,
        app_unfurl_url: None,
    }
}

/// Noop runner for tests that need SlackState without a real worker.
pub(crate) struct NoopRunner;
