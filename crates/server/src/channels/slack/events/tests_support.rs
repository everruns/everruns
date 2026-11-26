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
        private_approvals: false,
        signing_secret: "secret".to_string(),
        bot_token: "xoxb-token".to_string(),
        connection: Default::default(),
        app_token: String::new(),
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
        reaction: None,
        item: None,
    }
}

pub(crate) fn test_slack_file(
    name: &str,
    mimetype: &str,
    filetype: &str,
    _url: Option<&str>,
) -> SlackFile {
    SlackFile {
        id: Some("F0123456789".to_string()),
        name: Some(name.to_string()),
        mimetype: Some(mimetype.to_string()),
        filetype: Some(filetype.to_string()),
        is_external: false,
        permalink: None,
        size: Some(1024),
    }
}

// ==========================================
// Manifest helper tests
// ==========================================

pub(crate) const TEST_INTERACTIVITY_URL: &str =
    "https://api.example.com/v1/channels/appchan_test/slack/interactivity";
pub(crate) const TEST_REQUEST_URL: &str =
    "https://example.com/api/v1/channels/appchan_test123/slack/events";
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

/// A provider fixture with real installation discovery, isolated to loopback.
pub(crate) async fn mock_provider() -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::path("/auth.test"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": true, "team_id": "T1", "user_id": "UBOT"})),
        )
        .mount(&server)
        .await;
    server
}

pub(crate) async fn provider_requests(server: &wiremock::MockServer) -> Vec<wiremock::Request> {
    server
        .received_requests()
        .await
        .expect("provider requests")
        .into_iter()
        .filter(|request| request.url.path() != "/auth.test")
        .collect()
}

/// The library still validates Slack's HTTPS upload URL. Only this test-owned
/// host transport maps the approved fixture hop into its local HTTP server.
pub(crate) fn upload_egress(base: &str) -> Arc<dyn everruns_core::EgressService> {
    struct LoopbackSlackEgress(String);
    #[async_trait::async_trait]
    impl everruns_core::EgressService for LoopbackSlackEgress {
        async fn send(
            &self,
            _: everruns_core::egress::EgressRequest,
        ) -> everruns_core::egress::EgressResult<everruns_core::egress::EgressResponse> {
            unreachable!()
        }
        async fn send_stream(
            &self,
            request: everruns_core::egress::EgressRequest,
        ) -> everruns_core::egress::EgressResult<everruns_core::egress::EgressStreamResponse>
        {
            assert!(request.dns_pinning_required);
            let url = if request.url == "https://files.slack.com/upload-here" {
                assert!(!request.headers.contains_key("authorization"));
                format!("{}/upload-here", self.0)
            } else {
                assert!(request.url.starts_with(&format!("{}/", self.0)));
                request.url
            };
            let client = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let mut outbound = client
                .request(
                    reqwest::Method::from_bytes(request.method.as_bytes()).unwrap(),
                    url,
                )
                .body(request.body);
            for (name, value) in request.headers {
                outbound = outbound.header(name, value);
            }
            let response = outbound.send().await.unwrap();
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_str().unwrap().to_string()))
                .collect();
            let body = response.bytes().await.unwrap().to_vec();
            assert!(body.len() <= 1024 * 1024);
            Ok(everruns_core::egress::EgressStreamResponse {
                status,
                headers,
                body: Box::pin(futures::stream::iter([Ok(body)])),
            })
        }
    }
    let url = url::Url::parse(base).unwrap();
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    Arc::new(LoopbackSlackEgress(base.into()))
}
