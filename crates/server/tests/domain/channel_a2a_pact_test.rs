//! PACT Identity profile on an A2A channel (`/v1/channels/{channel_id}/a2a/pact`): personal
//! agents sign their own JWTs, every reply is a synchronous Message, a
//! `contextId` continues only for the same user, and a repeated `messageId`
//! returns the stored reply. A stand-in runner answers each turn, so the
//! tests cover the whole send path without a worker or a model.

use crate::test_harness;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use axum::http::{Method, StatusCode};
use base64::Engine as _;
use everruns_core::events::{EventContext, EventRequest, OutputMessageCompletedData};
use everruns_core::message::RuntimeMessage;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use test_harness::{TestResponse, TestServer};

pub(crate) const ISSUER: &str = "https://pa.example";
pub(crate) const AUDIENCE: &str = "everruns-pact-test";

/// A personal agent's ES256 signing key, minted per test run.
pub(crate) struct PersonalAgentKey {
    encoding: jsonwebtoken::EncodingKey,
    pub(crate) jwk: Value,
    /// The private scalar, for handing the key to PACT's conformance suite.
    pub(crate) d: String,
}

impl PersonalAgentKey {
    pub(crate) fn generate() -> Self {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .unwrap();
        let pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).unwrap();
        let point = pair.public_key().as_ref();
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        // PKCS#8 wraps an ECPrivateKey (`02 01 01 04 20` + 32-byte scalar).
        let der = pkcs8.as_ref();
        let at = der
            .windows(5)
            .position(|w| w == [0x02, 0x01, 0x01, 0x04, 0x20])
            .unwrap()
            + 5;
        Self {
            d: b64(&der[at..at + 32]),
            encoding: jsonwebtoken::EncodingKey::from_ec_der(pkcs8.as_ref()),
            jwk: json!({
                "kty": "EC", "crv": "P-256", "alg": "ES256", "kid": "k1",
                "x": b64(&point[1..33]), "y": b64(&point[33..65]),
            }),
        }
    }

    pub(crate) fn token_with(&self, overrides: Value) -> String {
        let now = chrono::Utc::now().timestamp();
        let mut claims = json!({
            "iss": ISSUER, "aud": AUDIENCE, "sub": "user-1", "iat": now, "exp": now + 120,
        });
        for (key, value) in overrides.as_object().unwrap() {
            claims[key] = value.clone();
        }
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some("k1".into());
        jsonwebtoken::encode(&header, &claims, &self.encoding).unwrap()
    }

    pub(crate) fn token(&self, sub: &str) -> String {
        self.token_with(json!({ "sub": sub }))
    }
}

/// Stands in for the worker: every turn answers with "Reply N".
#[derive(Default)]
pub(crate) struct ReplyingRunner {
    events: OnceLock<Arc<everruns_server::services::EventService>>,
    turns: AtomicUsize,
}

