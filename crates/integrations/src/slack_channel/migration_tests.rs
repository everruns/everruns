use super::*;
use everruns_contracts::runtime::channel::{
    ApprovalPrompt, ChannelApprovalPrompt, ChannelAttachmentResolver, ChannelProgressSurface,
    ChannelStreamDelivery,
};
use std::{collections::BTreeMap, sync::Mutex, time::Duration};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn sdk_stream_and_progress_open_preserve_typed_rate_limits() {
    let server = MockServer::start().await;
    super::tests::bot_identity(&server).await;
    for (endpoint, delay) in [("chat.startStream", 7), ("chat.postMessage", 11)] {
        Mock::given(method("POST"))
            .and(path(format!("/{endpoint}")))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after", delay.to_string())
                    .set_body_json(json!({"ok":false,"error":"ratelimited"})),
            )
            .expect(1)
            .mount(&server)
            .await;
    }
    let slack = Slack::new("fixture-token", "fixture-secret")
        .api_base(server.uri())
        .agent_pane()
        .task_progress();
    let context = slack.delivery_context(&DeliveryTarget::new("D1", "1.0"));
    assert_eq!(
        slack.start_checked(&context).await,
        Err(DeliveryResult::RateLimited {
            retry_after: Duration::from_secs(7)
        })
    );
    assert_eq!(
        slack.post_checked("Working", &context).await,
        Err(DeliveryResult::RateLimited {
            retry_after: Duration::from_secs(11)
        })
    );
}

#[tokio::test]
async fn sdk_signing_secret_and_original_installation_are_mandatory() {
    let body = super::tests::mention("fixture");
    let mut unsigned = ChannelRequest::json(&body);
    unsigned.path = "slack-socket-mode".into();
    assert_eq!(
        Slack::new("fixture-token", "")
            .receive(&unsigned)
            .await
            .unwrap_err()
            .status(),
        401
    );
    let slack = Slack::new("fixture-token", "fixture-secret").installation("app-a");
    let mut target = DeliveryTarget::new("C1", "1.0");
    target
        .extra
        .insert(bridge::INSTALLATION_ID.into(), "app-b".into());
    let context = slack.delivery_context(&target);
    assert_eq!(context.extra[bridge::INSTALLATION_ID], "app-b");
    let result = slack
        .deliver(
            &OutboundChannelMessage {
                session_id: everruns_contracts::typed_id::SessionId::new(),
                text: "fixture".into(),
                thread_ref: "1.0".into(),
                correlation_id: None,
            },
            &context,
        )
        .await;
    assert!(matches!(result, DeliveryResult::PermanentError(_)));
    let request = super::tests::signed(&body, "fixture-secret", unix_now());
    assert_eq!(
        slack
            .workspace("T-other")
            .receive(&request)
            .await
            .unwrap_err()
            .status(),
        401
    );
}

struct FileTransport {
    seen: Mutex<Vec<String>>,
    returned_url: &'static str,
}
#[async_trait]
impl channelkit::http::Transport for FileTransport {
    async fn execute(
        &self,
        request: channelkit::http::Request,
    ) -> channelkit::Result<channelkit::http::Response> {
        self.seen.lock().unwrap().push(request.url.clone());
        assert_eq!(
            request.headers.get("authorization").map(String::as_str),
            Some("Bearer fixture-token")
        );
        let body = if request.url.ends_with("/auth.test") {
            json!({"ok":true,"team_id":"T1","user_id":"U-bot"})
                .to_string()
                .into_bytes()
        } else if request.url.contains("/files.info?") {
            json!({"ok":true,"file":{"id":"F1","size":12,"url_private_download":self.returned_url}})
                .to_string()
                .into_bytes()
        } else {
            assert_eq!(request.url, "https://files.slack.com/fixture.pdf");
            b"%PDF-1.7 body".to_vec()
        };
        Ok(channelkit::http::Response {
            status: 200,
            headers: BTreeMap::new(),
            body,
        })
    }
}
struct FileEgress(Arc<FileTransport>);
#[async_trait]
impl everruns_contracts::runtime::egress::EgressService for FileEgress {
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
        assert!(request.dns_pinning_required);
        let result = self
            .0
            .execute(channelkit::http::Request {
                method: request.method,
                url: request.url,
                headers: request.headers,
                body: request.body,
                max_response_bytes: 1024 * 1024,
            })
            .await
            .map_err(|_| {
                everruns_contracts::runtime::egress::EgressError::Transport(
                    "fixture failure".into(),
                )
            })?;
        Ok(everruns_contracts::runtime::egress::EgressStreamResponse {
            status: result.status,
            headers: result.headers,
            body: Box::pin(futures::stream::iter([Ok(result.body)])),
        })
    }
}
struct ForbiddenFileDefault;
#[async_trait]
impl channelkit::http::Transport for ForbiddenFileDefault {
    async fn execute(
        &self,
        _: channelkit::http::Request,
    ) -> channelkit::Result<channelkit::http::Response> {
        panic!("provider files must use the host intake egress")
    }
}

