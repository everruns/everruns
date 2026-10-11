//! Everruns agents as Poppy personal agents: `worker_poppy_call` (the command
//! behind the `poppy_client` tools) against our own Poppy channel and, when
//! `PI_POPPY_DIR` is set, against pi-poppy's independent company. Requests
//! leave through a stand-in egress service that knows where the test
//! companies live; everything else is the real path.

use crate::channel_a2a_pact_test::serving;
use crate::channel_poppy_test::create_channel;
use crate::test_harness::TestServer;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use everruns_core::{
    Caller, DefaultPermissionResolver, EgressError, EgressRequest, EgressResponse, EgressResult,
    EgressService, EgressStreamResponse,
};
use everruns_server::domains::common::{Ctx, dispatch};
use serde_json::{Value, json};

/// The deployment's public base URL in these tests. The client metadata
/// document is served under it.
const AGENT_BASE: &str = "https://agent.poppy.localhost/api";
const CHANNEL_DOMAIN: &str = "poppy.localhost";

fn client_id() -> String {
    everruns_server::domains::poppy_agent::client_id(AGENT_BASE)
}

/// Sends `https://{host}/...` to a plain-HTTP test server, as a TLS front
/// would: `Host` kept, `X-Forwarded-Proto: https`. A company's
/// `/.well-known/poppy.json` is answered here with the redirect the company's
/// own website would serve.
struct ToTestServer {
    base_url: String,
    channel_id: String,
}

fn headers(response: &reqwest::Response) -> BTreeMap<String, String> {
    response
        .headers()
        .iter()
        .filter_map(|(name, value)| Some((name.to_string(), value.to_str().ok()?.to_string())))
        .collect()
}

async fn forward(
    client: &reqwest::Client,
    url: &str,
    request: EgressRequest,
    host: Option<&str>,
) -> EgressResult<EgressResponse> {
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|error| EgressError::InvalidRequest(error.to_string()))?;
    let mut outgoing = client.request(method, url).body(request.body);
    for (name, value) in &request.headers {
        outgoing = outgoing.header(name, value);
    }
    if let Some(host) = host {
        outgoing = outgoing
            .header("host", host)
            .header("x-forwarded-proto", "https");
    }
    let response = outgoing
        .send()
        .await
        .map_err(|error| EgressError::Transport(error.to_string()))?;
    let status = response.status().as_u16();
    let headers = headers(&response);
    let body = response
        .bytes()
        .await
        .map_err(|error| EgressError::Transport(error.to_string()))?
        .to_vec();
    Ok(EgressResponse {
        status,
        headers,
        body,
    })
}

fn no_redirects() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[async_trait::async_trait]
impl EgressService for ToTestServer {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let url = url::Url::parse(&request.url).unwrap();
        let host = url.host_str().unwrap().to_string();
        if url.path() == "/.well-known/poppy.json" {
            let location = format!(
                "/api/v1/channels/{}/poppy/poppy.json?domain={CHANNEL_DOMAIN}",
                self.channel_id
            );
            return Ok(EgressResponse {
                status: 302,
                headers: BTreeMap::from([("location".into(), location)]),
                body: Vec::new(),
            });
        }
        let target = format!(
            "{}{}{}",
            self.base_url,
            url.path(),
            url.query()
                .map(|query| format!("?{query}"))
                .unwrap_or_default()
        );
        forward(&no_redirects(), &target, request, Some(&host)).await
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::Transport("no streaming".into()))
    }
}

/// A personal-agent session: an agent of its own and a session on it.
async fn personal_agent_session(server: &TestServer) -> String {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("pa-{}", uuid::Uuid::new_v4().simple()),
                "display_name": "Personal agent",
                "description": "Talks to companies for its person.",
                "system_prompt": "You are a brief test agent."
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session: Value = server
        .post("/v1/sessions", json!({ "agent_id": agent["id"] }))
        .await
        .assert_success()
        .json();
    session["id"].as_str().unwrap().to_string()
}

fn ctx(server: &TestServer, egress: Arc<dyn EgressService>) -> Ctx {
    let capabilities = Arc::new(everruns_server::services::CapabilityService::new(
        server.db.clone(),
        server.encryption.clone(),
    ));
    Ctx::new(
        Caller::internal(everruns_core::DEFAULT_ORG_ID),
        server.db.clone(),
        capabilities,
        server.encryption.clone(),
        Arc::new(DefaultPermissionResolver),
    )
    .with_egress_service(egress)
}

