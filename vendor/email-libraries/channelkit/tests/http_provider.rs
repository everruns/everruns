#![cfg(feature = "http")]

use async_trait::async_trait;
use channelkit::http::Request;
use channelkit::http::provider::*;
use channelkit::http::{Response, Transport};
use channelkit::{Adapter, Button, Card, Provider, WebhookResponse};
use channelkit::{Content, ConversationKey, Error, Result, WebhookRequest};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use std::{collections::BTreeMap, sync::Arc};
struct Reply {
    status: u16,
    body: Vec<u8>,
    fail: bool,
}
#[async_trait]
impl Transport for Reply {
    async fn execute(&self, request: Request) -> Result<Response> {
        assert_eq!(request.url, "http://127.0.0.1/api/a%2Fb?q=a+b");
        assert!(request.headers["authorization"].ends_with("synthetic"));
        assert_eq!(request.headers["x-test"], "test");
        if !request.body.is_empty() {
            assert_eq!(
                serde_json::from_slice::<Value>(&request.body).unwrap(),
                serde_json::json!({"a":1})
            );
        }
        if self.fail {
            return Err(Error::Transport);
        }
        Ok(Response {
            status: self.status,
            headers: BTreeMap::new(),
            body: self.body.clone(),
        })
    }
}
struct Token;
#[async_trait]
impl TokenProvider for Token {
    async fn token(&self) -> Result<String> {
        Ok("synthetic".into())
    }
}
#[tokio::test]
async fn http_outcomes_and_paths() {
    for scheme in [
        Authorization::Bearer,
        Authorization::Bot,
        Authorization::Raw,
    ] {
        for (status, body, fail, write, expected) in [
            (200, b"{}".as_slice(), false, true, "ok"),
            (204, b"".as_slice(), false, true, "ok"),
            (429, b"{}".as_slice(), false, true, "rate"),
            (503, b"{}".as_slice(), false, true, "unknown"),
            (408, b"{}".as_slice(), false, true, "unknown"),
            (403, b"sensitive".as_slice(), false, true, "rejected"),
            (503, b"{}".as_slice(), false, false, "rejected"),
            (200, b"bad".as_slice(), false, true, "unknown"),
            (200, b"bad".as_slice(), false, false, "invalid"),
            (200, b"{}".as_slice(), true, true, "unknown"),
            (200, b"{}".as_slice(), true, false, "transport"),
        ] {
            let client = JsonClient::new("https://example.com", "synthetic", scheme)
                .unwrap()
                .api_origin("http://127.0.0.1/api")
                .unwrap()
                .header("x-test", "test")
                .token_provider(Arc::new(Token))
                .transport(Arc::new(Reply {
                    status,
                    body: body.to_vec(),
                    fail,
                }));
            let result = client
                .call(
                    "POST",
                    &["a/b"],
                    &[("q", "a b".into())],
                    if write {
                        Some(serde_json::json!({"a":1}))
                    } else {
                        None
                    },
                    write,
                )
                .await;
            assert!(match expected {
                "ok" => result.is_ok(),
                "rate" => matches!(result, Err(Error::RateLimited { .. })),
                "unknown" => matches!(result, Err(Error::DeliveryUnknown)),
                "rejected" => matches!(result, Err(Error::Rejected { .. })),
                "invalid" => matches!(result, Err(Error::InvalidResponse)),
                "transport" => matches!(result, Err(Error::Transport)),
                _ => false,
            });
        }
    }
    let client =
        JsonClient::new("https://example.com", "synthetic", Authorization::Bearer).unwrap();
    for segment in ["", ".", "..", &"a".repeat(4097)] {
        assert!(
            client
                .call("GET", &[segment], &[], None, false)
                .await
                .is_err()
        );
    }
    assert!(JsonClient::new("https://example.com", "", Authorization::Bearer).is_err());
    assert!(JsonClient::new("http://example.com", "synthetic", Authorization::Bearer).is_err());
    assert!(client.api_origin("https://user:pass@example.com").is_err());
}
#[test]
fn validation_and_webhooks() {
    assert!(unix_now() > 0);
    for secret in ["", "\n", &"x".repeat(16385)] {
        assert!(validate_secret(secret).is_err());
    }
    assert!(validate_secret("synthetic").is_ok());
    for value in [
        serde_json::json!(""),
        Value::Null,
        serde_json::json!("\n"),
        serde_json::json!("x".repeat(257)),
    ] {
        assert!(identifier(&value).is_err());
    }
    assert_eq!(identifier(&serde_json::json!(42)).unwrap(), "42");
    assert!(required(&Value::Null, "id").is_err());
    for provider in [
        Provider::Slack,
        Provider::Whatsapp,
        Provider::Discord,
        Provider::Teams,
        Provider::Github,
        Provider::Linear,
        Provider::X,
        Provider::Web,
    ] {
        assert_eq!(provider.as_str().parse::<Provider>().unwrap(), provider);
        assert_eq!(
            serde_json::from_str::<Provider>(&serde_json::to_string(&provider).unwrap()).unwrap(),
            provider
        );
    }
    let key = ConversationKey::new(Provider::Web, "a", "b", None).unwrap();
    assert!(receipt(&key, &Value::Null).is_err());
    assert_eq!(
        receipt(&key, &serde_json::json!(1)).unwrap().messages[0].id,
        "1"
    );
    assert_eq!(
        text(Content::Markdown("markdown".into())).unwrap(),
        "markdown"
    );
    assert!(text(Content::Text("x".repeat(channelkit::MAX_OUTPUT_BYTES + 1))).is_err());
    assert!(
        text(Content::Card(Card {
            title: "title".into(),
            body: "body".into(),
            actions: vec![Button::new("a", "A")]
        }))
        .is_err()
    );
    assert_eq!(
        text(Content::Card(Card {
            title: "title".into(),
            body: "body".into(),
            actions: vec![]
        }))
        .unwrap(),
        "## title\n\nbody"
    );
    for decimal_value in ["", "x", &"1".repeat(33)] {
        assert!(decimal(decimal_value).is_err());
    }
    for (cursor, limit) in [(Some("0"), 1), (Some("bad"), 1), (None, 0), (None, 101)] {
        assert!(page(cursor, limit).is_err());
    }
    assert_eq!(page(None, 10).unwrap(), ("1".into(), "10".into()));
    assert_eq!(
        next_page(
            &BTreeMap::from([("link".into(), "<url>; rel=\"next\"".into())]),
            "1"
        ),
        Some("2".into())
    );
    assert!(next_page(&BTreeMap::new(), "1").is_none());
    let mut request = WebhookRequest::post(b"{}".to_vec());
    let mut mac = Hmac::<Sha256>::new_from_slice(b"synthetic").unwrap();
    mac.update(&request.body);
    request = request.header(
        "sig",
        format!("sha256={}", hex::encode(mac.finalize().into_bytes())),
    );
    assert!(verify_hmac_hex(&request, "sig", "synthetic", "sha256=").is_ok());
    assert!(json_webhook(&request).is_ok());
    request.body.push(b' ');
    assert!(verify_hmac_hex(&request, "sig", "synthetic", "sha256=").is_err());
    request.headers.clear();
    assert!(verify_hmac_hex(&request, "sig", "synthetic", "sha256=").is_err());
    request = request.header("sig", format!("sha256={}", "z".repeat(64)));
    assert!(verify_hmac_hex(&request, "sig", "synthetic", "sha256=").is_err());
    request.method = "GET".into();
    assert!(verify_hmac_hex(&request, "sig", "synthetic", "sha256=").is_err());
    assert!(json_webhook(&request).is_err());
    assert!(json_webhook(&WebhookRequest::post(b"invalid".to_vec())).is_err());
    request.body = vec![b' '; channelkit::MAX_WEBHOOK_BYTES + 1];
    assert!(verify_hmac_hex(&request, "sig", "synthetic", "sha256=").is_err());
    let _ = WebhookResponse::text("ok");
    // Adapter imports exercise the public trait under the optional HTTP feature.
    fn _object_safe(_: &dyn Adapter) {}
}
