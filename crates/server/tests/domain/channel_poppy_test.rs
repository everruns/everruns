//! Personal Agent Protocol ("Poppy") channel (`/v1/channels/{channel_id}/poppy`):
//! discovery, Sessions started with a personal agent's own assertions and
//! DPoP-bound Session Tokens, and conversations over the channel's agent. A
//! stand-in runner answers each turn, and the personal agent's client
//! document and keys are served from the harness instead of the network.

use crate::channel_a2a_pact_test::{ReplyingRunner, server};
use crate::test_harness;

use std::sync::Arc;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use axum::http::{Method, StatusCode};
use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use test_harness::{TestResponse, TestServer};

const HOST: &str = "company.test";
const CLIENT_ID: &str = "https://pa.example/agent.json";
const JWKS_URI: &str = "https://pa.example/jwks.json";

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// An ES256 key pair, as the personal agent's published key or its DPoP key.
struct Key {
    encoding: jsonwebtoken::EncodingKey,
    jwk: Value,
}

impl Key {
    fn generate() -> Self {
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                .unwrap();
        let pair =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).unwrap();
        let point = pair.public_key().as_ref();
        Self {
            encoding: jsonwebtoken::EncodingKey::from_ec_der(pkcs8.as_ref()),
            jwk: json!({
                "kty": "EC", "crv": "P-256",
                "x": b64(&point[1..33]), "y": b64(&point[33..65]),
            }),
        }
    }

    /// A JWT signed as the personal agent, with `kid` `k1`.
    fn assertion(&self, claims: Value) -> String {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some("k1".into());
        jsonwebtoken::encode(&header, &claims, &self.encoding).unwrap()
    }

    /// A DPoP proof for `method` on `url`, bound to `token` when given.
    fn proof(&self, method: &str, url: &str, token: Option<&str>) -> String {
        let mut claims = json!({
            "jti": uuid::Uuid::new_v4().to_string(),
            "htm": method,
            "htu": url,
            "iat": chrono::Utc::now().timestamp(),
        });
        if let Some(token) = token {
            claims["ath"] = json!(b64(&Sha256::digest(token.as_bytes())));
        }
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.typ = Some("dpop+jwt".into());
        header.jwk = Some(serde_json::from_value(self.jwk.clone()).unwrap());
        jsonwebtoken::encode(&header, &claims, &self.encoding).unwrap()
    }
}

/// A personal agent: its published signing key and a DPoP key.
struct Agent {
    signing: Key,
    dpop: Key,
}

impl Agent {
    async fn publish(server: &TestServer) -> Self {
        let signing = Key::generate();
        let mut jwk = signing.jwk.clone();
        jwk["kid"] = json!("k1");
        jwk["alg"] = json!("ES256");
        server
            .poppy_documents
            .prime_public_document(
                CLIENT_ID,
                json!({
                    "client_id": CLIENT_ID,
                    "client_name": "Example Agent",
                    "jwks_uri": JWKS_URI,
                    "redirect_uris": ["https://pa.example/oauth/callback"],
                    "token_endpoint_auth_method": "private_key_jwt",
                }),
            )
            .await;
        server
            .poppy_documents
            .prime_public_document(JWKS_URI, json!({ "keys": [jwk] }))
            .await;
        Self {
            signing,
            dpop: Key::generate(),
        }
    }

    fn signed(&self, sub: &str, aud: &str) -> String {
        let now = chrono::Utc::now().timestamp();
        self.signing.assertion(json!({
            "iss": CLIENT_ID, "sub": sub, "aud": aud,
            "iat": now, "exp": now + 60, "jti": uuid::Uuid::new_v4().simple().to_string(),
        }))
    }
}

fn url(channel: &str, path: &str) -> String {
    format!("https://{HOST}/api/v1/channels/{channel}/poppy{path}")
}