impl ReplyingRunner {
    /// Turns started so far.
    pub(crate) fn turns(&self) -> usize {
        self.turns.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl everruns_core::host::TurnBackend for ReplyingRunner {
    async fn start_turn(
        &self,
        request: everruns_core::host::TurnRequest,
    ) -> everruns_contracts::error::Result<everruns_core::host::TurnTicket> {
        let n = self.turns.fetch_add(1, Ordering::SeqCst) + 1;
        let events = self.events.get().expect("event service wired").clone();
        let session_id = request.session_id;
        let turn_id = request.turn_id;
        tokio::spawn(async move {
            let output =
                OutputMessageCompletedData::new(RuntimeMessage::assistant(format!("Reply {n}")));
            events
                .emit(EventRequest::new(session_id, EventContext::empty(), output))
                .await
                .unwrap();
            let completed: everruns_core::events::TurnCompletedData =
                serde_json::from_value(json!({ "turn_id": turn_id, "iterations": 1 })).unwrap();
            events
                .emit(EventRequest::new(
                    session_id,
                    EventContext::empty(),
                    completed,
                ))
                .await
                .unwrap();
        });
        Ok(everruns_core::host::TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
    ) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: everruns_contracts::typed_id::SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

pub(crate) async fn server() -> (TestServer, Arc<ReplyingRunner>) {
    let runner = Arc::new(ReplyingRunner::default());
    let server = TestServer::in_memory_with_runner(runner.clone()).await;
    runner.events.set(server.event_service.clone()).ok();
    (server, runner)
}

pub(crate) fn pact_config(key: &PersonalAgentKey) -> Value {
    json!({
        "audience": AUDIENCE,
        "personal_agents": [
            { "issuer": ISSUER, "jwks": { "keys": [key.jwk] } },
            { "issuer": "https://disabled.example", "jwks": { "keys": [key.jwk] }, "enabled": false },
        ],
    })
}

/// A published A2A channel, with `pact` when given.
pub(crate) async fn create_channel(server: &TestServer, pact: Option<Value>) -> String {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("pact-{}", uuid::Uuid::new_v4().simple()),
                "display_name": "PACT agent",
                "description": "Answers personal agents.",
                "system_prompt": "You are a brief test agent."
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap();
    let api_key = format!("evra2a_{}", uuid::Uuid::new_v4().simple());
    let mut config = json!({
        "session_mode": "shared_session",
        "message": "{{a2a.text}}",
        "api_key_hash": hex::encode(Sha256::digest(api_key.as_bytes())),
        "api_key_prefix": &api_key[..15],
    });
    if let Some(pact) = pact {
        config["pact"] = pact;
    }
    let channel: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/channels"),
            json!({ "channel_type": "a2a", "channel_config": config }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = channel["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/agents/{agent_id}/channels/{id}/publish"),
            json!({}),
        )
        .await
        .assert_success();
    id
}

pub(crate) async fn call(
    server: &TestServer,
    channel: &str,
    method: Method,
    route: &str,
    token: Option<&str>,
    body: Option<&str>,
) -> TestResponse {
    let auth = token.map(|token| format!("Bearer {token}"));
    let mut headers = vec![("content-type", "application/json")];
    if let Some(auth) = auth.as_deref() {
        headers.push(("authorization", auth));
    }
    server
        .request_raw(
            method,
            &format!("/v1/channels/{channel}/a2a/pact/{route}"),
            headers,
            body.unwrap_or_default().as_bytes().to_vec(),
        )
        .await
}

pub(crate) fn send_body(text: &str, message_id: &str, context_id: Option<&str>) -> String {
    let mut message = json!({
        "messageId": message_id,
        "role": "ROLE_USER",
        "parts": [{ "text": text, "mediaType": "text/plain" }],
    });
    if let Some(context_id) = context_id {
        message["contextId"] = json!(context_id);
    }
    json!({ "message": message }).to_string()
}

async fn send(
    server: &TestServer,
    channel: &str,
    token: &str,
    message_id: &str,
    context_id: Option<&str>,
) -> TestResponse {
    let body = send_body("Is my order late?", message_id, context_id);
    call(
        server,
        channel,
        Method::POST,
        "message:send",
        Some(token),
        Some(&body),
    )
    .await
}

fn assert_a2a_error(response: TestResponse, status: StatusCode, reason: &str) -> Value {
    let response = response.assert_status(status);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/a2a+json"
    );
    let body: Value = response.json();
    assert_eq!(body["error"]["code"], status.as_u16(), "{body}");
    assert_eq!(body["error"]["details"][0]["reason"], reason, "{body}");
    assert_eq!(
        body["error"]["details"][0]["domain"], "a2a-protocol.org",
        "{body}"
    );
    body
}

pub(crate) fn assert_no_a2a_body(response: &TestResponse) {
    assert_ne!(
        response
            .headers()
            .get("content-type")
            .map(|value| value.to_str().unwrap()),
        Some("application/a2a+json")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn card_advertises_http_json_and_the_personal_agent_jwt() {
    let (server, _) = server().await;
    let key = PersonalAgentKey::generate();
    let channel = create_channel(&server, Some(pact_config(&key))).await;

    let card: Value = call(
        &server,
        &channel,
        Method::GET,
        ".well-known/agent-card.json",
        None,
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let interface = &card["supportedInterfaces"][0];
    assert!(
        interface["url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/v1/channels/{channel}/a2a/pact")),
        "{card}"
    );
    assert_eq!(interface["protocolBinding"], "HTTP+JSON");
    assert_eq!(interface["protocolVersion"], "1.0");
    assert_eq!(
        card["securitySchemes"]["platformJwt"]["httpAuthSecurityScheme"]["scheme"],
        "Bearer"
    );

    // A channel without `pact` has no PACT surface at all.
    let plain = create_channel(&server, None).await;
    call(
        &server,
        &plain,
        Method::GET,
        ".well-known/agent-card.json",
        None,
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    let token = key.token("user-1");
    let response = call(&server, &plain, Method::GET, "tasks", Some(&token), None).await;
    let response = response.assert_status(StatusCode::NOT_FOUND);
    assert_no_a2a_body(&response);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replies_are_messages_and_contexts_continue() {
    let (server, runner) = server().await;
    let key = PersonalAgentKey::generate();
    let channel = create_channel(&server, Some(pact_config(&key))).await;
    let token = key.token("user-1");

    let first = send(&server, &channel, &token, "m-1", None)
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        first.headers().get("content-type").unwrap(),
        "application/a2a+json"
    );
    let first: Value = first.json();
    let reply = &first["message"];
    assert_eq!(reply["role"], "ROLE_AGENT", "{first}");
    assert_eq!(reply["parts"], json!([{ "text": "Reply 1" }]));
    assert!(reply.get("taskId").is_none());
    let context_id = reply["contextId"].as_str().unwrap().to_string();

    let second: Value = send(&server, &channel, &token, "m-2", Some(&context_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(second["message"]["contextId"], context_id.as_str());
    assert_eq!(second["message"]["parts"], json!([{ "text": "Reply 2" }]));

    // A retry of `m-1` in its context returns the stored reply and runs nothing.
    let retry: Value = send(&server, &channel, &token, "m-1", Some(&context_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(retry["message"]["messageId"], reply["messageId"]);
    assert_eq!(retry["message"]["parts"], json!([{ "text": "Reply 1" }]));
    assert_eq!(runner.turns.load(Ordering::SeqCst), 2);

    // A new conversation without `contextId` gets its own context, even on a
    // channel whose ordinary mode is one shared session.
    let other: Value = send(&server, &channel, &token, "m-3", None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_ne!(other["message"]["contextId"], context_id.as_str());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_context_belongs_to_one_user_on_one_channel() {
    let (server, _) = server().await;
    let key = PersonalAgentKey::generate();
    let channel = create_channel(&server, Some(pact_config(&key))).await;
    let other_channel = create_channel(&server, Some(pact_config(&key))).await;

    let owner: Value = send(&server, &channel, &key.token("owner"), "m-1", None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    let context_id = owner["message"]["contextId"].as_str().unwrap();

    // Another user, the same user on another channel, and a context that
    // does not exist all get the same answer.
    for (channel, sub, context) in [
        (&channel, "someone-else", context_id),
        (&other_channel, "owner", context_id),
        (&channel, "owner", "not-a-context"),
    ] {
        let body = assert_a2a_error(
            send(&server, channel, &key.token(sub), "m-2", Some(context)).await,
            StatusCode::BAD_REQUEST,
            "INVALID_PARAMS",
        );
        assert_eq!(body["error"]["message"], "Unknown contextId");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authentication_failures_are_a_bare_401() {
    let (server, _) = server().await;
    let key = PersonalAgentKey::generate();
    let channel = create_channel(&server, Some(pact_config(&key))).await;
    let now = chrono::Utc::now().timestamp();
    let unpublished = PersonalAgentKey::generate();
    let hs256 = jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256),
        &json!({ "iss": ISSUER, "aud": AUDIENCE, "sub": "u", "iat": now, "exp": now + 60 }),
        &jsonwebtoken::EncodingKey::from_secret(b"not-an-allowed-platform-key"),
    )
    .unwrap();

    let tokens = [
        None,
        Some(unpublished.token("u")),
        Some(key.token_with(json!({ "aud": "someone-else" }))),
        // Well past the 30 s skew, so a second ticking during the test cannot
        // bring it back inside.
        Some(key.token_with(json!({ "iat": now + 120, "exp": now + 240 }))),
        Some(key.token_with(json!({ "iat": now - 200, "exp": now - 100 }))),
        Some(key.token_with(json!({ "iss": "https://disabled.example" }))),
        Some(key.token_with(json!({ "iss": "https://unknown.example" }))),
        Some(hs256),
    ];
    for token in tokens {
        let response = call(
            &server,
            &channel,
            Method::GET,
            "tasks",
            token.as_deref(),
            None,
        )
        .await;
        let response = response.assert_status(StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers().get("www-authenticate").unwrap(),
            "Bearer realm=\"a2a\""
        );
        assert!(response.text().is_empty());
    }
    // Authentication comes before reading the body.
    call(
        &server,
        &channel,
        Method::POST,
        "message:send",
        None,
        Some("{"),
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);

    // An unknown channel is 404 even with a valid token.
    let response = call(
        &server,
        &format!("ach_{}", uuid::Uuid::new_v4().simple()),
        Method::GET,
        "tasks",
        Some(&key.token("u")),
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    assert_no_a2a_body(&response);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn task_and_unsupported_routes_use_the_pact_errors() {
    let (server, _) = server().await;
    let key = PersonalAgentKey::generate();
    let channel = create_channel(&server, Some(pact_config(&key))).await;
    let token = key.token("u");
    let token = Some(token.as_str());

    let listed: Value = call(
        &server,
        &channel,
        Method::GET,
        "tasks?pageSize=20",
        token,
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(
        listed,
        json!({ "tasks": [], "nextPageToken": "", "pageSize": 20, "totalSize": 0 })
    );
    let default: Value = call(&server, &channel, Method::GET, "tasks", token, None)
        .await
        .json();
    assert_eq!(default["pageSize"], 50);
    assert_a2a_error(
        call(
            &server,
            &channel,
            Method::GET,
            "tasks?pageSize=0",
            token,
            None,
        )
        .await,
        StatusCode::BAD_REQUEST,
        "INVALID_PARAMS",
    );

    let task = uuid::Uuid::new_v4().to_string();
    for (method, route) in [
        (Method::GET, format!("tasks/{task}")),
        (Method::POST, format!("tasks/{task}:cancel")),
    ] {
        let body = assert_a2a_error(
            call(&server, &channel, method, &route, token, None).await,
            StatusCode::NOT_FOUND,
            "TASK_NOT_FOUND",
        );
        assert_eq!(body["error"]["message"], format!("Task not found: {task}"));
    }
    for (method, route) in [
        (Method::POST, "message:stream".to_string()),
        (Method::POST, format!("tasks/{task}:subscribe")),
        (Method::GET, "extendedAgentCard".to_string()),
    ] {
        assert_a2a_error(
            call(&server, &channel, method, &route, token, None).await,
            StatusCode::BAD_REQUEST,
            "UNSUPPORTED_OPERATION",
        );
    }
    for (method, route) in [
        (Method::GET, format!("tasks/{task}/pushNotificationConfigs")),
        (
            Method::POST,
            format!("tasks/{task}/pushNotificationConfigs"),
        ),
        (
            Method::GET,
            format!("tasks/{task}/pushNotificationConfigs/c1"),
        ),
        (
            Method::DELETE,
            format!("tasks/{task}/pushNotificationConfigs/c1"),
        ),
    ] {
        assert_a2a_error(
            call(&server, &channel, method, &route, token, None).await,
            StatusCode::BAD_REQUEST,
            "PUSH_NOTIFICATION_NOT_SUPPORTED",
        );
    }

    // Not a PACT operation: plain 404/405, decided before authentication.
    for (method, route, status) in [
        (
            Method::POST,
            format!("tasks/{task}:archive"),
            StatusCode::NOT_FOUND,
        ),
        (
            Method::GET,
            "message:send".to_string(),
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            Method::POST,
            "tasks".to_string(),
            StatusCode::METHOD_NOT_ALLOWED,
        ),
    ] {
        let response = call(&server, &channel, method, &route, None, None)
            .await
            .assert_status(status);
        assert_no_a2a_body(&response);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_messages_are_refused_before_a_turn() {
    let (server, runner) = server().await;
    let key = PersonalAgentKey::generate();
    let channel = create_channel(&server, Some(pact_config(&key))).await;
    let token = key.token("u");
    let token = Some(token.as_str());
    let with_task = json!({ "message": {
        "messageId": "m-1", "role": "ROLE_USER", "taskId": "task-1", "parts": [{ "text": "hi" }],
    }})
    .to_string();
    let raw = json!({ "message": {
        "messageId": "m-1", "role": "ROLE_USER", "parts": [{ "raw": "aGVsbG8=" }],
    }})
    .to_string();

    for (body, status, reason) in [
        (with_task.as_str(), StatusCode::NOT_FOUND, "TASK_NOT_FOUND"),
        ("{", StatusCode::BAD_REQUEST, "INVALID_PARAMS"),
        (
            raw.as_str(),
            StatusCode::BAD_REQUEST,
            "CONTENT_TYPE_NOT_SUPPORTED",
        ),
    ] {
        assert_a2a_error(
            call(
                &server,
                &channel,
                Method::POST,
                "message:send",
                token,
                Some(body),
            )
            .await,
            status,
            reason,
        );
    }
    assert_eq!(runner.turns.load(Ordering::SeqCst), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pact_config_is_validated() {
    let (server, _) = server().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("pact-{}", uuid::Uuid::new_v4().simple()),
                "display_name": "PACT agent",
                "system_prompt": "You are a brief test agent."
            }),
        )
        .await
        .json();
    let agent_id = agent["id"].as_str().unwrap();
    for pact in [
        json!({ "audience": "", "personal_agents": [{ "issuer": ISSUER, "jwks_uri": "https://pa.example/jwks" }] }),
        json!({ "audience": "a", "personal_agents": [] }),
        json!({ "audience": "a", "personal_agents": [{ "issuer": ISSUER }] }),
        json!({ "audience": "a", "personal_agents": [{ "issuer": ISSUER, "jwks_uri": "http://pa.example/jwks" }] }),
        json!({ "audience": "a", "personal_agents": [{ "issuer": ISSUER, "jwks": { "nope": 1 } }] }),
    ] {
        server
            .post(
                &format!("/v1/agents/{agent_id}/channels"),
                json!({
                    "channel_type": "a2a",
                    "channel_config": {
                        "message": "{{a2a.text}}",
                        "api_key_hash": "h",
                        "api_key_prefix": "evra2a_h",
                        "pact": pact,
                    }
                }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

/// PACT's own conformance suite (`e2e/pact.test.ts` in
/// github.com/openpactprotocol/openpactprotocol) against this server over a
/// real socket. Runs only when `PACT_E2E_DIR` names that suite's directory
/// with its dependencies installed; `scripts/pact-conformance.sh` sets it up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pact_conformance_suite_passes() {
    let Ok(suite_dir) = std::env::var("PACT_E2E_DIR") else {
        return;
    };
    let runner = Arc::new(ReplyingRunner::default());
    let (server, base_url) = TestServer::serving_in_memory_with_runner(runner.clone()).await;
    runner.events.set(server.event_service.clone()).ok();
    let key = PersonalAgentKey::generate();
    let customer = create_channel(&server, Some(pact_config(&key))).await;
    let other_customer = create_channel(&server, Some(pact_config(&key))).await;
    let mut private_jwk = key.jwk.clone();
    private_jwk["d"] = json!(key.d);

    let output = tokio::process::Command::new("pnpm")
        .args([
            "exec",
            "vitest",
            "run",
            "--sequence.concurrent=false",
            "pact.test.ts",
        ])
        .current_dir(&suite_dir)
        .env("PROVIDER_URL", format!("{base_url}/api/v1"))
        .env("CUSTOMER_ID", &customer)
        .env("OTHER_CUSTOMER_ID", &other_customer)
        .env("PA_ISSUER", ISSUER)
        .env("PA_AUDIENCE", AUDIENCE)
        .env("PA_PRIVATE_JWK", private_jwk.to_string())
        .env("E2E_PROVIDER", "any")
        .output()
        .await
        .expect("run pnpm");
    println!("{}", String::from_utf8_lossy(&output.stdout));
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(output.status.success(), "PACT conformance suite failed");
}
