#![cfg(feature = "chatgpt")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
use async_trait::async_trait;
use base64::Engine as _;
use everruns_drivers::chatgpt::{
    ChatGptChatDriver, CodexAuth, OpenSourceGrant,
    auth::{RotatingAuth, TokenRoute, TokenStore},
    login::LoginAttempt,
    oauth::{self, Endpoints},
};
use everruns_contracts::{
    BearerAuth, LlmCallConfig, LlmStreamEvent, Message, MessageRole, Provider,
};
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn codex_compaction_cooldown_is_account_scoped_and_preserves_opaque_output() {
    use everruns_drivers::codex::CodexChatDriver;
    use everruns_contracts::{ChatDriver, CompactRequest};
    use wiremock::matchers::header;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/responses/compact"))
        .and(header("chatgpt-account-id", "unavailable"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/responses/compact"))
        .and(header("chatgpt-account-id", "available"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "output":[{"type":"compaction","encrypted_content":"opaque-provider-state"}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let driver = CodexChatDriver::new();
    let first = Provider::new("first", driver.clone())
        .base_url(server.uri())
        .auth(BearerAuth::new("first"))
        .header("chatgpt-account-id", "unavailable");
    let second = Provider::new("second", driver.clone())
        .base_url(server.uri())
        .auth(BearerAuth::new("second"))
        .header("chatgpt-account-id", "available");
    let request = CompactRequest {
        reasoning_state: None,
        model: "test".into(),
        input: vec![],
        previous_response_id: None,
        instructions: None,
    };
    assert!(
        driver
            .compact(first.endpoint(), request.clone())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        driver
            .compact(first.endpoint(), request.clone())
            .await
            .unwrap()
            .is_none()
    );
    let output = driver
        .compact(second.endpoint(), request)
        .await
        .unwrap()
        .unwrap()
        .output;
    assert_eq!(
        serde_json::to_value(output).unwrap(),
        json!([{"type":"compaction","encrypted_content":"opaque-provider-state"}])
    );
}

fn sign(claims: &Value) -> String {
    let key = base64::engine::general_purpose::STANDARD
        .decode(
            include_str!("fixtures/chatgpt_test_rsa.b64")
                .split_whitespace()
                .collect::<String>(),
        )
        .unwrap();
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test".into());
    jsonwebtoken::encode(
        &header,
        claims,
        &jsonwebtoken::EncodingKey::from_rsa_der(&key),
    )
    .unwrap()
}
fn keys() -> oauth::Jwks {
    serde_json::from_str(include_str!("fixtures/chatgpt_test_jwks.json")).unwrap()
}
fn claims() -> Value {
    json!({"iss":"https://auth.openai.com","aud":"app_test","sub":"subject","exp":2000000000i64,"nonce":"nonce","email":"user@example.com"})
}
#[test]
fn identity_requires_signature_issuer_client_expiry_nonce_and_subject() {
    let good = claims();
    let validate = |s: &str| {
        oauth::validate_id_token(
            s,
            &keys(),
            "https://auth.openai.com",
            "app_test",
            "nonce",
            1900000000,
        )
    };
    assert_eq!(validate(&sign(&good)).unwrap().subject, "subject");
    for (field, value) in [
        ("iss", json!("https://evil.example")),
        ("aud", json!("app_other")),
        ("nonce", json!("other")),
        ("exp", json!(1800000000)),
        ("sub", json!("")),
        ("azp", json!("app_other")),
        ("nbf", json!(2000000000)),
    ] {
        let mut bad = good.clone();
        bad[field] = value;
        assert!(validate(&sign(&bad)).is_err(), "{field}");
    }
    let token = sign(&good);
    let parts: Vec<_> = token.split('.').collect();
    let forged = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({"sub":"attacker"}).to_string());
    assert!(validate(&format!("{}.{}.{}", parts[0], forged, parts[2])).is_err());
    assert!(oauth::issued_client_id(None, Some(oauth::DYNAMIC_CLIENT_ID)).is_err());
    assert!(oauth::issued_client_id(Some("app_test"), Some("app_other")).is_err());
}
struct Tokens {
    gate: Arc<Mutex<()>>,
    auth: Mutex<Option<CodexAuth>>,
    saves: AtomicUsize,
}
#[async_trait]
impl TokenStore for Tokens {
    async fn lock(&self) -> anyhow::Result<Box<dyn Send>> {
        Ok(Box::new(self.gate.clone().lock_owned().await))
    }
    async fn load(&self) -> anyhow::Result<Option<CodexAuth>> {
        Ok(self.auth.lock().await.clone())
    }
    async fn save(&self, auth: CodexAuth) -> anyhow::Result<()> {
        *self.auth.lock().await = Some(auth);
        self.saves.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}
fn auth() -> CodexAuth {
    CodexAuth {
        access_token: "old-access".into(),
        refresh_token: Some("old-refresh".into()),
        expires_at: Some(0),
        account_id: None,
        email: None,
        client_id: Some("app_test".into()),
        open_source: Some(OpenSourceGrant {
            id_token: None,
            scopes: vec![oauth::PLAN_SCOPE.into()],
            subject: Some("subject".into()),
        }),
    }
}
#[tokio::test]
async fn concurrent_refresh_loads_under_lease_and_persists_full_rotated_pair() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let tokens = Arc::new(Tokens {
        gate: Arc::new(Mutex::new(())),
        auth: Mutex::new(Some(auth())),
        saves: AtomicUsize::new(0),
    });
    let driver = RotatingAuth::new(tokens.clone(), TokenRoute::ChatGptPlan)
        .with_token_url(format!("{}/token", server.uri()));
    let (a, b) = tokio::join!(driver.token(), driver.token());
    assert_eq!(a.unwrap().access_token, "new-access");
    assert_eq!(b.unwrap().refresh_token.as_deref(), Some("new-refresh"));
    assert_eq!(tokens.saves.load(Ordering::Relaxed), 1);
    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    let form: std::collections::HashMap<_, _> =
        reqwest::Url::parse(&format!("http://localhost/?{body}"))
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
    assert_eq!(form["client_id"], "app_test");
    assert_eq!(form["resource"], oauth::RESOURCE);
}
#[tokio::test]
async fn declined_scope_and_failed_refresh_keep_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401).set_body_string("credential-body-must-not-leak"))
        .mount(&server)
        .await;
    let tokens = Arc::new(Tokens {
        gate: Arc::new(Mutex::new(())),
        auth: Mutex::new(Some(auth())),
        saves: AtomicUsize::new(0),
    });
    let driver =
        RotatingAuth::new(tokens.clone(), TokenRoute::ChatGptPlan).with_token_url(server.uri());
    assert!(
        !driver
            .token()
            .await
            .unwrap_err()
            .to_string()
            .contains("credential-body")
    );
    assert_eq!(tokens.saves.load(Ordering::Relaxed), 0);
    tokens
        .auth
        .lock()
        .await
        .as_mut()
        .unwrap()
        .open_source
        .as_mut()
        .unwrap()
        .scopes
        .clear();
    assert!(driver.token().await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
async fn collect(sse: &str) -> (Vec<everruns_contracts::error::Result<LlmStreamEvent>>, Value) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .insert_header("connection", "close")
                .set_body_string(sse),
        )
        .mount(&server)
        .await;
    let driver = Provider::new("test", ChatGptChatDriver::new())
        .base_url(format!("{}/v1", server.uri()))
        .auth(BearerAuth::new("access"));
    let config = LlmCallConfig::new("model");
    let events = driver
        .chat_completion_stream(
            vec![
                Message::text(MessageRole::System, "instructions"),
                Message::text(MessageRole::User, "hi"),
            ],
            &config,
        )
        .await
        .unwrap()
        .collect()
        .await;
    let body = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    (events, body)
}
#[tokio::test]
async fn foreground_preserves_store_false_and_requires_response_completed() {
    let (events,body)=collect("event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n").await;
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["instructions"], "instructions");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(LlmStreamEvent::Error(_))))
    );
    for field in everruns_drivers::chatgpt::REJECTED_FIELDS {
        assert!(body.get(*field).is_none(), "{field}");
    }
    let (events,_)=collect("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"completed\",\"output\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n\n").await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(LlmStreamEvent::Done { .. })))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Ok(LlmStreamEvent::Error(_))))
    );
}
#[tokio::test]
async fn discovers_only_visible_models_in_provider_order() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/v1/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"models":[{"slug":"second","display_name":"Second","visibility":"list"},{"slug":"hidden","visibility":"hide"},{"slug":"first","visibility":"list"}]}))).mount(&server).await;
    let provider = Provider::new("test", ChatGptChatDriver::new())
        .base_url(format!("{}/v1", server.uri()))
        .auth(BearerAuth::new("access"));
    let models = provider.list_models().await.unwrap().unwrap();
    assert_eq!(
        models
            .iter()
            .map(|m| m.model_id.as_str())
            .collect::<Vec<_>>(),
        ["second", "first"]
    );
}
#[tokio::test]
async fn login_registers_with_host_pkce_and_exchanges_with_issued_client() {
    let server = MockServer::start().await;
    let endpoints = Endpoints {
        issuer: "https://auth.openai.com".into(),
        authorize: format!("{}/authorize", server.uri()),
        token: format!("{}/token", server.uri()),
        revoke: format!("{}/revoke", server.uri()),
        jwks: format!("{}/jwks", server.uri()),
    };
    let attempt = LoginAttempt::start(
        endpoints,
        "Example app",
        "urn:uuid:00000000-0000-4000-8000-000000000000",
        None,
        None,
        false,
    )
    .await
    .unwrap();
    let url = reqwest::Url::parse(&attempt.authorize_url).unwrap();
    let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], oauth::DYNAMIC_CLIENT_ID);
    assert_eq!(query["agent_name_hint"], "Example app");
    assert_eq!(query["code_challenge_method"], "S256");
    let mut c = claims();
    c["nonce"] = json!(query["nonce"]);
    c["exp"] = json!(oauth::now_epoch_millis() / 1000 + 3600);
    Mock::given(method("POST")).and(path("/token")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token":"access","refresh_token":"refresh","expires_in":3600,"id_token":sign(&c),"scope":oauth::SCOPE}))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/jwks"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            include_str!("fixtures/chatgpt_test_jwks.json"),
            "application/json",
        ))
        .mount(&server)
        .await;
    let callback = format!(
        "{}?state={}&code=code&client_id=app_test",
        query["redirect_uri"], query["state"]
    );
    let (result, response) = tokio::join!(attempt.finish(), reqwest::get(callback));
    assert!(response.unwrap().status().is_success());
    let result = result.unwrap();
    assert_eq!(result.client_id.as_deref(), Some("app_test"));
    assert_eq!(
        result.open_source.unwrap().subject.as_deref(),
        Some("subject")
    );
    let requests = server.received_requests().await.unwrap();
    let request = requests.iter().find(|r| r.url.path() == "/token").unwrap();
    let body = String::from_utf8(request.body.clone()).unwrap();
    let form: std::collections::HashMap<_, _> =
        reqwest::Url::parse(&format!("http://localhost/?{body}"))
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect();
    assert_eq!(form["client_id"], "app_test");
    assert_eq!(form["redirect_uri"], query["redirect_uri"]);
    assert_eq!(form["resource"], oauth::RESOURCE);
}