#[tokio::test]
async fn provider_file_resolution_uses_the_library_and_rejects_foreign_scope_and_hosts() {
    for (url, success) in [
        ("https://files.slack.com/fixture.pdf", true),
        ("https://attacker.invalid/fixture.pdf", false),
    ] {
        let transport = Arc::new(FileTransport {
            seen: Mutex::new(vec![]),
            returned_url: url,
        });
        let slack = Slack::new("fixture-token", "fixture-secret")
            .installation("app-a")
            .workspace("T1")
            .transport(Arc::new(ForbiddenFileDefault));
        for reference in [r#"["app-b","T1","F1"]"#, r#"["app-a","T-other","F1"]"#] {
            assert!(
                ChannelAttachmentResolver::download(
                    &slack,
                    reference,
                    1024,
                    Arc::new(FileEgress(transport.clone()))
                )
                .await
                .is_err()
            );
        }
        assert!(transport.seen.lock().unwrap().is_empty());
        let result = ChannelAttachmentResolver::download(
            &slack,
            r#"["app-a","T1","F1"]"#,
            1024,
            Arc::new(FileEgress(transport.clone())),
        )
        .await;
        assert_eq!(result.is_ok(), success);
        assert_eq!(
            transport.seen.lock().unwrap().len(),
            if success { 3 } else { 2 }
        );
        if success {
            assert_eq!(result.unwrap(), b"%PDF-1.7 body");
        }
    }
}

fn signed_action(payload: &Value) -> ChannelRequest {
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;
    let body = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("payload", &payload.to_string())
        .finish()
        .into_bytes();
    let now = unix_now();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"fixture-secret").unwrap();
    mac.update(format!("v0:{now}:").as_bytes());
    mac.update(&body);
    ChannelRequest {
        path: String::new(),
        headers: vec![],
        body,
    }
    .header("content-type", "application/x-www-form-urlencoded")
    .header("x-slack-request-timestamp", now.to_string())
    .header(
        "x-slack-signature",
        format!("v0={}", hex::encode(mac.finalize().into_bytes())),
    )
}
#[tokio::test]
async fn native_sdk_card_values_remain_requester_thread_and_one_use_bound() {
    let server = MockServer::start().await;
    super::tests::bot_identity(&server).await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"2.0"})))
        .expect(1)
        .mount(&server)
        .await;
    let slack = Slack::new("fixture-token", "fixture-secret")
        .api_base(server.uri())
        .workspace("T1")
        .approval_buttons();
    let target = advanced::target("C1", "1.0", "U7", "T1");
    let context = slack.delivery_context(&target);
    assert_eq!(
        slack
            .prompt(
                everruns_contracts::typed_id::SessionId::new(),
                &ApprovalPrompt {
                    action: "fixture action".into(),
                    question: None,
                    turn_id: Some("turn-1".into())
                },
                &context
            )
            .await,
        DeliveryResult::Ok
    );
    let posts = server.received_requests().await.unwrap();
    let post: Value = serde_json::from_slice(
        &posts
            .iter()
            .find(|request| request.url.path() == "/chat.postMessage")
            .unwrap()
            .body,
    )
    .unwrap();
    let value = post["blocks"][1]["elements"][0]["value"].as_str().unwrap();
    let payload = json!({"type":"block_actions","team":{"id":"T1"},"user":{"id":"U7"},"channel":{"id":"C1"},"message":{"ts":"2.0","thread_ts":"1.0"},"actions":[{"action_id":"everruns_approve","value":value,"action_ts":"3.0"}]});
    for (section, field, replacement) in [
        ("user", "id", "U-other"),
        ("channel", "id", "C-other"),
        ("message", "thread_ts", "9.0"),
    ] {
        let mut wrong = payload.clone();
        wrong[section][field] = json!(replacement);
        assert_eq!(
            slack
                .receive(&signed_action(&wrong))
                .await
                .unwrap_err()
                .status(),
            401
        );
    }
    let Inbound::Message(message) = slack.receive(&signed_action(&payload)).await.unwrap() else {
        panic!("approval should become a user message")
    };
    assert_eq!(message.event.actor.actor_id, "U7");
    assert!(message.event.text.starts_with("Approved"));
    assert!(matches!(
        slack.receive(&signed_action(&payload)).await.unwrap(),
        Inbound::Ignore
    ));
}