/// Run one `poppy_client` call; `Err` is the refusal the model would see.
async fn call(ctx: &Ctx, session_id: &str, call: Value) -> Result<Value, String> {
    let params = json!({ "session_id": session_id, "call": call });
    match dispatch("worker_poppy_call", params, ctx).await {
        Ok(json) => Ok(serde_json::from_str(&json).unwrap()),
        Err(error) => Err(error.to_string()),
    }
}

fn texts(answer: &Value) -> Vec<String> {
    answer["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["text"].as_str().unwrap_or_default().to_string())
        .collect()
}

#[tokio::test]
async fn poppy_agent_talks_to_our_channel() {
    everruns_server::domains::poppy_agent::configure_public_base_url(AGENT_BASE);
    let (server, base_url, _runner) = serving().await;
    let channel_id = create_channel(
        &server,
        json!({ "organization_name": "Interop Co", "domains": [CHANNEL_DOMAIN] }),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json::<Value>()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // What a company would fetch from our client_id: the server's own
    // documents, served by its identity routes.
    let metadata: Value = server
        .get("/v1/poppy/agent.json")
        .await
        .assert_success()
        .json();
    assert_eq!(metadata["client_id"], client_id());
    let jwks: Value = server
        .get("/v1/poppy/jwks.json")
        .await
        .assert_success()
        .json();
    assert_eq!(jwks["keys"][0]["kty"], "EC");
    assert!(jwks["keys"][0].get("d").is_none(), "only the public key");
    server
        .poppy_documents
        .prime_public_document(&client_id(), metadata.clone())
        .await;
    server
        .poppy_documents
        .prime_public_document(metadata["jwks_uri"].as_str().unwrap(), jwks)
        .await;

    let egress = Arc::new(ToTestServer {
        base_url: base_url.clone(),
        channel_id,
    });
    let ctx = ctx(&server, egress);
    let session = personal_agent_session(&server).await;

    let discovered = call(
        &ctx,
        &session,
        json!({ "action": "discover", "company": CHANNEL_DOMAIN }),
    )
    .await
    .unwrap();
    assert_eq!(discovered["name"], "Interop Co");
    assert_eq!(discovered["conversations"], true);

    let send = json!({
        "action": "send", "company": CHANNEL_DOMAIN, "text": "Is my order late?",
        "message_id": "msg_tool_1", "wait_seconds": 10,
    });
    let first = call(&ctx, &session, send.clone()).await.unwrap();
    let conversation = first["conversation_id"].as_str().unwrap().to_string();
    assert_eq!(texts(&first), ["Reply 1"], "{first}");

    // The same tool call again (a retried turn) is the same message.
    let retried = call(&ctx, &session, send).await.unwrap();
    assert_eq!(retried["conversation_id"], conversation.as_str());

    let second = call(
        &ctx,
        &session,
        json!({
            "action": "send", "company": CHANNEL_DOMAIN, "text": "Thanks, and the refund?",
            "message_id": "msg_tool_2", "wait_seconds": 10,
        }),
    )
    .await
    .unwrap();
    assert_eq!(second["conversation_id"], conversation.as_str());
    assert_eq!(texts(&second), ["Reply 2"], "{second}");

    // Nothing new since the last answer.
    let read = call(
        &ctx,
        &session,
        json!({ "action": "read", "company": CHANNEL_DOMAIN }),
    )
    .await
    .unwrap();
    assert!(texts(&read).is_empty(), "{read}");

    let handoff = call(
        &ctx,
        &session,
        json!({ "action": "handoff", "company": CHANNEL_DOMAIN }),
    )
    .await
    .unwrap();
    // No person can join at our channel yet; it stays with the agent.
    assert_eq!(handoff["responder"], "agent");
    let closed = call(
        &ctx,
        &session,
        json!({ "action": "close", "company": CHANNEL_DOMAIN }),
    )
    .await
    .unwrap();
    assert_eq!(closed["status"], "closed");

    // A closed conversation refuses more messages, and a new one starts.
    let refused = call(
        &ctx,
        &session,
        json!({
            "action": "send", "company": CHANNEL_DOMAIN, "text": "One more thing",
            "message_id": "msg_tool_3",
        }),
    )
    .await
    .unwrap_err();
    assert!(
        refused.contains("poppy.localhost refused the message"),
        "{refused}"
    );
    let fresh = call(
        &ctx,
        &session,
        json!({
            "action": "send", "company": CHANNEL_DOMAIN, "text": "One more thing",
            "message_id": "msg_tool_4", "new_conversation": true, "wait_seconds": 10,
        }),
    )
    .await
    .unwrap();
    assert_ne!(fresh["conversation_id"], conversation.as_str());

    // A company that does not speak Poppy is a refusal, not a fault.
    let missing = call(
        &ctx,
        &session,
        json!({ "action": "discover", "company": "intranet" }),
    )
    .await
    .unwrap_err();
    assert!(missing.contains("not a domain"), "{missing}");
}

/// Sends to pi-poppy's company over HTTPS, trusting its test CA and
/// resolving its names to loopback.
struct ToPiPoppy {
    client: reqwest::Client,
}

#[async_trait::async_trait]
impl EgressService for ToPiPoppy {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let url = request.url.clone();
        forward(&self.client, &url, request, None).await
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        Err(EgressError::Transport("no streaming".into()))
    }
}

#[tokio::test]
async fn poppy_agent_talks_to_pi_poppy_company() {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let Ok(pi_poppy) = std::env::var("PI_POPPY_DIR") else {
        return;
    };
    everruns_server::domains::poppy_agent::configure_public_base_url(AGENT_BASE);
    let driver = format!(
        "{}/../../scripts/poppy-interop/company.ts",
        env!("CARGO_MANIFEST_DIR")
    );
    let (server, base_url, _runner) = serving().await;
    let mut child = tokio::process::Command::new("node")
        .args(["--experimental-strip-types", "--no-warnings", &driver])
        .env("PI_POPPY_DIR", &pi_poppy)
        .env("AGENT_SERVER", &base_url)
        .env("COMPANY_DOMAIN", "shop.poppy.localhost")
        .env("AGENT_HOST", "agent.poppy.localhost")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("run node");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let ready: Value = loop {
        let line = lines
            .next_line()
            .await
            .unwrap()
            .expect("the company driver exited before it was ready");
        if let Some(json) = line.strip_prefix("COMPANY ") {
            break serde_json::from_str(json).unwrap();
        }
    };
    let port = ready["port"].as_u64().unwrap() as u16;
    let ca = std::fs::read(ready["ca_file"].as_str().unwrap()).unwrap();
    let loopback = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .add_root_certificate(reqwest::Certificate::from_pem(&ca).unwrap())
        .resolve("shop.poppy.localhost", loopback)
        .build()
        .unwrap();
    let ctx = ctx(&server, Arc::new(ToPiPoppy { client }));
    let session = personal_agent_session(&server).await;
    let company = format!("shop.poppy.localhost:{port}");

    let discovered = call(
        &ctx,
        &session,
        json!({ "action": "discover", "company": company }),
    )
    .await
    .unwrap();
    assert_eq!(discovered["name"], "Example Company", "{discovered}");

    let send = json!({
        "action": "send", "company": company, "text": "Do you ship to Kyiv?",
        "message_id": "msg_pi_1", "wait_seconds": 5,
    });
    let first = call(&ctx, &session, send.clone()).await.unwrap();
    assert_eq!(texts(&first), ["I can help with that."], "{first}");
    let conversation = first["conversation_id"].as_str().unwrap().to_string();
    let retried = call(&ctx, &session, send).await.unwrap();
    assert_eq!(retried["conversation_id"], conversation.as_str());

    let second = call(
        &ctx,
        &session,
        json!({
            "action": "send", "company": company, "text": "And how long does it take?",
            "message_id": "msg_pi_2", "wait_seconds": 5,
        }),
    )
    .await
    .unwrap();
    assert_eq!(second["conversation_id"], conversation.as_str());
    assert_eq!(texts(&second), ["I can help with that."], "{second}");

    let read = call(
        &ctx,
        &session,
        json!({ "action": "read", "company": company }),
    )
    .await
    .unwrap();
    assert!(texts(&read).is_empty(), "{read}");
    call(
        &ctx,
        &session,
        json!({ "action": "handoff", "company": company }),
    )
    .await
    .unwrap();
    let closed = call(
        &ctx,
        &session,
        json!({ "action": "close", "company": company }),
    )
    .await
    .unwrap();
    assert_eq!(closed["status"], "closed", "{closed}");

    drop(child.stdin.take());
    let _ = child.wait().await;
}
