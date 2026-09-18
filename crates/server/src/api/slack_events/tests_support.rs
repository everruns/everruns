//! Fixtures shared by the test modules.

use super::*;
use crate::storage::StorageBackend;
use everruns_core::channel::SessionBinding;
use everruns_platform::{ConversationStarter, SlackChannelConfig, SlackReplyMode};
use hmac::{KeyInit, Mac};
use std::ops::{Deref, DerefMut};

pub(crate) fn make_signature(secret: &str, timestamp: &str, body: &str) -> String {
    let sig_basestring = format!("v0:{}:{}", timestamp, body);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(sig_basestring.as_bytes());
    format!("v0={}", hex::encode(mac.finalize().into_bytes()))
}

pub(crate) struct TestIngress {
    context: crate::api::app_ingress::IngressContext,
    pub(crate) channels: Vec<crate::api::app_ingress::IngressEndpoint>,
}

impl Deref for TestIngress {
    type Target = crate::api::app_ingress::IngressContext;

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
    use everruns_platform::{ChannelType, EndpointStatus};
    use everruns_provider::typed_id::AppChannelId;

    TestIngress {
        context: crate::api::app_ingress::IngressContext::for_test("Test App", None),
        channels: vec![crate::api::app_ingress::IngressEndpoint {
            public_id: AppChannelId::from_uuid(uuid::Uuid::nil()),
            internal_id: uuid::Uuid::nil(),
            channel_type: ChannelType::Slack,
            channel_config: serde_json::json!({}),
            auth: None,
            enabled: true,
            status: EndpointStatus::Live,
        }],
    }
}

pub(crate) fn test_config(strategy: SessionBinding) -> SlackChannelConfig {
    SlackChannelConfig {
        agent_surface_enabled: false,
        signing_secret: "secret".to_string(),
        bot_token: "xoxb-token".to_string(),
        channel_id: None,
        team_id: None,
        session_strategy: strategy,
        reply_mode: SlackReplyMode::AllMessages,
        webhook_verified_at: None,
        first_message_received_at: None,
        tool_visibility: Default::default(),
        generic_tool_text: everruns_platform::app::DEFAULT_AG_UI_GENERIC_TOOL_TEXT.to_string(),
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

pub(crate) async fn setup_test_session(
    db: &StorageBackend,
) -> everruns_provider::typed_id::SessionId {
    use crate::storage::models::CreateSessionRow;

    let row = CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        workspace_id: None,
        org_id: 1,
        app_id: None,
        endpoint_id: None,
        harness_id: Some(everruns_provider::typed_id::HarnessId::from_uuid(
            uuid::Uuid::nil(),
        )),
        agent_id: Some(everruns_provider::typed_id::AgentId::from_uuid(
            uuid::Uuid::nil(),
        )),
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
        resolved_owner_user_id: None,
        title: Some("test".to_string()),
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::Value::Array(vec![]),
        hints: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        network_access: None,
        parent_session_id: None,
        budget_root_session_id: None,
    };
    let session = db.create_session(row).await.unwrap();
    session.id
}

// ==========================================
// Manifest helper tests
// ==========================================

pub(crate) const TEST_REQUEST_URL: &str =
    "https://example.com/api/v1/apps/app_test123/slack/events";

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