async fn create_channel(server: &TestServer, config: Value) -> TestResponse {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("poppy-{}", uuid::Uuid::new_v4().simple()),
                "display_name": "Support agent",
                "description": "Answers personal agents.",
                "system_prompt": "You are a brief test agent."
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap().to_string();
    let response = server
        .post(
            &format!("/v1/agents/{agent_id}/channels"),
            json!({ "channel_type": "poppy", "channel_config": config }),
        )
        .await;
    if response.status() != StatusCode::CREATED {
        return response;
    }
    let id = response.json::<Value>()["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/agents/{agent_id}/channels/{id}/publish"),
            json!({}),
        )
        .await
        .assert_success();
    response
}

async fn channel(server: &TestServer) -> String {
    let created = create_channel(
        server,
        json!({
            "organization_name": "Example Co",
            "domains": ["example.com", "example.co.uk"],
        }),
    )
    .await
    .assert_status(StatusCode::CREATED);
    created.json::<Value>()["id"].as_str().unwrap().to_string()
}

async fn get(server: &TestServer, uri: &str) -> TestResponse {
    server
        .request_raw(Method::GET, uri, vec![("host", HOST)], vec![])
        .await
}

/// `POST {issuer}/oauth/token` with a form body.
async fn token_request(
    server: &TestServer,
    channel: &str,
    agent: &Agent,
    params: &[(&str, String)],
    dpop: bool,
) -> TestResponse {
    let token_url = url(channel, "/oauth/token");
    let mut form = vec![
        ("client_id", CLIENT_ID.to_string()),
        (
            "client_assertion_type",
            "urn:ietf:params:oauth:client-assertion-type:jwt-bearer".to_string(),
        ),
        ("client_assertion", agent.signed(CLIENT_ID, &token_url)),
    ];
    form.extend(params.iter().cloned());
    let body = serde_urlencoded::to_string(&form).unwrap();
    let proof = agent.dpop.proof("POST", &token_url, None);
    let mut headers = vec![
        ("host", HOST),
        ("content-type", "application/x-www-form-urlencoded"),
    ];
    if dpop {
        headers.push(("dpop", proof.as_str()));
    }
    server
        .request_raw(
            Method::POST,
            &format!("/v1/channels/{channel}/poppy/oauth/token"),
            headers,
            body.into_bytes(),
        )
        .await
}

fn session_grant(agent: &Agent, channel: &str, sub: &str) -> Vec<(&'static str, String)> {
    vec![
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:jwt-bearer".to_string(),
        ),
        (
            "assertion",
            agent.signed(sub, &url(channel, "/oauth/token")),
        ),
    ]
}