#[tokio::test]
async fn in_band_quota_errors_keep_their_code_and_are_non_transient() {
    let (events, _) = collect("event: response.failed\ndata: {\"type\":\"response.failed\",\"response\":{\"id\":\"r1\",\"status\":\"failed\",\"output\":[],\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"Plan allowance exhausted\"}}}\n\n").await;
    let error = events
        .into_iter()
        .find_map(|e| match e {
            Ok(LlmStreamEvent::Error(e)) if e.code.is_some() => Some(e),
            _ => None,
        })
        .expect("provider code preserved");
    assert_eq!(
        error.code.as_deref(),
        Some("subscription_sharing_usage_limit_exceeded")
    );
    assert_eq!(
        error.kind(),
        everruns_contracts::LlmErrorKind::QuotaExhausted
    );
}
#[tokio::test]
async fn invalid_completion_is_not_success() {
    let (events, _) = collect("event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"r1\",\"status\":\"failed\",\"output\":[]}}\n\n").await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(LlmStreamEvent::Error(_))))
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, Ok(LlmStreamEvent::Done { .. })))
    );
}
#[tokio::test]
async fn refreshed_credentials_cannot_restore_a_removed_connection() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(60))
                .set_body_json(
                    json!({"access_token":"fresh","refresh_token":"rotated","expires_in":3600}),
                ),
        )
        .mount(&server)
        .await;
    let tokens = Arc::new(Tokens {
        gate: Arc::new(Mutex::new(())),
        auth: Mutex::new(Some(auth())),
        saves: AtomicUsize::new(0),
    });
    let driver =
        RotatingAuth::new(tokens.clone(), TokenRoute::ChatGptPlan).with_token_url(server.uri());
    let refresh = tokio::spawn(async move { driver.token().await });
    while server.received_requests().await.unwrap().is_empty() {
        tokio::task::yield_now().await;
    }
    *tokens.auth.lock().await = None;
    assert!(refresh.await.unwrap().is_err());
    assert!(tokens.auth.lock().await.is_none());
    assert_eq!(tokens.saves.load(Ordering::Relaxed), 0);
}
#[tokio::test]
async fn local_tools_are_namespaced_and_only_supported_hosted_tools_are_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).respond_with(ResponseTemplate::new(200)
        .insert_header("content-type","text/event-stream")
        .set_body_string("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r\",\"status\":\"completed\",\"output\":[]}}\n\n")).mount(&server).await;
    let provider = Provider::new("test", ChatGptChatDriver::new())
        .base_url(server.uri())
        .auth(BearerAuth::new("access"));
    let mut config = LlmCallConfig::new("model");
    config
        .tools
        .push(everruns_contracts::ToolDefinition::function(
            "read_file",
            "Read a file",
            json!({"type":"object","properties":{}}),
        ));
    config
        .driver_options
        .insert("openai/hosted_tools".into(), json!({"web_search":{}}));
    provider
        .chat_completion_stream(vec![Message::text(MessageRole::User, "hi")], &config)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert!(
        body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["type"] == "namespace" && t["tools"][0]["name"] == "read_file")
    );
    assert!(
        body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["type"] == "web_search")
    );
    config.driver_options.insert(
        "openai/hosted_tools".into(),
        json!({"code_interpreter":{"container":{"type":"auto"}}}),
    );
    assert!(
        provider
            .chat_completion_stream(vec![], &config)
            .await
            .is_err()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}
