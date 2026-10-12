//! PACT Delegated `message:send` (PACT 1.0 §5.5, §5.6): a personal agent
//! sends the user's delegation token with its own JWT. The turn runs as that
//! company user and reaches the company's API through the catalog MCP server
//! the config names; a call needing an ungranted scope answers with a step-up
//! task; replies carry a signed receipt.
//!
//! `SkylineRunner` stands in for the worker and the model: it reads the
//! user's token the way the MCP client does (the real connection resolver,
//! `actsAs: user`), "calls" the company tools, and records their tool events.
//! Against PACT's reference company it calls the real account API instead.

use crate::channel_a2a_pact_delegation_test::{Fixture, claims, delegated_config};
use crate::channel_a2a_pact_test::{ISSUER, PersonalAgentKey, assert_no_a2a_body, send_body};
use crate::test_harness::{TestResponse, TestServer};

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use base64::Engine as _;
use everruns_contracts::typed_id::SessionId;
use everruns_core::McpServerActsAs;
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::events::{
    EventContext, EventRequest, INPUT_MESSAGE, InputMessageData, OutputMessageCompletedData,
    ToolCompletedData, ToolStartedData,
};
use everruns_core::message::RuntimeMessage;
use everruns_server::storage::{CreateMcpServerRow, DbConnectionResolver};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const UPCOMING: &str = "mcp_skyline__list_upcoming_trips";
const PAST: &str = "mcp_skyline__list_past_trips";
const REBOOK: &str = "mcp_skyline__rebook_trip";

/// The Skyline scopes PACT's reference company offers, each unlocking the
/// matching tool of the `skyline` MCP server.
pub(crate) fn skyline_delegation(base: Value) -> Value {
    let mut delegation = base;
    delegation["mcp_server"] = json!("skyline");
    delegation["scopes"] = json!([
        { "id": "flights:upcoming:read", "description": "View your upcoming flights", "tools": [UPCOMING] },
        { "id": "flights:history:read", "description": "View your past flights", "tools": [PAST] },
        { "id": "flights:rebook", "description": "Rebook your flights", "tools": [REBOOK] },
    ]);
    delegation
}

/// Stands in for the worker and the model of a Skyline account agent.
#[derive(Default)]
pub(crate) struct SkylineRunner {
    wiring: OnceLock<Wiring>,
    /// PACT's reference company. Without it, a tool succeeds when the token
    /// carries its scope, which is what the company's API checks.
    brand_url: OnceLock<String>,
}

/// What the runner needs of the server.
struct Wiring {
    db: Arc<everruns_server::storage::StorageBackend>,
    encryption: everruns_server::storage::EncryptionService,
    egress: Arc<dyn everruns_core::EgressService>,
    events: Arc<everruns_server::services::EventService>,
    mcp_server_id: Uuid,
}

impl SkylineRunner {
    /// Wire the runner to `server` and create the org's `skyline` MCP server.
    pub(crate) async fn wire(&self, server: &TestServer) {
        let mcp_server_id = Uuid::now_v7();
        server
            .db
            .create_mcp_server_with_id(
                everruns_core::DEFAULT_ORG_ID,
                mcp_server_id,
                CreateMcpServerRow {
                    name: "skyline".into(),
                    description: None,
                    url: "https://skyline.example/mcp".into(),
                    transport_type: "http".into(),
                    api_key_encrypted: None,
                    headers: None,
                    settings: Some(json!({ "auth_mode": "oauth" })),
                },
            )
            .await
            .unwrap();
        self.wiring
            .set(Wiring {
                db: server.db.clone(),
                encryption: server.encryption.as_ref().unwrap().as_ref().clone(),
                egress: server.mcp_servers.clone(),
                events: server.event_service.clone(),
                mcp_server_id,
            })
            .ok();
    }

    pub(crate) fn use_brand(&self, url: String) {
        self.brand_url.set(url).ok();
    }