/// Start a Session for `sub`; returns (Session Token, session id).
async fn start_session(
    server: &TestServer,
    channel: &str,
    agent: &Agent,
    sub: &str,
) -> (String, String) {
    let body: Value = token_request(
        server,
        channel,
        agent,
        &session_grant(agent, channel, sub),
        true,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(body["token_type"], "DPoP");
    assert_eq!(body["signed_in"], false);
    assert_eq!(body["scope"], "");
    (
        body["access_token"].as_str().unwrap().to_string(),
        body["session_id"].as_str().unwrap().to_string(),
    )
}

/// A conversation request with the Session Token and a fresh proof.
async fn call(
    server: &TestServer,
    channel: &str,
    agent: &Agent,
    token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> TestResponse {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let proof = agent
        .dpop
        .proof(method.as_str(), &url(channel, route), Some(token));
    let auth = format!("DPoP {token}");
    let uri = match query {
        "" => format!("/v1/channels/{channel}/poppy{route}"),
        query => format!("/v1/channels/{channel}/poppy{route}?{query}"),
    };
    server
        .request_raw(
            method,
            &uri,
            vec![
                ("host", HOST),
                ("content-type", "application/json"),
                ("authorization", auth.as_str()),
                ("dpop", proof.as_str()),
            ],
            body.map(|body| body.to_string().into_bytes())
                .unwrap_or_default(),
        )
        .await
}

fn message(id: &str, text: &str) -> Value {
    json!({ "message": { "id": id, "sender": "agent", "text": text,
                         "context": { "locale": "en-US", "user_available": true } } })
}

fn texts(events: &Value) -> Vec<(String, String)> {
    events["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["type"] == "message")
        .map(|event| {
            (
                event["message"]["role"].as_str().unwrap().to_string(),
                event["message"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect()
}

#[tokio::test]
async fn discovery_names_the_company_and_its_issuer() {
    let (server, _runner) = server().await;
    let channel = channel(&server).await;

    let document: Value = get(&server, &format!("/v1/channels/{channel}/poppy/poppy.json"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let issuer = url(&channel, "");
    assert_eq!(document["protocol_version"], "0.1");
    assert_eq!(
        document["organization"],
        json!({ "name": "Example Co", "domain": "example.com" })
    );
    assert_eq!(document["auth"]["issuer"], issuer);
    assert_eq!(
        document["agent"]["protocols"],
        json!([{ "type": "poppy", "endpoint": url(&channel, "/conversations") }])
    );
    let uk: Value = get(
        &server,
        &format!("/v1/channels/{channel}/poppy/poppy.json?domain=www.example.co.uk"),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(uk["organization"]["domain"], "example.co.uk");
    get(
        &server,
        &format!("/v1/channels/{channel}/poppy/poppy.json?domain=evil.example"),
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);

    // RFC 8414 metadata, appended to the issuer and at the RFC 8414 path.
    for uri in [
        format!("/v1/channels/{channel}/poppy/.well-known/oauth-authorization-server"),
        format!("/.well-known/oauth-authorization-server/api/v1/channels/{channel}/poppy"),
    ] {
        let metadata: Value = get(&server, &uri)
            .await
            .assert_status(StatusCode::OK)
            .json();
        assert_eq!(metadata["issuer"], issuer, "{uri}");
        assert_eq!(metadata["token_endpoint"], url(&channel, "/oauth/token"));
        assert_eq!(
            metadata["revocation_endpoint"],
            url(&channel, "/oauth/revoke")
        );
        assert_eq!(
            metadata["poppy_domains"],
            json!(["example.com", "example.co.uk"])
        );
    }

    get(&server, "/v1/channels/agc_missing/poppy/poppy.json")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn channel_config_is_validated() {
    let (server, _runner) = server().await;
    for config in [
        json!({ "domains": [] }),
        json!({ "domains": ["https://example.com"] }),
        json!({ "domains": ["example.com"], "allowed_agents": ["http://pa.example/a.json"] }),
    ] {
        create_channel(&server, config.clone())
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn sessions_start_renew_and_refuse_misuse() {
    let (server, _runner) = server().await;
    let channel = channel(&server).await;
    let agent = Agent::publish(&server).await;

    let (_token, session_id) = start_session(&server, &channel, &agent, "usr_1").await;
    // Renewing keeps the Session.
    let mut renew = session_grant(&agent, &channel, "usr_1");
    renew.push(("session_id", session_id.clone()));
    let renewed: Value = token_request(&server, &channel, &agent, &renew, true)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(renewed["session_id"], session_id);
    // Another User cannot take it over.
    let mut stolen = session_grant(&agent, &channel, "usr_2");
    stolen.push(("session_id", session_id.clone()));
    let body: Value = token_request(&server, &channel, &agent, &stolen, true)
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert_eq!(body["error"], "invalid_session");

    // A replayed assertion, a wrong audience and a missing proof.
    let grant = session_grant(&agent, &channel, "usr_1");
    token_request(&server, &channel, &agent, &grant, true)
        .await
        .assert_status(StatusCode::OK);
    let body: Value = token_request(&server, &channel, &agent, &grant, true)
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert_eq!(body["error"], "invalid_grant");
    let wrong_aud = vec![
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:jwt-bearer".to_string(),
        ),
        (
            "assertion",
            agent.signed("usr_1", "https://other.example/token"),
        ),
    ];
    let body: Value = token_request(&server, &channel, &agent, &wrong_aud, true)
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert_eq!(body["error"], "invalid_grant");
    let body: Value = token_request(
        &server,
        &channel,
        &agent,
        &session_grant(&agent, &channel, "usr_1"),
        false,
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST)
    .json();
    assert_eq!(body["error"], "invalid_dpop_proof");

    // An agent whose keys are not the published ones.
    let impostor = Agent {
        signing: Key::generate(),
        dpop: Key::generate(),
    };
    let body: Value = token_request(
        &server,
        &channel,
        &impostor,
        &session_grant(&impostor, &channel, "usr_1"),
        true,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED)
    .json();
    assert_eq!(body["error"], "invalid_client");
}

#[tokio::test]
async fn blocked_agents_cannot_start_sessions() {
    let (server, _runner) = server().await;
    let created = create_channel(
        &server,
        json!({ "domains": ["example.com"], "blocked_agents": [CLIENT_ID] }),
    )
    .await
    .assert_status(StatusCode::CREATED);
    let channel = created.json::<Value>()["id"].as_str().unwrap().to_string();
    let agent = Agent::publish(&server).await;
    let body: Value = token_request(
        &server,
        &channel,
        &agent,
        &session_grant(&agent, &channel, "usr_1"),
        true,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED)
    .json();
    assert_eq!(body["error"], "invalid_client");
}

#[tokio::test]
async fn conversation_round_trip_with_retries_and_close() {
    let (server, runner): (TestServer, Arc<ReplyingRunner>) = server().await;
    let channel = channel(&server).await;
    let agent = Agent::publish(&server).await;
    let (token, _) = start_session(&server, &channel, &agent, "usr_1").await;

    // Start, and wait for the reply in the same request.
    let started: Value = call(
        &server,
        &channel,
        &agent,
        &token,
        Method::POST,
        "/conversations?wait=10",
        Some(message("msg_first", "Is my order late?")),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let conversation = started["conversation_id"].as_str().unwrap().to_string();
    assert!(conversation.starts_with("cnv_"), "{started}");
    assert_eq!(started["responder"], "agent");
    assert_eq!(
        texts(&started),
        vec![
            ("user".into(), "Is my order late?".into()),
            ("company".into(), "Reply 1".into()),
        ]
    );
    let cursor = started["cursor"].as_str().unwrap().to_string();

    // A retried first message returns the same conversation, without a turn.
    let retried: Value = call(
        &server,
        &channel,
        &agent,
        &token,
        Method::POST,
        "/conversations",
        Some(message("msg_first", "Is my order late?")),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    assert_eq!(retried["conversation_id"], conversation);
    assert_eq!(runner.turns(), 1);
    let conflict: Value = call(
        &server,
        &channel,
        &agent,
        &token,
        Method::POST,
        "/conversations",
        Some(message("msg_first", "Something else")),
    )
    .await
    .assert_status(StatusCode::CONFLICT)
    .json();
    assert_eq!(conflict["error"], "message_id_conflict");

    // A follow-up, read back from the cursor.
    call(
        &server,
        &channel,
        &agent,
        &token,
        Method::POST,
        &format!("/conversations/{conversation}/messages"),
        Some(message("msg_second", "Can I exchange it?")),
    )
    .await
    .assert_status(StatusCode::ACCEPTED);
    let read: Value = call(
        &server,
        &channel,
        &agent,
        &token,
        Method::GET,
        &format!("/conversations/{conversation}/events?cursor={cursor}&wait=10"),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let mut seen = texts(&read);
    if seen.len() < 2 {
        let more: Value = call(
            &server,
            &channel,
            &agent,
            &token,
            Method::GET,
            &format!(
                "/conversations/{conversation}/events?cursor={}&wait=10",
                read["cursor"].as_str().unwrap()
            ),
            None,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
        seen.extend(texts(&more));
    }
    assert_eq!(
        seen,
        vec![
            ("user".into(), "Can I exchange it?".into()),
            ("company".into(), "Reply 2".into()),
        ]
    );
    assert_eq!(read["has_more"], false);

    // Another User of the same agent cannot see it.
    let (other, _) = start_session(&server, &channel, &agent, "usr_2").await;
    let body: Value = call(
        &server,
        &channel,
        &agent,
        &other,
        Method::GET,
        &format!("/conversations/{conversation}/events"),
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND)
    .json();
    assert_eq!(body["error"], "conversation_not_found");

    // Close: a final state event, and no more messages.
    let closed: Value = call(
        &server,
        &channel,
        &agent,
        &token,
        Method::POST,
        &format!("/conversations/{conversation}/close"),
        Some(json!({})),
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(closed["status"], "closed");
    let all: Value = call(
        &server,
        &channel,
        &agent,
        &token,
        Method::GET,
        &format!("/conversations/{conversation}/events"),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(all["status"], "closed");
    let last = all["events"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["type"], "state");
    assert_eq!(last["status"], "closed");
    let body: Value = call(
        &server,
        &channel,
        &agent,
        &token,
        Method::POST,
        &format!("/conversations/{conversation}/messages"),
        Some(message("msg_third", "One more thing")),
    )
    .await
    .assert_status(StatusCode::CONFLICT)
    .json();
    assert_eq!(body["error"], "conversation_closed");
}

#[tokio::test]
async fn session_tokens_need_their_own_dpop_key() {
    let (server, _runner) = server().await;
    let channel = channel(&server).await;
    let agent = Agent::publish(&server).await;
    let (token, _) = start_session(&server, &channel, &agent, "usr_1").await;

    // The same token with a proof from another key.
    let thief = Agent {
        signing: Key::generate(),
        dpop: Key::generate(),
    };
    let response = call(
        &server,
        &channel,
        &thief,
        &token,
        Method::POST,
        "/conversations",
        Some(message("msg_x", "hi")),
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
    assert!(
        response.headers()["www-authenticate"]
            .to_str()
            .unwrap()
            .starts_with("DPoP ")
    );
    // A Bearer token is refused outside MCP APIs.
    let bearer = format!("Bearer {token}");
    server
        .request_raw(
            Method::POST,
            &format!("/v1/channels/{channel}/poppy/conversations"),
            vec![
                ("host", HOST),
                ("content-type", "application/json"),
                ("authorization", bearer.as_str()),
            ],
            message("msg_x", "hi").to_string().into_bytes(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    // A token of one channel does not work at another.
    let other = self::channel(&server).await;
    call(
        &server,
        &other,
        &agent,
        &token,
        Method::POST,
        "/conversations",
        Some(message("msg_x", "hi")),
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn handoff_is_answered_by_the_agent() {
    let (server, runner) = server().await;
    let channel = channel(&server).await;
    let agent = Agent::publish(&server).await;
    let (token, _) = start_session(&server, &channel, &agent, "usr_1").await;
    let started: Value = call(
        &server,
        &channel,
        &agent,
        &token,
        Method::POST,
        "/conversations?wait=10",
        Some(message("msg_1", "I need a person")),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let conversation = started["conversation_id"].as_str().unwrap();
    let cursor = started["cursor"].as_str().unwrap();
    let handoff: Value = call(
        &server,
        &channel,
        &agent,
        &token,
        Method::POST,
        &format!("/conversations/{conversation}/handoff"),
        Some(json!({})),
    )
    .await
    .assert_status(StatusCode::ACCEPTED)
    .json();
    assert_eq!(handoff["responder"], "agent");
    // Read until the agent's answer arrives: the first read may end on the
    // first turn's idle state.
    let mut cursor = cursor.to_string();
    let mut seen = Vec::new();
    for _ in 0..3 {
        let read: Value = call(
            &server,
            &channel,
            &agent,
            &token,
            Method::GET,
            &format!("/conversations/{conversation}/events?cursor={cursor}&wait=10"),
            None,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
        seen.extend(texts(&read));
        cursor = read["cursor"].as_str().unwrap().to_string();
        if !seen.is_empty() {
            break;
        }
    }
    // The note to the agent stays hidden; its answer is a company message.
    assert_eq!(seen, vec![("company".into(), "Reply 2".into())]);
    assert_eq!(runner.turns(), 2);
}
