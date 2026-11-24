use everruns_contracts::typed_id::SessionId;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

const SECRET: &str = "8f742231b10e8888abcd99yyyzzz85a5";

pub(super) fn signed(body: &Value, secret: &str, timestamp: i64) -> ChannelRequest {
    let bytes = serde_json::to_vec(body).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(format!("v0:{timestamp}:").as_bytes());
    mac.update(&bytes);
    let signature = hex::encode(mac.finalize().into_bytes());
    ChannelRequest {
        path: "/".into(),
        headers: Vec::new(),
        body: bytes,
    }
    .header("X-Slack-Request-Timestamp", timestamp.to_string())
    .header("X-Slack-Signature", format!("v0={signature}"))
}

pub(super) fn mention(text: &str) -> Value {
    json!({
        "type": "event_callback",
        "event_id": "Ev1",
        "team_id": "T1",
        "event": {"type": "app_mention", "channel": "C1", "user": "U7", "ts": "1.2", "text": text}
    })
}

#[tokio::test]
async fn a_signed_mention_becomes_a_message_in_its_thread() {
    let slack = Slack::new("xoxb-test", SECRET).workspace("T1");
    let inbound = slack
        .receive(&signed(&mention("<@U9> revenue?"), SECRET, unix_now()))
        .await
        .unwrap();
    let Inbound::Message(message) = inbound else {
        panic!("expected a message")
    };
    assert_eq!(message.event.text, "revenue?");
    assert_eq!(message.event.dedup_key, "C1:1.2");
    assert_eq!(message.event.message_metadata["slack_ts"], "1.2");
    assert_eq!(message.event.message_metadata["slack_thread_ts"], "1.2");
    assert_eq!(message.event.actor.actor_id, "U7");
    assert_eq!(message.event.thread_ref.as_deref(), Some("C1:1.2"));
    assert_eq!(message.reply_to.channel_id, "C1");
    assert_eq!(message.reply_to.thread_ref, "1.2");
    assert_eq!(
        message.reply_to.extra[surfaces::SLACK_RECIPIENT_USER_ID],
        "U7"
    );
    assert_eq!(message.reply_to.extra[bridge::TEAM_ID], "T1");
}

#[tokio::test]
async fn a_reply_in_a_thread_keeps_the_thread() {
    let slack = Slack::new("xoxb-test", SECRET).workspace("T1");
    let mut body = mention("more");
    body["event"]["ts"] = json!("1.9");
    body["event"]["thread_ts"] = json!("1.2");
    let Inbound::Message(message) = slack
        .receive(&signed(&body, SECRET, unix_now()))
        .await
        .unwrap()
    else {
        panic!("expected a message")
    };
    assert_eq!(message.event.thread_ref.as_deref(), Some("C1:1.2"));
    assert_eq!(message.reply_to.thread_ref, "1.2");
}

#[tokio::test]
async fn wrong_missing_and_stale_signatures_are_refused() {
    let slack = Slack::new("xoxb-test", SECRET).workspace("T1");
    let body = mention("hi");
    let wrong = signed(&body, "another-secret", unix_now());
    assert_eq!(slack.receive(&wrong).await.unwrap_err().status(), 401);
    let unsigned = ChannelRequest::json(&body);
    assert_eq!(slack.receive(&unsigned).await.unwrap_err().status(), 401);
    let stale = signed(&body, SECRET, unix_now() - 10 * 60);
    assert_eq!(slack.receive(&stale).await.unwrap_err().status(), 401);
}

#[tokio::test]
async fn url_verification_is_answered_directly() {
    let slack = Slack::new("xoxb-test", SECRET).workspace("T1");
    let body = json!({"type": "url_verification", "challenge": "abc"});
    let Inbound::Respond(response) = slack
        .receive(&signed(&body, SECRET, unix_now()))
        .await
        .unwrap()
    else {
        panic!("expected a direct answer")
    };
    assert_eq!(response.body, json!({"challenge": "abc"}));
}

#[tokio::test]
async fn mention_only_ignores_plain_messages_bots_and_edits() {
    let slack = Slack::new("xoxb-test", SECRET)
        .workspace("T1")
        .mention_only();
    let plain = json!({"type": "event_callback", "event_id":"Ev1", "team_id":"T1", "event": {"type": "message", "user":"U7", "channel": "C1", "ts": "1", "text": "hi"}});
    let bot = json!({"type": "event_callback", "event_id":"Ev1", "team_id":"T1", "event": {"type": "app_mention", "bot_id": "B1", "channel": "C1", "ts": "1", "text": "hi"}});
    let edit = json!({"type": "event_callback", "event_id":"Ev1", "team_id":"T1", "event": {"type": "app_mention", "subtype": "message_changed", "channel": "C1", "ts": "1", "message":{"type":"message","user":"U7","ts":"1","text":"edited"}}});
    for body in [plain, bot, edit] {
        let inbound = slack
            .receive(&signed(&body, SECRET, unix_now()))
            .await
            .unwrap();
        assert!(matches!(inbound, Inbound::Ignore), "{body}");
    }
}

