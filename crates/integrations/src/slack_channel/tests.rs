use everruns_contracts::typed_id::SessionId;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const SECRET: &str = "8f742231b10e8888abcd99yyyzzz85a5";

fn signed(body: &Value, secret: &str, timestamp: i64) -> ChannelRequest {
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

fn mention(text: &str) -> Value {
    json!({
        "type": "event_callback",
        "event_id": "Ev1",
        "event": {"type": "app_mention", "channel": "C1", "user": "U7", "ts": "1.2", "text": text}
    })
}

#[tokio::test]
async fn a_signed_mention_becomes_a_message_in_its_thread() {
    let slack = Slack::new("xoxb-test", SECRET);
    let inbound = slack
        .receive(&signed(&mention("<@U9> revenue?"), SECRET, unix_now()))
        .await
        .unwrap();
    let Inbound::Message(message) = inbound else {
        panic!("expected a message")
    };
    assert_eq!(message.event.text, "revenue?");
    assert_eq!(message.event.dedup_key, "Ev1");
    assert_eq!(message.event.actor.actor_id, "U7");
    assert_eq!(message.event.thread_ref.as_deref(), Some("C1:1.2"));
    assert_eq!(message.reply_to, DeliveryTarget::new("C1", "1.2"));
}

#[tokio::test]
async fn a_reply_in_a_thread_keeps_the_thread() {
    let slack = Slack::new("xoxb-test", SECRET);
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
    let slack = Slack::new("xoxb-test", SECRET);
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
    let slack = Slack::new("xoxb-test", SECRET);
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
    let slack = Slack::new("xoxb-test", SECRET).mention_only();
    let plain = json!({"type": "event_callback", "event": {"type": "message", "channel": "C1", "ts": "1", "text": "hi"}});
    let bot = json!({"type": "event_callback", "event": {"type": "app_mention", "bot_id": "B1", "channel": "C1", "ts": "1", "text": "hi"}});
    let edit = json!({"type": "event_callback", "event": {"type": "app_mention", "subtype": "message_changed", "channel": "C1", "ts": "1"}});
    for body in [plain, bot, edit] {
        let inbound = slack
            .receive(&signed(&body, SECRET, unix_now()))
            .await
            .unwrap();
        assert!(matches!(inbound, Inbound::Ignore), "{body}");
    }
}

#[tokio::test]
async fn replies_post_in_the_thread_with_the_bot_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat.postMessage"))
        .and(header("authorization", "Bearer xoxb-test"))
        .and(body_partial_json(
            json!({"channel": "C1", "thread_ts": "1.2", "text": "hello"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(1)
        .mount(&server)
        .await;
    let slack = Slack::new("xoxb-test", SECRET).api_base(server.uri());
    let context = slack.delivery_context(&DeliveryTarget::new("C1", "1.2"));
    let message = OutboundChannelMessage {
        session_id: SessionId::new(),
        text: "hello".into(),
        thread_ref: "1.2".into(),
        correlation_id: None,
    };
    assert!(matches!(
        slack.deliver(&message, &context).await,
        DeliveryResult::Ok
    ));
}

#[tokio::test]
async fn a_refused_post_is_permanent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok": false, "error": "channel_not_found"})),
        )
        .mount(&server)
        .await;
    let slack = Slack::new("xoxb-test", SECRET).api_base(server.uri());
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
fn from_env_names_its_secrets_and_new_names_none() {
    assert_eq!(
        Slack::from_env().secrets(),
        vec![
            "SLACK_BOT_TOKEN".to_string(),
            "SLACK_SIGNING_SECRET".to_string()
        ]
    );
    assert!(Slack::new("t", "s").secrets().is_empty());
}

#[test]
fn strip_mentions_keeps_the_rest() {
    assert_eq!(strip_mentions("<@U1> hello <@U2>there"), "hello there");
    assert_eq!(strip_mentions("no mentions"), "no mentions");
    assert_eq!(strip_mentions("broken <@U1"), "broken <@U1");
}