    async fn run(&self, session_id: SessionId, turn_id: Value) {
        let wiring = self.wiring.get().expect("runner wired");
        let inputs = wiring
            .db
            .list_events(
                session_id,
                None,
                None,
                &[INPUT_MESSAGE.to_string()],
                &[],
                None,
                Some(1),
            )
            .await
            .unwrap();
        let input: InputMessageData =
            serde_json::from_value(inputs.last().unwrap().data.clone()).unwrap();
        let text = input.message.text().unwrap_or_default().to_lowercase();
        // The token the MCP client would send, resolved exactly as it is.
        let resolver = DbConnectionResolver::new(
            wiring.db.as_ref().clone(),
            wiring.encryption.clone(),
            None,
            wiring.egress.clone(),
        )
        .bound_to_input_message(input.message.id.uuid());
        let token = resolver
            .get_mcp_connection_token(
                session_id,
                &format!("mcp_oauth_{}", wiring.mcp_server_id),
                McpServerActsAs::User,
            )
            .await
            .unwrap();

        let mut reply = Vec::new();
        if text.contains("rebook") || text.contains("flight") {
            let upcoming = self
                .call(
                    token.as_deref(),
                    "flights:upcoming:read",
                    "GET",
                    "/api/trips/upcoming",
                    None,
                )
                .await;
            self.tool_events(session_id, UPCOMING, json!({}), upcoming.is_some())
                .await;
            let trip = upcoming
                .as_ref()
                .and_then(|body| body["trips"].get(0).cloned());
            if text.contains("rebook") {
                let confirmation = trip
                    .as_ref()
                    .and_then(|trip| trip["confirmation"].as_str())
                    .unwrap_or("UNKNOWN")
                    .to_string();
                let args = json!({ "confirmation": confirmation, "flight": "SK 318" });
                let rebooked = self
                    .call(
                        token.as_deref(),
                        "flights:rebook",
                        "POST",
                        &format!("/api/trips/{confirmation}/rebook"),
                        Some(json!({ "flight": "SK 318" })),
                    )
                    .await;
                self.tool_events(session_id, REBOOK, args, rebooked.is_some())
                    .await;
                reply.push(match rebooked {
                    Some(_) => "Done: you are now on SK 318.".to_string(),
                    None => "I need permission to rebook your flights.".to_string(),
                });
            } else {
                reply.push(match trip {
                    Some(trip) => format!(
                        "{} on {} leaves {} at {}.",
                        trip["flight"].as_str().unwrap_or_default(),
                        trip["date"].as_str().unwrap_or_default(),
                        trip["from"].as_str().unwrap_or_default(),
                        trip["departs"].as_str().unwrap_or_default(),
                    ),
                    None => "I need permission to view your upcoming flights.".to_string(),
                });
            }
        } else {
            reply.push("Hello from Skyline.".to_string());
        }

        let events = &wiring.events;
        events
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                OutputMessageCompletedData::new(RuntimeMessage::assistant(reply.join(" "))),
            ))
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
    }

    /// One company API call with the user's token. `None` is a refusal.
    async fn call(
        &self,
        token: Option<&str>,
        scope: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Option<Value> {
        let token = token?;
        let Some(brand) = self.brand_url.get() else {
            let granted = claims(token)["scope"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            return granted.split(' ').any(|s| s == scope).then(|| {
                json!({ "trips": [{
                    "confirmation": "QX7K2M", "flight": "SK 482", "date": "2026-11-02",
                    "from": "SFO", "departs": "9:40 AM",
                }] })
            });
        };
        let client = reqwest::Client::new();
        let url = format!("{brand}{path}");
        let request = match method {
            "POST" => client.post(url).json(&body.unwrap_or_default()),
            _ => client.get(url),
        };
        let response = request.bearer_auth(token).send().await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json().await.ok()
    }

    async fn tool_events(&self, session_id: SessionId, tool: &str, args: Value, success: bool) {
        let events = &self.wiring.get().unwrap().events;
        let call_id = format!("call_{}", Uuid::now_v7().simple());
        let started: ToolStartedData = serde_json::from_value(json!({
            "tool_call": { "id": call_id, "name": tool, "arguments": args },
        }))
        .unwrap();
        events
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                started,
            ))
            .await
            .unwrap();
        let completed: ToolCompletedData = serde_json::from_value(json!({
            "tool_call_id": call_id,
            "tool_name": tool,
            "success": success,
            "status": if success { "success" } else { "error" },
        }))
        .unwrap();
        events
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                completed,
            ))
            .await
            .unwrap();
    }
}

/// The runner behind the shared `Arc` the server holds.
pub(crate) struct SharedRunner(pub(crate) Arc<SkylineRunner>);