#[tokio::test]
async fn files_are_downloaded_from_slack_with_the_bot_token() {
    let mut body = mention("see attached");
    body["event"]["files"] = json!([
        {"id":"F1", "name": "report.pdf", "mimetype": "application/pdf", "size": 42,
         "url_private": "https://files.slack.com/files-pri/T1-F1/report.pdf",
         "url_private_download": "https://files.slack.com/files-pri/T1-F1/download/report.pdf"},
        {"name": "Plan", "mimetype": "application/vnd.google-apps.document", "is_external": true,
         "url_private": "https://docs.google.com/document/d/1",
         "permalink": "https://example.slack.com/files/U7/F2/plan"},
    ]);
    let request = signed(&body, SECRET, unix_now());
    let Inbound::Message(message) = Slack::new("xoxb-test", SECRET)
        .workspace("T1")
        .receive(&request)
        .await
        .unwrap()
    else {
        panic!("expected a message")
    };
    let [first, second] = message.event.attachments.as_slice() else {
        panic!("expected two attachments")
    };
    let InboundAttachment::ProviderFile {
        reference,
        name,
        size,
        ..
    } = first
    else {
        panic!("expected a scoped provider file, got {first:?}");
    };
    let scope: (String, String, String) = serde_json::from_str(reference).unwrap();
    assert_eq!(
        scope,
        ("everruns-sdk-slack".into(), "T1".into(), "F1".into())
    );
    assert_eq!(name.as_deref(), Some("report.pdf"));
    assert_eq!(*size, Some(42));
    assert!(!format!("{first:?}").contains("xoxb-test"));
    assert!(!format!("{first:?}").contains("https://"));
    assert!(matches!(
        second,
        InboundAttachment::FileDescription { name, .. }
            if name == "Plan (https://example.slack.com/files/U7/F2/plan)"
    ));

    // Without a bot token there is nothing to download with.
    let Inbound::Message(message) = Slack::new("", SECRET)
        .workspace("T1")
        .receive(&request)
        .await
        .unwrap()
    else {
        panic!("expected a message")
    };
    assert!(matches!(
        &message.event.attachments[0],
        InboundAttachment::FileDescription { name, .. } if name == "report.pdf"
    ));
}

#[tokio::test]
async fn replies_post_in_the_thread_with_the_bot_token() {
    let server = MockServer::start().await;
    bot_identity(&server).await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .and(header("authorization", "Bearer xoxb-test"))
        // Markdown goes in a `markdown` block; `text` is only the fallback.
        .and(body_partial_json(json!({
            "channel": "C1",
            "thread_ts": "1.2",
            "text": "## hello",
            "blocks": [{"type": "markdown", "text": "## hello"}],
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "ts":"2.0"})))
        .expect(1)
        .mount(&server)
        .await;
    let slack = Slack::new("xoxb-test", SECRET)
        .workspace("T1")
        .api_base(server.uri());
    let context = slack.delivery_context(&DeliveryTarget::new("C1", "1.2"));
    let message = OutboundChannelMessage {
        session_id: SessionId::new(),
        text: "## hello".into(),
        thread_ref: "1.2".into(),
        correlation_id: None,
    };
    assert!(matches!(
        slack.deliver(&message, &context).await,
        DeliveryResult::Ok
    ));
}

#[tokio::test]
async fn rejected_blocks_are_resent_as_plain_text() {
    let server = MockServer::start().await;
    bot_identity(&server).await;
    // Slack fails the whole block over one bad token.
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .and(body_partial_json(json!({"blocks": [{"type": "markdown"}]})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "invalid_blocks"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .and(body_partial_json(
            json!({"channel": "C1", "text": "ping @S404 @here"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true, "ts": "9.9"})))
        .expect(1)
        .mount(&server)
        .await;
    let slack = Slack::new("xoxb-test", SECRET)
        .workspace("T1")
        .api_base(server.uri());
    let context = slack.delivery_context(&DeliveryTarget::new("C1", ""));
    let message = OutboundChannelMessage {
        session_id: SessionId::new(),
        text: "ping <!subteam^S404> <!here>".into(),
        thread_ref: String::new(),
        correlation_id: None,
    };
    assert!(matches!(
        slack.deliver(&message, &context).await,
        DeliveryResult::Ok
    ));
    let requests: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|request| request.url.path() == "/chat.postMessage")
        .collect();
    let retry: Value = serde_json::from_slice(&requests[1].body).unwrap();
    assert!(
        retry.get("blocks").is_none(),
        "the retry must not resend blocks"
    );
}

#[tokio::test]
async fn a_refused_post_is_permanent() {
    let server = MockServer::start().await;
    bot_identity(&server).await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "channel_not_found"})),
        )
        .mount(&server)
        .await;
    let slack = Slack::new("xoxb-test", SECRET)
        .workspace("T1")
        .api_base(server.uri());
    let context = slack.delivery_context(&DeliveryTarget::new("C404", ""));
    let message = OutboundChannelMessage {
        session_id: SessionId::new(),
        text: "hello".into(),
        thread_ref: String::new(),
        correlation_id: None,
    };
    let DeliveryResult::PermanentError(error) = slack.deliver(&message, &context).await else {
        panic!("expected a permanent error")
    };
    assert!(error.contains("channel_not_found"), "{error}");
}

#[test]
fn strip_mentions_keeps_the_rest() {
    assert_eq!(strip_mentions("<@U1> hello <@U2>there"), "hello there");
    assert_eq!(strip_mentions("no mentions"), "no mentions");
    assert_eq!(strip_mentions("broken <@U1"), "broken <@U1");
}

pub(super) async fn bot_identity(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"team_id":"T1","user_id":"U-bot"})),
        )
        .mount(server)
        .await;
}