#[tokio::test]
async fn token_only_installation_scope_is_verified_before_accepting_input() {
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
    let slack = Slack::new("fixture-token", "fixture-secret")
        .installation("token-only-scope-regression")
        .api_base(server.uri());
    let body = super::tests::mention("fixture");
    assert!(matches!(
        slack
            .receive(&super::tests::signed(&body, "fixture-secret", unix_now()))
            .await
            .unwrap(),
        Inbound::Message(_)
    ));
    let mut foreign = body;
    foreign["team_id"] = json!("T-other");
    assert_eq!(
        slack
            .receive(&super::tests::signed(
                &foreign,
                "fixture-secret",
                unix_now()
            ))
            .await
            .unwrap_err()
            .status(),
        401
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

struct LostCardReceipt;
#[async_trait]
impl everruns_contracts::runtime::channel::DeliveryReceiptSink for LostCardReceipt {
    async fn confirmed(&self, key: &str) -> Result<(), ChannelError> {
        assert_eq!(key, "C1:2.0");
        Err(ChannelError::Session("sensitive-store-diagnostic".into()))
    }
}
#[tokio::test]
async fn accepted_sdk_card_keeps_its_binding_after_a_receipt_commit_failure() {
    let server = MockServer::start().await;
    super::tests::bot_identity(&server).await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"2.0"})))
        .expect(1)
        .mount(&server)
        .await;
    let slack = Slack::new("fixture-token", "fixture-secret")
        .api_base(server.uri())
        .workspace("T1")
        .approval_buttons();
    let mut context = slack.delivery_context(&advanced::target("C1", "1.0", "U7", "T1"));
    context.receipts = Some(Arc::new(LostCardReceipt));
    let result = slack
        .prompt(
            everruns_contracts::typed_id::SessionId::new(),
            &ApprovalPrompt {
                action: "fixture action".into(),
                question: None,
                turn_id: None,
            },
            &context,
        )
        .await;
    assert!(!result.to_string().contains("sensitive-store-diagnostic"));
    assert!(
        matches!(result,DeliveryResult::Partial {confirmed,cause} if confirmed==["C1:2.0"] && cause.is_uncertain())
    );
    let requests = server.received_requests().await.unwrap();
    let post: Value = serde_json::from_slice(
        &requests
            .iter()
            .find(|request| request.url.path() == "/chat.postMessage")
            .unwrap()
            .body,
    )
    .unwrap();
    let value = post["blocks"][1]["elements"][0]["value"].as_str().unwrap();
    let action = json!({"type":"block_actions","team":{"id":"T1"},"user":{"id":"U7"},"channel":{"id":"C1"},"message":{"ts":"2.0","thread_ts":"1.0"},"actions":[{"action_id":"everruns_approve","value":value,"action_ts":"3.0"}]});
    assert!(matches!(
        slack.receive(&signed_action(&action)).await.unwrap(),
        Inbound::Message(_)
    ));
    assert!(matches!(
        slack.receive(&signed_action(&action)).await.unwrap(),
        Inbound::Ignore
    ));
}