#[async_trait]
impl everruns_core::host::TurnBackend for SharedRunner {
    async fn start_turn(
        &self,
        request: everruns_core::host::TurnRequest,
    ) -> everruns_contracts::error::Result<everruns_core::host::TurnTicket> {
        let runner = self.0.clone();
        let session_id = request.session_id;
        let turn_id = serde_json::to_value(request.turn_id).unwrap();
        tokio::spawn(async move { runner.run(session_id, turn_id).await });
        Ok(everruns_core::host::TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(&self, _session_id: SessionId) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

struct Setup {
    f: Fixture,
}

async fn setup() -> Setup {
    let runner = Arc::new(SkylineRunner::default());
    let server = TestServer::in_memory_with_runner(Arc::new(SharedRunner(runner.clone()))).await;
    runner.wire(&server).await;
    let f = Fixture::new(server, |pa, brand| {
        let mut config = delegated_config(pa, brand);
        config["delegation"] = skyline_delegation(config["delegation"].clone());
        config
    })
    .await;
    Setup { f }
}

impl Setup {
    /// Run the device flow: the company user `sub` approves `scopes`.
    async fn grant(&self, scopes: &[&str], sub: &str) -> Value {
        let f = &self.f;
        let started = f.start(&scopes.join(" ")).await;
        let assertion = f
            .assertion(
                started["user_code"].as_str().unwrap(),
                json!({ "sub": sub }),
            )
            .await;
        f.consent(&assertion, "allow", scopes)
            .await
            .assert_status(StatusCode::SEE_OTHER);
        f.poll(started["device_code"].as_str().unwrap())
            .await
            .assert_status(StatusCode::OK)
            .json()
    }

    async fn send(
        &self,
        text: &str,
        message_id: &str,
        context_id: Option<&str>,
        delegation: Option<&str>,
    ) -> TestResponse {
        let auth = format!("Bearer {}", self.f.pa_token());
        let delegation = delegation.map(|token| format!("Bearer {token}"));
        let mut headers = vec![
            ("content-type", "application/json"),
            ("authorization", auth.as_str()),
        ];
        if let Some(delegation) = delegation.as_deref() {
            headers.push(("x-a2a-user-delegation", delegation));
        }
        self.f
            .server
            .request_raw(
                Method::POST,
                &format!("/v1/channels/{}/a2a/pact/message:send", self.f.channel),
                headers,
                send_body(text, message_id, context_id).into_bytes(),
            )
            .await
    }

    /// Verify a receipt's JWS against the endpoint's published key and
    /// return its claims, checking they are what the JWS signs.
    async fn verify_receipt(&self, receipt: &Value) -> Value {
        let jwks: jsonwebtoken::jwk::JwkSet = self
            .f
            .server
            .get(&format!(
                "/v1/channels/{}/a2a/pact/oauth/jwks.json",
                self.f.channel
            ))
            .await
            .assert_status(StatusCode::OK)
            .json();
        let jws = receipt["jws"].as_str().unwrap();
        let header = jsonwebtoken::decode_header(jws).unwrap();
        assert_eq!(header.typ.as_deref(), Some("pact-receipt+jws"));
        let jwk = jwks.find(header.kid.as_deref().unwrap()).unwrap();
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::ES256);
        validation.validate_exp = false;
        validation.validate_aud = false;
        validation.required_spec_claims.clear();
        let payload = jsonwebtoken::decode::<Value>(
            jws,
            &jsonwebtoken::DecodingKey::from_jwk(jwk).unwrap(),
            &validation,
        )
        .unwrap()
        .claims;
        assert_eq!(payload, receipt["claims"]);
        payload
    }
}

fn step_up(response: TestResponse) -> Value {
    let body: Value = response.assert_status(StatusCode::OK).json();
    let task = &body["task"];
    assert_eq!(
        task["status"]["state"], "TASK_STATE_AUTH_REQUIRED",
        "{body}"
    );
    task.clone()
}

fn base64url_sha256(value: &Value) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(value.to_string()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_token_a_scoped_tool_asks_for_permission() {
    let s = setup().await;
    let task = step_up(s.send("my upcoming flight?", "m1", None, None).await);
    assert_eq!(
        task["metadata"]["pact.missingScopes"],
        json!(["flights:upcoming:read"])
    );
    let link = task["metadata"]["pact.verificationUriComplete"]
        .as_str()
        .unwrap();
    assert!(
        link.starts_with("https://brand.example/login?return_to="),
        "{link}"
    );
    assert!(
        task["status"]["message"]["parts"][0]["text"]
            .as_str()
            .unwrap()
            .contains("permission")
    );
    // A turn that touches no scoped tool is an ordinary reply.
    let context = task["contextId"].as_str().unwrap();
    let body: Value = s
        .send("hello", "m2", Some(context), None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["message"]["parts"][0]["text"], "Hello from Skyline.");
    assert!(body["message"]["metadata"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delegated_turn_acts_as_the_user_and_signs_a_receipt() {
    let s = setup().await;
    let token = s
        .grant(&["flights:upcoming:read", "flights:history:read"], "alex")
        .await;
    let access = token["access_token"].as_str().unwrap();

    let response = s
        .send("my upcoming flight?", "m1", None, Some(access))
        .await;
    let body: Value = response.assert_status(StatusCode::OK).json();
    let message = &body["message"];
    assert!(
        message["parts"][0]["text"]
            .as_str()
            .unwrap()
            .contains("SK 482"),
        "{body}"
    );
    let receipt = &message["metadata"]["pact.receipt"];
    let claims = s.verify_receipt(receipt).await;
    assert_eq!(claims["user"], "alex");
    assert_eq!(claims["pa"], ISSUER);
    assert_eq!(claims["grantId"], self::claims(access)["grant_id"]);
    assert!(
        claims["brand"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/v1/channels/{}/a2a/pact", s.f.channel))
    );
    assert_eq!(claims["scopesUsed"], json!(["flights:upcoming:read"]));
    assert_eq!(
        claims["actions"],
        json!([{ "tool": UPCOMING, "argsHash": base64url_sha256(&json!({})) }])
    );

    // A repeated messageId returns the stored reply, still with a receipt.
    let again: Value = s
        .send(
            "my upcoming flight?",
            "m1",
            message["contextId"].as_str(),
            Some(access),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(again["message"]["messageId"], message["messageId"]);
    assert!(again["message"]["metadata"]["pact.receipt"]["jws"].is_string());

    // Rebooking needs a scope the user did not grant: step-up for just it.
    let task = step_up(
        s.send(
            "please rebook me onto SK 318",
            "m2",
            message["contextId"].as_str(),
            Some(access),
        )
        .await,
    );
    assert_eq!(
        task["metadata"]["pact.missingScopes"],
        json!(["flights:rebook"])
    );
    assert_eq!(task["contextId"], message["contextId"]);

    // Without the token, the next turn no longer has the user's grant.
    let task = step_up(
        s.send(
            "my upcoming flight?",
            "m3",
            message["contextId"].as_str(),
            None,
        )
        .await,
    );
    assert_eq!(
        task["metadata"]["pact.missingScopes"],
        json!(["flights:upcoming:read"])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_context_stays_with_its_company_user() {
    let s = setup().await;
    let alex = s.grant(&["flights:upcoming:read"], "alex").await;
    let sam = s.grant(&["flights:upcoming:read"], "sam").await;
    let body: Value = s
        .send(
            "my upcoming flight?",
            "m1",
            None,
            alex["access_token"].as_str(),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let context = body["message"]["contextId"].as_str().unwrap();
    let refused: Value = s
        .send(
            "my upcoming flight?",
            "m2",
            Some(context),
            sam["access_token"].as_str(),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert!(refused.to_string().contains("another account"), "{refused}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_delegation_tokens_are_invalid_token() {
    let s = setup().await;
    let token = s.grant(&["flights:upcoming:read"], "alex").await;
    let access = token["access_token"].as_str().unwrap();
    let tampered = format!("{}AAAA", &access[..access.len() - 4]);
    // A token another endpoint signed, with the right shape.
    let other = PersonalAgentKey::generate().token_with(json!({
        "iss": claims(access)["iss"], "aud": claims(access)["aud"],
        "sub": "alex", "client_id": ISSUER, "scope": "flights:upcoming:read",
        "grant_id": claims(access)["grant_id"],
    }));
    for bad in [tampered.as_str(), other.as_str(), "", "not-a-jwt"] {
        let response = s
            .send("my upcoming flight?", "m1", None, Some(bad))
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
        let challenge = response
            .headers()
            .get("www-authenticate")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert_eq!(challenge, r#"Bearer realm="a2a", error="invalid_token""#);
        assert_no_a2a_body(&response);
    }
}

/// The RFC 7638 thumbprint PACT's reference company uses as its key's `kid`.
fn thumbprint(jwk: &Value) -> String {
    let canonical = format!(
        r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
        jwk["x"].as_str().unwrap(),
        jwk["y"].as_str().unwrap()
    );
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(canonical))
}

/// PACT's own Delegated conformance suite (`e2e/delegated.test.ts` in
/// github.com/openpactprotocol/openpactprotocol) against this server over a
/// real socket, with PACT's reference company (`reference/brand`) as the
/// company: its login signs users in, and its account API is what the
/// stand-in agent calls with the user's token. Runs only when `PACT_E2E_DIR`
/// names that suite's directory with its dependencies installed;
/// `scripts/pact-conformance.sh` sets it up.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pact_delegated_conformance_suite_passes() {
    let Ok(suite_dir) = std::env::var("PACT_E2E_DIR") else {
        return;
    };
    let checkout = std::path::Path::new(&suite_dir).join("..");
    let brand_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let brand_url = format!("http://127.0.0.1:{brand_port}");

    let runner = Arc::new(SkylineRunner::default());
    runner.use_brand(brand_url.clone());
    let (server, base_url) =
        TestServer::serving_in_memory_with_runner(Arc::new(SharedRunner(runner.clone()))).await;
    runner.wire(&server).await;
    let provider_url = format!("{base_url}/api/v1");
    let brand_for_config = brand_url.clone();
    let f = Fixture::new(server, move |pa, brand| {
        let mut config = delegated_config(pa, brand);
        let mut public = brand.jwk.clone();
        public["kid"] = json!(thumbprint(&brand.jwk));
        public["use"] = json!("sig");
        config["delegation"] = skyline_delegation(json!({
            "brand_name": "Skyline Airways",
            "login_url": format!("{brand_for_config}/login"),
            "login_issuer": brand_for_config,
            "login_jwks": { "keys": [public] },
            "connected_url": format!("{brand_for_config}/connected"),
        }));
        config
    })
    .await;
    let mut pa_private = f.pa.jwk.clone();
    pa_private["d"] = json!(f.pa.d);
    let brand_private = json!({
        "kty": "EC", "crv": "P-256",
        "x": f.brand.jwk["x"], "y": f.brand.jwk["y"], "d": f.brand.d,
    });

    let mut brand = tokio::process::Command::new(checkout.join("node_modules/.bin/tsx"))
        .arg("src/server.ts")
        .current_dir(checkout.join("reference/brand"))
        .env("PORT", brand_port.to_string())
        .env("BRAND_URL", &brand_url)
        .env("PROVIDER_URL", &provider_url)
        .env("CONSENT_ORIGIN", &provider_url)
        .env("BRAND_CUSTOMER_ID", &f.channel)
        .env("BRAND_PRIVATE_JWK", brand_private.to_string())
        .kill_on_drop(true)
        .spawn()
        .expect("start the reference company");
    let client = reqwest::Client::new();
    let mut ready = false;
    for _ in 0..150 {
        if client.get(&brand_url).send().await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert!(ready, "the reference company did not start");

    let output = tokio::process::Command::new("pnpm")
        .args([
            "exec",
            "vitest",
            "run",
            "--sequence.concurrent=false",
            "delegated.test.ts",
        ])
        .current_dir(&suite_dir)
        .env("PROVIDER_URL", &provider_url)
        .env("DELEGATED_CUSTOMER_ID", &f.channel)
        .env("PA_ISSUER", ISSUER)
        .env("PA_AUDIENCE", crate::channel_a2a_pact_test::AUDIENCE)
        .env("PA_PRIVATE_JWK", pa_private.to_string())
        .output()
        .await
        .expect("run pnpm");
    brand.kill().await.ok();
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{stdout}");
    eprintln!("{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "PACT delegated conformance suite failed"
    );
    // A skipped suite (no delegation on the card) must not pass silently.
    assert!(
        !stdout.contains("skipped"),
        "the delegated suite was skipped"
    );
}
