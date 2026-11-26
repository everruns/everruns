use base64::{Engine, engine::general_purpose::STANDARD};
use channelkit::http::{Request, Response, Transport};
use channelkit::{Error, WebhookRequest};
use channelkit_agentmail::*;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

fn signed(body: &str, time: i64) -> WebhookRequest {
    let mut h = Hmac::<Sha256>::new_from_slice(b"0123456789abcdef0123456789abcdef").unwrap();
    h.update(format!("evt.{time}.{body}").as_bytes());
    WebhookRequest::post(body)
        .header("svix-id", "evt")
        .header("svix-timestamp", time.to_string())
        .header(
            "svix-signature",
            format!("v1,{}", STANDARD.encode(h.finalize().into_bytes())),
        )
}
fn secret() -> String {
    format!(
        "whsec_{}",
        STANDARD.encode(b"0123456789abcdef0123456789abcdef")
    )
}
const MAIL: &str = r#"{"event_id":"event1","event_type":"message.received","message":{"inbox_id":"support@agentmail.to","message_id":"m1","thread_id":"t1","from":"alice@example.com","to":["support+one@agentmail.to"],"subject":"Hi","text":"hello"}}"#;
#[test]
fn signature_scope_replay_and_event_routing() {
    let req = signed(MAIL, 1000);
    let event = verify_received(&req, &secret(), "support@agentmail.to", 1000)
        .unwrap()
        .unwrap();
    assert_eq!(event.message.to[0], "support+one@agentmail.to");
    assert!(verify_received(&req, &secret(), "other@agentmail.to", 1000).is_err());
    assert!(verify_received(&req, &secret(), "support@agentmail.to", 1301).is_err());
    let mut tampered = req.clone();
    tampered.body.push(b' ');
    assert!(verify_received(&tampered, &secret(), "support@agentmail.to", 1000).is_err());
    let other = signed(r#"{"event_type":"message.delivered"}"#, 1000);
    assert!(
        verify_received(&other, &secret(), "support@agentmail.to", 1000)
            .unwrap()
            .is_none()
    );
    assert!(
        verify_received(
            &WebhookRequest::post(MAIL),
            &secret(),
            "support@agentmail.to",
            1000
        )
        .is_err()
    );
}
#[test]
fn aliases_and_headers_do_not_expand_authority() {
    assert_eq!(
        recipient_tag("a@agentmail.to", "a+one@agentmail.to"),
        Some("one")
    );
    assert_eq!(recipient_tag("a@agentmail.to", "a+two@evil.to"), None);
    assert_eq!(recipient_tag("a@agentmail.to", "ab+one@agentmail.to"), None);
    let mut send = EmailSend {
        to: vec!["alice@example.com".into()],
        cc: vec![],
        subject: "hello".into(),
        text: "hi".into(),
        html: Some("<p>hi</p>".into()),
    };
    assert!(send.validate().is_ok());
    send.to[0] = "alice@example.com\r\nBcc:other@example.com".into();
    assert!(send.validate().is_err());
}
struct Wire {
    requests: Mutex<Vec<Request>>,
    status: u16,
}
#[async_trait::async_trait]
impl Transport for Wire {
    async fn execute(&self, request: Request) -> channelkit::Result<Response> {
        self.requests.lock().unwrap().push(request);
        Ok(Response {
            status: self.status,
            headers: BTreeMap::new(),
            body: br#"{"message_id":"sent","thread_id":"t1"}"#.to_vec(),
        })
    }
}
#[tokio::test]
async fn one_multipart_send_is_idempotent_and_unknown_is_not_retried() {
    for status in [200, 500] {
        let wire = Arc::new(Wire {
            requests: Mutex::new(vec![]),
            status,
        });
        let client = AgentMail::new(
            "test_key".into(),
            "support@agentmail.to".into(),
            wire.clone(),
        )
        .unwrap();
        let send = EmailSend {
            to: vec!["alice@example.com".into()],
            cc: vec![],
            subject: "reply".into(),
            text: "hello".into(),
            html: Some("<p>hello</p>".into()),
        };
        let result = client.send(&send, Some("<message/1>"), "stable-key").await;
        if status == 200 {
            assert!(result.is_ok());
        } else {
            assert!(matches!(result, Err(Error::DeliveryUnknown)));
        }
        let requests = wire.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].headers["idempotency-key"], "stable-key");
        assert!(requests[0].url.contains("%2F"));
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["text"], "hello");
        assert_eq!(body["html"], "<p>hello</p>");
        assert!(body.get("subject").is_none());
        assert_eq!(body["reply_all"], false);
        assert_eq!(body["headers"]["Auto-Submitted"], "auto-replied");
    }
}

