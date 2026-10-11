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

/// pi-poppy's `CompanyServer`, run by `scripts/poppy-interop/company.ts`,
/// with an HTTPS client that trusts its CA and finds it on loopback.
struct PiPoppyCompany {
    child: tokio::process::Child,
    port: u16,
    client: reqwest::Client,
}

impl PiPoppyCompany {
    async fn start(pi_poppy: &str, agent_server: &str) -> Self {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let driver = format!(
            "{}/../../scripts/poppy-interop/company.ts",
            env!("CARGO_MANIFEST_DIR")
        );
        let mut child = tokio::process::Command::new("node")
            .args(["--experimental-strip-types", "--no-warnings", &driver])
            .env("PI_POPPY_DIR", pi_poppy)
            .env("AGENT_SERVER", agent_server)
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
        Self {
            child,
            port,
            client,
        }
    }

    fn domain(&self) -> String {
        format!("shop.poppy.localhost:{}", self.port)
    }

    fn egress(&self) -> Arc<dyn EgressService> {
        Arc::new(ToPiPoppy {
            client: self.client.clone(),
        })
    }

    async fn stop(mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait().await;
    }
}

#[tokio::test]
async fn poppy_agent_talks_to_pi_poppy_company() {
    let Ok(pi_poppy) = std::env::var("PI_POPPY_DIR") else {
        return;
    };
    everruns_server::domains::poppy_agent::configure_public_base_url(AGENT_BASE);
    let (server, base_url, _runner) = serving().await;
    let pi = PiPoppyCompany::start(&pi_poppy, &base_url).await;
    let ctx = ctx(&server, pi.egress());
    let session = personal_agent_session(&server).await;
    let company = pi.domain();

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

    pi.stop().await;
}

/// Hands the worker's `worker_poppy_call` straight to the command, as the
/// server's in-process worker does.
struct DispatchCommands {
    ctx: Ctx,
}

#[async_trait::async_trait]
impl everruns_worker::internal_commands::InternalCommandTransport for DispatchCommands {
    async fn execute_internal_command(
        &self,
        name: &str,
        params: Value,
    ) -> everruns_contracts::error::Result<
        Result<Value, everruns_internal_protocol::proto::CommandError>,
    > {
        use everruns_internal_protocol::proto::{CommandError, command_error::Kind};
        use everruns_server::domains::common::CommandErrorKind;

        Ok(match dispatch(name, params, &self.ctx).await {
            Ok(json) => Ok(serde_json::from_str(&json).unwrap()),
            Err(error) => Err(CommandError {
                kind: match error.kind {
                    CommandErrorKind::BadRequest(_) => Kind::BadRequest,
                    _ => Kind::Internal,
                } as i32,
                message: error.to_string(),
            }),
        })
    }
}

/// A real model as the personal agent: a gpt-6.1-sol turn with the
/// `poppy_client` tools, its calls running through the worker's invoker and
/// the server's command, against pi-poppy's company. Runs when both
/// `PI_POPPY_DIR` and `OPENAI_API_KEY` are set.
#[tokio::test]
async fn real_model_asks_pi_poppy_company() {
    use everruns_capabilities::capabilities::{PoppyClientCapability, PoppyInvokerExt};
    use everruns_contracts::driver_registry::DriverRegistry;
    use everruns_contracts::model_spec::ModelSpec;
    use everruns_core::CapabilityRegistry;
    use everruns_core::host::HostComposition;
    use everruns_worker::internal_commands::CommandPoppyInvoker;

    let (Ok(pi_poppy), Ok(_)) = (
        std::env::var("PI_POPPY_DIR"),
        std::env::var("OPENAI_API_KEY"),
    ) else {
        return;
    };
    everruns_server::domains::poppy_agent::configure_public_base_url(AGENT_BASE);
    let (server, base_url, _runner) = serving().await;
    let pi = PiPoppyCompany::start(&pi_poppy, &base_url).await;
    let commands = DispatchCommands {
        ctx: ctx(&server, pi.egress()),
    };
    let server_session = personal_agent_session(&server).await.parse().unwrap();
    let invoker: Arc<dyn everruns_capabilities::capabilities::PoppyInvoker> =
        Arc::new(CommandPoppyInvoker::new(commands, server_session));

    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(PoppyClientCapability);
    let runtime = everruns::batteries::runtime_builder()
        .host_composition(HostComposition::new(capabilities, DriverRegistry::new()))
        .provider(everruns::OpenAI::from_env().unwrap().into())
        .default_model(ModelSpec::on("openai", "gpt-6.1-sol"))
        .with_tool_context_extensions_factory(Arc::new(move |_, _| {
            let mut extensions = everruns_core::tool_context::ToolContextExtensions::default();
            extensions.insert(Arc::new(PoppyInvokerExt(invoker.clone())));
            extensions
        }))
        .single_session(|s| {
            s.harness("personal", "You are a personal assistant.")
                .with_capability("poppy_client")
                .agent("personal-agent", "Act for the person you work for.")
                .agent_max_iterations(8)
        })
        .build()
        .await
        .unwrap();
    let session = runtime.default_session_id().unwrap();
    let company = pi.domain();
    let result = runtime
        .run_text_turn(
            session,
            format!(
                "Ask {company} whether my order ord_7Hk2 has shipped, and tell me what they said."
            ),
        )
        .await
        .unwrap();

    let calls: Vec<(String, Value)> = runtime
        .messages(session)
        .await
        .unwrap()
        .iter()
        .flat_map(|message| message.tool_calls())
        .map(|call| (call.name.clone(), call.arguments.clone()))
        .collect();
    eprintln!("tool calls: {calls:?}\nanswer: {}", result.response);
    assert!(result.success, "{}", result.response);
    assert!(
        calls
            .iter()
            .any(|(name, arguments)| name == "poppy_send" && arguments["company"] == company),
        "the model should message the company: {calls:?}"
    );
    // pi-poppy answers order questions from a signed-out Session with "I need
    // you to sign in before I can see that order."; the model relays it.
    assert!(
        result.response.to_lowercase().contains("sign in")
            || result.response.to_lowercase().contains("sign-in"),
        "{}",
        result.response
    );

    pi.stop().await;
}
