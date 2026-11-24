use crate::{channelkit_bridge::ReceiptObserver, slack_channel::bridge::*};
use channelkit::{Adapter, ConversationKey, Provider};
use everruns_contracts::runtime::channel::{ChannelError, DeliveryReceiptSink, DeliveryResult};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[tokio::test]
async fn clients_reuse_connections_without_sharing_installation_or_rotated_credentials() {
    let server = MockServer::start().await;
    let factory =
        SlackAdapterFactory::new(Arc::new(channelkit::http::ReqwestTransport::new().unwrap()));
    let first = factory
        .get("app-a", "T1", Some("token-a"), None, &server.uri())
        .unwrap();
    let reused = factory
        .get("app-a", "T1", Some("token-a"), None, &server.uri())
        .unwrap();
    let sibling = factory
        .get("app-b", "T1", Some("token-b"), None, &server.uri())
        .unwrap();
    let rotated = factory
        .get("app-a", "T1", Some("token-new"), None, &server.uri())
        .unwrap();
    assert!(Arc::ptr_eq(&first, &reused));
    assert!(!Arc::ptr_eq(&first, &sibling));
    assert!(!Arc::ptr_eq(&first, &rotated));
    let target = ConversationKey::new(Provider::Slack, "app-a", "C1", None).unwrap();
    assert!(matches!(
        sibling
            .post(&target, channelkit::Content::Text("hello".into()))
            .await,
        Err(channelkit::Error::Scope)
    ));
    for token in ["token-a", "token-new"] {
        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .and(header("authorization", format!("Bearer {token}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"2.0"})))
            .expect(1)
            .mount(&server)
            .await;
    }
    first
        .post(&target, channelkit::Content::Text("hello".into()))
        .await
        .unwrap();
    rotated
        .post(&target, channelkit::Content::Text("hello".into()))
        .await
        .unwrap();
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

struct FailedCommit;
#[async_trait::async_trait]
impl DeliveryReceiptSink for FailedCommit {
    async fn confirmed(&self, key: &str) -> Result<(), ChannelError> {
        assert_eq!(key, "C1:2.0");
        Err(ChannelError::Session("injected lost lease".into()))
    }
}

#[tokio::test]
async fn failed_receipt_commit_stops_the_next_split_part_and_preserves_its_reference() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"2.0"})))
        .expect(1)
        .mount(&server)
        .await;
    let client = direct_factory()
        .unwrap()
        .get("app-a", "T1", Some("token-a"), None, &server.uri())
        .unwrap();
    let target = ConversationKey::new(Provider::Slack, "app-a", "C1", None).unwrap();
    let observer = ReceiptObserver {
        sink: &FailedCommit,
        target: &target,
        encode: message_key,
    };
    let error = client
        .post_markdown(
            &target,
            &"x".repeat(600_000),
            &Default::default(),
            Some(&observer),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(outcome(&error), DeliveryResult::Partial { confirmed, cause } if confirmed == ["C1:2.0"] && matches!(*cause, DeliveryResult::Unknown { .. }))
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn real_http_rate_advice_and_unknown_acceptance_are_not_flattened_or_retried() {
    for (status, expected_rate) in [(429, true), (503, false)] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat.postMessage"))
            .respond_with(
                ResponseTemplate::new(status)
                    .insert_header("retry-after", "7")
                    .set_body_json(json!({"ok":false,"error":"internal_error"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = direct_factory()
            .unwrap()
            .get("app-a", "T1", Some("token-a"), None, &server.uri())
            .unwrap();
        let error = client
            .api_call(
                "chat.postMessage",
                json!({"channel":"C1","text":"hello"}),
                true,
            )
            .await
            .unwrap_err();
        let result = outcome(&error);
        if expected_rate {
            assert_eq!(
                result,
                DeliveryResult::RateLimited {
                    retry_after: Duration::from_secs(7)
                }
            );
        } else {
            assert_eq!(result, DeliveryResult::Unknown { confirmed: vec![] });
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn token_identity_is_single_flight_and_workspace_changes_fail_before_output() {
    use everruns_contracts::runtime::channel::DeliveryTarget;
    let server = MockServer::start().await;
    for (token, team) in [("token-a", "T1"), ("token-b", "T2")] {
        Mock::given(method("POST"))
            .and(path("/auth.test"))
            .and(header("authorization", format!("Bearer {token}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_millis(10))
                    .set_body_json(json!({"ok":true,"team_id":team,"user_id":"U-bot"})),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let factory =
        SlackAdapterFactory::new(Arc::new(channelkit::http::ReqwestTransport::new().unwrap()));
    let mut context = DeliveryTarget::new("C1", "1.0").context("token-a");
    context.extra.insert(INSTALLATION_ID.into(), "app-a".into());
    context.extra.insert(TEAM_ID.into(), "T1".into());
    let api = server.uri();
    let (first, second) = tokio::join!(
        factory.for_context(&context, &api),
        factory.for_context(&context, &api)
    );
    assert!(Arc::ptr_eq(&first.unwrap(), &second.unwrap()));
    context.auth_token = "token-b".into();
    assert!(matches!(
        factory.for_context(&context, &api).await,
        Err(channelkit::Error::Scope)
    ));
    context.extra.remove(TEAM_ID);
    let actual = factory.for_context(&context, &api).await.unwrap();
    let payload = json!({"type":"events_api","envelope_id":"fixture","payload":{"type":"event_callback","team_id":"T2","event":{"type":"app_mention","channel":"C1","user":"U-other","ts":"1.0","text":"fixture"}}});
    assert!(
        actual
            .receive_socket_mode(&serde_json::to_vec(&payload).unwrap())
            .is_ok()
    );
    let mut foreign = payload;
    foreign["payload"]["team_id"] = json!("T1");
    assert!(matches!(
        actual.receive_socket_mode(&serde_json::to_vec(&foreign).unwrap()),
        Err(channelkit::Error::Scope)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

struct ScopeEgress {
    direct: channelkit::http::ReqwestTransport,
    seen: std::sync::Mutex<Vec<String>>,
    denied: bool,
}
#[async_trait::async_trait]
impl everruns_contracts::runtime::egress::EgressService for ScopeEgress {
    async fn send(
        &self,
        _: everruns_contracts::runtime::egress::EgressRequest,
    ) -> everruns_contracts::runtime::egress::EgressResult<
        everruns_contracts::runtime::egress::EgressResponse,
    > {
        unreachable!()
    }
    async fn send_stream(
        &self,
        request: everruns_contracts::runtime::egress::EgressRequest,
    ) -> everruns_contracts::runtime::egress::EgressResult<
        everruns_contracts::runtime::egress::EgressStreamResponse,
    > {
        use channelkit::http::Transport;
        use everruns_contracts::runtime::egress::*;
        assert!(request.dns_pinning_required);
        self.seen.lock().unwrap().push(request.url.clone());
        if self.denied {
            return Err(EgressError::NetworkAccessDenied { url: request.url });
        }
        let result = self
            .direct
            .execute(channelkit::http::Request {
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: request.body,
                max_response_bytes: 1024 * 1024,
            })
            .await
            .map_err(|_| EgressError::Transport("fixture request failed".into()))?;
        Ok(EgressStreamResponse {
            status: result.status,
            headers: result.headers,
            body: Box::pin(futures::stream::iter([Ok(result.body)])),
        })
    }
}
fn scope_egress(denied: bool) -> Arc<ScopeEgress> {
    Arc::new(ScopeEgress {
        direct: channelkit::http::ReqwestTransport::new().unwrap(),
        seen: std::sync::Mutex::new(vec![]),
        denied,
    })
}
struct ForbiddenDefault;
#[async_trait::async_trait]
impl channelkit::http::Transport for ForbiddenDefault {
    async fn execute(
        &self,
        _: channelkit::http::Request,
    ) -> channelkit::Result<channelkit::http::Response> {
        panic!("host-scoped requests must not use the driver's default transport")
    }
}
#[tokio::test]
async fn host_egress_is_used_for_identity_and_writes_and_is_part_of_cache_scope() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"team_id":"T1","user_id":"U-bot"})),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"2.0"})))
        .expect(2)
        .mount(&server)
        .await;
    let factory = SlackAdapterFactory::new(Arc::new(ForbiddenDefault));
    let first = scope_egress(false);
    let second = scope_egress(false);
    let denied = scope_egress(true);
    let mut context = everruns_contracts::runtime::channel::DeliveryTarget::new("C1", "1.0")
        .context("fixture-token");
    context.extra.insert(INSTALLATION_ID.into(), "app-a".into());
    context.extra.insert(TEAM_ID.into(), "T1".into());
    let a = factory
        .for_scoped_context(&context, &server.uri(), Some(first.clone()))
        .await
        .unwrap();
    let reused = factory
        .for_scoped_context(&context, &server.uri(), Some(first.clone()))
        .await
        .unwrap();
    let b = factory
        .for_scoped_context(&context, &server.uri(), Some(second.clone()))
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&a, &reused));
    assert!(!Arc::ptr_eq(&a, &b));
    assert!(
        factory
            .for_scoped_context(&context, &server.uri(), Some(denied.clone()))
            .await
            .is_err()
    );
    for client in [a, b] {
        client
            .post(
                &target(&client, &context).unwrap(),
                channelkit::Content::Text("fixture".into()),
            )
            .await
            .unwrap();
    }
    assert_eq!(first.seen.lock().unwrap().len(), 2);
    assert_eq!(second.seen.lock().unwrap().len(), 2);
    assert_eq!(denied.seen.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn cached_identity_keeps_its_scoped_service_alive_after_a_workspace_refusal() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"team_id":"T1","user_id":"U-bot"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let factory = SlackAdapterFactory::new(Arc::new(ForbiddenDefault));
    let scoped = scope_egress(false);
    let retained = Arc::downgrade(&scoped);
    assert!(matches!(
        factory
            .for_credentials(
                "app-a",
                Some("T-other"),
                "fixture-token",
                &server.uri(),
                Some(scoped.clone())
            )
            .await,
        Err(channelkit::Error::Scope)
    ));
    drop(scoped);
    assert!(
        retained.upgrade().is_some(),
        "a cached pointer must not outlive its scoped service"
    );
    drop(factory);
    assert!(retained.upgrade().is_none());
}