#[test]
fn nullable_provider_fields_are_not_parse_failures() {
    let value = serde_json::json!({"inbox_id":"i","message_id":"m","thread_id":"t","cc":null,"subject":null,"preview":null,"headers":null,"attachments":null});
    let mail: EmailMessage = serde_json::from_value(value).unwrap();
    assert!(mail.cc.is_empty() && mail.headers.is_empty() && mail.attachments.is_empty());
    assert!(mail.subject.is_empty() && mail.preview.is_empty());
}

struct ReadWire {
    requests: Mutex<Vec<Request>>,
    body: serde_json::Value,
}
#[async_trait::async_trait]
impl Transport for ReadWire {
    async fn execute(&self, request: Request) -> channelkit::Result<Response> {
        self.requests.lock().unwrap().push(request);
        Ok(Response {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&self.body).unwrap(),
        })
    }
}
#[tokio::test]
async fn scoped_reads_and_pagination_never_follow_payload_urls() {
    let mail: serde_json::Value = serde_json::from_str(MAIL).unwrap();
    let wire = Arc::new(ReadWire {
        requests: Mutex::new(vec![]),
        body: serde_json::json!({"messages":[mail["message"]],"next_page_token":"next"}),
    });
    let client =
        AgentMail::new("test".into(), "support@agentmail.to".into(), wire.clone()).unwrap();
    let page = client
        .list_received("2026-11-25T00:00:00+00:00", Some("opaque/&cursor"))
        .await
        .unwrap();
    assert_eq!(page.messages.len(), 1);
    {
        let requests = wire.requests.lock().unwrap();
        let url = url::Url::parse(&requests[0].url).unwrap();
        assert_eq!(url.host_str(), Some("api.agentmail.to"));
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "after" && v == "2026-11-25T00:00:00Z")
        );
        assert!(
            url.query_pairs()
                .any(|(k, v)| k == "page_token" && v == "opaque/&cursor")
        );
    }
    let wire = Arc::new(ReadWire {
        requests: Mutex::new(vec![]),
        body: mail["message"].clone(),
    });
    let client = AgentMail::new("test".into(), "support@agentmail.to".into(), wire).unwrap();
    assert!(client.get_message("m1").await.is_ok());
    assert!(matches!(
        client.get_message("other").await,
        Err(Error::Scope)
    ));
    assert!(client.get_message("").await.is_err());
}
#[tokio::test]
async fn definite_rate_limit_is_distinct_from_unknown_acceptance() {
    let wire = Arc::new(Wire {
        requests: Mutex::new(vec![]),
        status: 429,
    });
    let client =
        AgentMail::new("test".into(), "support@agentmail.to".into(), wire.clone()).unwrap();
    let mail = EmailSend {
        to: vec!["a@example.com".into()],
        cc: vec![],
        subject: "Hi".into(),
        text: "Hello".into(),
        html: None,
    };
    assert!(matches!(
        client.send(&mail, None, "stable").await,
        Err(Error::RateLimited { .. })
    ));
    assert_eq!(wire.requests.lock().unwrap().len(), 1);
    assert!(client.send(&mail, None, "key with spaces").await.is_err());
    assert_eq!(wire.requests.lock().unwrap().len(), 1);
}