#[tokio::test]
async fn shared_private_card_neutralizes_notifications_and_never_falls_back_publicly() {
    use wiremock::matchers::body_partial_json;
    let server = MockServer::start().await;
    super::tests::bot_identity(&server).await;
    Mock::given(path("/chat.postEphemeral"))
        .and(body_partial_json(
            json!({"channel":"C1","thread_ts":"1.0","user":"U1","text":"Ask @here @S1"}),
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":false,"error":"missing_scope"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let slack = Slack::new("fixture-token", "fixture-secret")
        .installation("private-card-boundary")
        .api_base(server.uri());
    let context = slack.delivery_context(&DeliveryTarget::new("C1", "1.0"));
    let surface = channelkit_surfaces::ChannelkitSurfaces {
        api_base: &slack.api_base,
        factory: slack.factory().unwrap(),
    };
    let result = surface
        .post_card(
            &channelkit_slack::SlackCard {
                title: None,
                text: "Ask <!here> <!subteam^S1>".into(),
                actions: vec![],
            },
            Some("U1"),
            &context,
        )
        .await;
    assert!(matches!(result, DeliveryResult::PermanentError(_)));
}

#[tokio::test]
async fn shared_card_stops_at_a_failed_receipt_checkpoint() {
    let server = MockServer::start().await;
    super::tests::bot_identity(&server).await;
    Mock::given(path("/chat.postMessage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"2.0"})))
        .expect(1)
        .mount(&server)
        .await;
    let slack = Slack::new("fixture-token", "fixture-secret")
        .installation("server-card-receipt")
        .api_base(server.uri());
    let mut context = slack.delivery_context(&DeliveryTarget::new("C1", "1.0"));
    context.receipts = Some(Arc::new(LostCardReceipt));
    let surface = channelkit_surfaces::ChannelkitSurfaces {
        api_base: &slack.api_base,
        factory: slack.factory().unwrap(),
    };
    assert_eq!(
        surface
            .post_card(
                &channelkit_slack::SlackCard {
                    title: None,
                    text: "Question".into(),
                    actions: vec![]
                },
                None,
                &context
            )
            .await,
        DeliveryResult::Unknown {
            confirmed: vec!["C1:2.0".into()]
        }
    );
}

#[tokio::test]
async fn sdk_history_accepts_its_full_hundred_message_configuration() {
    let server = MockServer::start().await;
    super::tests::bot_identity(&server).await;
    let messages: Vec<_> = (0..100).map(|index| json!({"user":"U1","ts":format!("0.{index:06}"),"text":format!("Previous {index}"),"thread_ts":"0.000000"})).collect();
    Mock::given(path("/conversations.replies"))
        .and(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::query_param("limit", "100"))
        .and(wiremock::matchers::query_param("ts", "0.000000"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"messages":messages})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let slack = Slack::new("fixture-token", "fixture-secret")
        .installation("history-upper-bound")
        .workspace("T1")
        .api_base(server.uri())
        .thread_history(100);
    let mut input = super::tests::mention("Current");
    input["event"]["thread_ts"] = json!("0.000000");
    let Inbound::Message(message) = slack
        .receive(&super::tests::signed(&input, "fixture-secret", unix_now()))
        .await
        .unwrap()
    else {
        panic!("message");
    };
    let history = slack.backfill(&message).await.unwrap();
    assert_eq!(history.len(), 100);
    assert!(history.iter().all(|event| event.actor.actor_id == "U1"
        && event.message_metadata["slack_user"] == "U1"
        && event.routing_metadata["user_id"] == "U1"));
}

#[tokio::test]
async fn progress_formatting_fallback_cannot_create_uneditable_extra_messages() {
    use everruns_contracts::runtime::channel::ChannelProgressSurface;
    let server = MockServer::start().await;
    super::tests::bot_identity(&server).await;
    Mock::given(path("/chat.postMessage"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":false,"error":"invalid_blocks"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let slack = Slack::new("fixture-token", "fixture-secret")
        .installation("progress-single-post")
        .api_base(server.uri());
    let context = slack.delivery_context(&DeliveryTarget::new("C1", "1.0"));
    let surface = channelkit_surfaces::ChannelkitSurfaces {
        api_base: &slack.api_base,
        factory: slack.factory().unwrap(),
    };
    assert!(matches!(
        surface.post_checked(&"x".repeat(40_001), &context).await,
        Err(DeliveryResult::PermanentError(_))
    ));
}
