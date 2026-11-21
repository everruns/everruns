// End to end: the core MCP client, minting session tokens like a data plane,
// calls the server's injected Knowledge Base endpoint.

use super::knowledge_base::{KnowledgeBaseService, SessionCapabilityConfig};
use super::*;
use async_trait::async_trait;
use axum::body::Body;
use axum::http::Request;
use everruns_capabilities::{KnowledgeSearchHit, KnowledgeStore};
use everruns_contracts::error::Result as CoreResult;
use everruns_contracts::runtime::capabilities::injected::InjectedToolNames;
use everruns_contracts::typed_id::{OrgId, SessionId};
use everruns_core::McpServerAuthMode;
use everruns_core::mcp::session_token::{
    LocalSessionTokenMinter, SessionTokenAuthProvider, SessionTokenRequest, SessionTokenSigner,
};
use everruns_core::mcp::{McpClient, McpConnection, NoAuthProvider, load_injected_server_state};
use serde_json::json;
use tower::ServiceExt;

// An IP literal passes the client's SSRF check without DNS; the egress below
// hands every request to the router in process.
const BASE_URL: &str = "http://8.8.8.8/v1";
const KB: &str = "kb_0123456789abcdef0123456789abcdef";

struct RouterEgress {
    router: Router,
}

#[async_trait]
impl everruns_core::EgressService for RouterEgress {
    async fn send(
        &self,
        request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
        let path = request.url.trim_start_matches("http://8.8.8.8").to_string();
        let mut builder = Request::builder().method(request.method.as_str()).uri(path);
        for (name, value) in &request.headers {
            builder = builder.header(name, value);
        }
        let response = self
            .router
            .clone()
            .oneshot(builder.body(Body::from(request.body)).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_str().unwrap_or("").to_string()))
            .collect();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec();
        Ok(everruns_core::EgressResponse {
            status,
            headers,
            body,
        })
    }

    async fn send_stream(
        &self,
        request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(everruns_core::EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::iter([Ok(response.body)])),
        })
    }
}

/// One org, one session whose agent uses Knowledge Base over `KB`.
struct OneSession {
    org: OrgId,
    session: SessionId,
}

#[async_trait]
impl SessionCapabilityConfig for OneSession {
    async fn config(
        &self,
        org: OrgId,
        session: SessionId,
        capability_ids: &[&str],
    ) -> anyhow::Result<Option<Value>> {
        assert_eq!(
            capability_ids,
            ["injected:knowledge_base", "knowledge_base"]
        );
        Ok((org == self.org && session == self.session).then(|| json!({ "bases": [KB] })))
    }
}

struct OneEntry;

#[async_trait]
impl KnowledgeStore for OneEntry {
    async fn search_knowledge(
        &self,
        _org_id: OrgId,
        kb_public_ids: &[String],
        query: &str,
        _kind: Option<&str>,
        _tags: &[String],
        _limit: usize,
    ) -> CoreResult<Vec<KnowledgeSearchHit>> {
        assert_eq!(kb_public_ids, [KB.to_string()]);
        Ok(vec![KnowledgeSearchHit {
            id: "kbe_1".into(),
            kb_id: KB.into(),
            title: format!("Revenue rules ({query})"),
            kind: "business".into(),
            tags: Vec::new(),
            snippet: "Revenue excludes refunds.".into(),
            resource: None,
        }])
    }
}

struct Fixture {
    signer: Arc<SessionTokenSigner>,
    router: Router,
    org: OrgId,
    session: SessionId,
    definition: InjectedCapabilityDefinition,
}

fn fixture() -> Fixture {
    let signer = Arc::new(SessionTokenSigner::generate("https://dp.example").unwrap());
    let verifier =
        SessionTokenVerifier::from_jwks(&signer.jwks(), vec!["https://dp.example".into()]).unwrap();
    let org = OrgId::new();
    let session = SessionId::new();
    let service = Arc::new(KnowledgeBaseService::new(
        BASE_URL,
        Arc::new(OneSession { org, session }),
        Arc::new(OneEntry),
    ));
    let capabilities = InjectedCapabilities::new(BASE_URL, Some(verifier)).register(service);
    let definition = capabilities.definitions().remove(0);
    let router = routes(Arc::new(capabilities), "/v1");
    Fixture {
        signer,
        router,
        org,
        session,
        definition,
    }
}

fn client(fixture: &Fixture, group: OrgId, session: SessionId) -> McpClient {
    let group = group.to_string();
    let minter = Arc::new(LocalSessionTokenMinter::new(
        fixture.signer.clone(),
        Arc::new(move |_| Some(group.clone())),
    ));
    McpClient::new(
        Arc::new(RouterEgress {
            router: fixture.router.clone(),
        }),
        Arc::new(SessionTokenAuthProvider::new(
            Arc::new(NoAuthProvider),
            minter,
        )),
    )
    .for_session(session.uuid())
}

fn connection(definition: &InjectedCapabilityDefinition) -> McpConnection {
    let mut connection = McpConnection::http(definition.name(), definition.server.url.clone());
    connection.auth_mode = McpServerAuthMode::SessionToken;
    connection
}

#[test]
fn definition_points_at_this_server() {
    let fixture = fixture();
    let definition = &fixture.definition;
    definition.validate().unwrap();
    assert_eq!(definition.id(), "injected:knowledge_base");
    assert_eq!(
        definition.server.url,
        "http://8.8.8.8/v1/injected/knowledge_base/mcp"
    );
    assert_eq!(definition.tools, InjectedToolNames::Native);
    assert!(
        definition
            .capability
            .system_prompt
            .as_deref()
            .unwrap()
            .contains("search_knowledge")
    );
}

#[tokio::test]
async fn data_plane_session_lists_and_calls_search_knowledge() {
    let fixture = fixture();
    let client = client(&fixture, fixture.org, fixture.session);
    let connection = connection(&fixture.definition);

    let tools = client.discover(&connection).await.unwrap();
    assert_eq!(
        tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        ["search_knowledge"]
    );

    let result = client
        .call(
            &connection,
            "search_knowledge",
            json!({ "query": "revenue" }),
            None,
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&result).unwrap();
    assert!(text.contains("Revenue rules (revenue)"), "{text}");
    assert!(text.contains("kbe_1"), "{text}");

    // Knowledge Base speaks no extension yet: no facts, no hooks.
    let state = load_injected_server_state(&client, &connection)
        .await
        .unwrap();
    assert!(state.extensions.0.is_empty());
    assert!(state.facts.is_empty());
}

#[tokio::test]
async fn session_from_another_org_gets_no_tools() {
    let fixture = fixture();
    let other_org = client(&fixture, OrgId::new(), fixture.session);
    let tools = other_org
        .discover(&connection(&fixture.definition))
        .await
        .unwrap();
    assert!(tools.is_empty());

    let other_session = client(&fixture, fixture.org, SessionId::new());
    let call = other_session
        .call(
            &connection(&fixture.definition),
            "search_knowledge",
            json!({ "query": "revenue" }),
            None,
        )
        .await;
    assert!(call.is_err());
}

async fn post(router: &Router, path: &str, token: Option<&str>) -> StatusCode {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    router
        .clone()
        .oneshot(
            request
                .body(Body::from(
                    json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn refuses_missing_foreign_and_misaddressed_tokens() {
    let fixture = fixture();
    let path = "/v1/injected/knowledge_base/mcp";
    let good = fixture
        .signer
        .sign(
            &SessionTokenRequest {
                session_id: fixture.session.uuid(),
                capability_id: "injected:knowledge_base".into(),
                audience: fixture.definition.server.url.clone(),
            },
            Some(fixture.org.to_string()),
        )
        .unwrap();
    assert_eq!(
        post(&fixture.router, path, Some(&good)).await,
        StatusCode::OK
    );
    assert_eq!(
        post(&fixture.router, path, None).await,
        StatusCode::UNAUTHORIZED
    );

    // Another data plane's key.
    let stranger = SessionTokenSigner::generate("https://dp.example").unwrap();
    let foreign = stranger
        .sign(
            &SessionTokenRequest {
                session_id: fixture.session.uuid(),
                capability_id: "injected:knowledge_base".into(),
                audience: fixture.definition.server.url.clone(),
            },
            Some(fixture.org.to_string()),
        )
        .unwrap();
    assert_eq!(
        post(&fixture.router, path, Some(&foreign)).await,
        StatusCode::UNAUTHORIZED
    );

    // A token minted for another server.
    let elsewhere = fixture
        .signer
        .sign(
            &SessionTokenRequest {
                session_id: fixture.session.uuid(),
                capability_id: "injected:knowledge_base".into(),
                audience: "https://evil.example/mcp".into(),
            },
            Some(fixture.org.to_string()),
        )
        .unwrap();
    assert_eq!(
        post(&fixture.router, path, Some(&elsewhere)).await,
        StatusCode::UNAUTHORIZED
    );

    // Unknown capabilities are not found, before any token check.
    assert_eq!(
        post(&fixture.router, "/v1/injected/poppy/mcp", Some(&good)).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn endpoint_is_off_without_trusted_keys() {
    let service = Arc::new(KnowledgeBaseService::new(
        BASE_URL,
        Arc::new(OneSession {
            org: OrgId::new(),
            session: SessionId::new(),
        }),
        Arc::new(OneEntry),
    ));
    let capabilities = InjectedCapabilities::new(BASE_URL, None).register(service);
    assert!(!capabilities.is_enabled());
    let router = routes(Arc::new(capabilities), "/v1");
    assert_eq!(
        post(&router, "/v1/injected/knowledge_base/mcp", Some("x")).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn the_linked_data_planes_tokens_are_trusted_from_its_key_alone() {
    use everruns_contracts::dataplane_seal::{SESSION_TOKEN_ISSUER, session_token_pkcs8};
    let dp_key = [5u8; 32];
    let org = OrgId::new();
    let session = SessionId::new();
    let service = Arc::new(KnowledgeBaseService::new(
        BASE_URL,
        Arc::new(OneSession { org, session }),
        Arc::new(OneEntry),
    ));
    let verifier = super::linked_data_plane_verifier(&dp_key).unwrap();
    let capabilities = InjectedCapabilities::new(BASE_URL, Some(verifier)).register(service);
    let url = capabilities.definitions().remove(0).server.url;
    let router = routes(Arc::new(capabilities), "/v1");
    let path = "/v1/injected/knowledge_base/mcp";
    let request = SessionTokenRequest {
        session_id: session.uuid(),
        capability_id: "injected:knowledge_base".into(),
        audience: url,
    };
    // What a cell's executor signs with (`mcp.rs` in everruns-agentd).
    let executor =
        SessionTokenSigner::from_pkcs8(SESSION_TOKEN_ISSUER, &session_token_pkcs8(&dp_key))
            .unwrap();
    let token = executor.sign(&request, Some(org.to_string())).unwrap();
    assert_eq!(post(&router, path, Some(&token)).await, StatusCode::OK);
    // Another data plane's key is not trusted.
    let other =
        SessionTokenSigner::from_pkcs8(SESSION_TOKEN_ISSUER, &session_token_pkcs8(&[6u8; 32]))
            .unwrap();
    let token = other.sign(&request, Some(org.to_string())).unwrap();
    assert_eq!(
        post(&router, path, Some(&token)).await,
        StatusCode::UNAUTHORIZED
    );
}

#[test]
fn a_data_planes_package_calls_the_served_capability_instead_of_the_hosted_one() {
    let served = vec![fixture().definition];
    let mut package = serde_json::json!({
        "name": "a",
        "capabilities": [
            "current_time",
            { "ref": "knowledge_base", "config": { "limit": 3 } },
            { "ref": "injected:knowledge_base", "config": {
                "name": "knowledge_base", "description": "x",
                "mcp_servers": { "knowledge_base": { "url": "https://evil.example/mcp", "actsAs": "none" } }
            } },
            { "ref": "injected:poppy" },
        ],
    });
    super::attach(&mut package, &served);
    let capabilities = package["capabilities"].as_array().unwrap();
    // Hosted Knowledge Base becomes the injected one, once, in its place; an
    // author's URL never survives; one not served here is dropped.
    assert_eq!(capabilities.len(), 2, "{capabilities:?}");
    assert_eq!(capabilities[0], "current_time");
    assert_eq!(capabilities[1]["ref"], "injected:knowledge_base");
    let server = &capabilities[1]["config"]["mcp_servers"]["knowledge_base"];
    assert_eq!(server["url"], served[0].server.url.as_str());
    assert_eq!(server["auth_mode"], "session_token");

    // Nothing served (no trusted data plane): the package is left alone.
    let mut untouched = serde_json::json!({ "capabilities": ["knowledge_base"] });
    super::attach(&mut untouched, &[]);
    assert_eq!(
        untouched["capabilities"],
        serde_json::json!(["knowledge_base"])
    );
}

#[test]
fn slacks_model_view_stays_native_beside_its_injected_tools() {
    let served = vec![super::hosted::definition_of(
        &everruns_capabilities::capabilities::SlackCapability,
        BASE_URL,
    )];
    let mut package = serde_json::json!({ "capabilities": ["slack", "slack_model_view"] });
    super::attach(&mut package, &served);
    assert_eq!(
        package["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| super::ref_id(c).unwrap())
            .collect::<Vec<_>>(),
        ["injected:slack", "slack_model_view"]
    );
    assert!(served[0].validate().is_ok(), "{:?}", served[0].validate());
}

/// The `memory_file` handler, for one org and session: `/memory/user` only.
struct MemoryCalls {
    org: OrgId,
    session: SessionId,
    asked: parking_lot::Mutex<Vec<Value>>,
}

#[async_trait]
impl hosted::HostCallRunner for MemoryCalls {
    async fn answer(
        &self,
        org: OrgId,
        call: &'static everruns_contracts::host_calls::HostCall,
        params: Value,
    ) -> anyhow::Result<everruns_contracts::host_calls::HostCallAnswer> {
        assert_eq!(call.name, everruns_contracts::host_calls::MEMORY_FILE);
        assert_eq!(org, self.org);
        assert_eq!(params["session_id"], self.session.uuid().to_string());
        self.asked.lock().push(params.clone());
        let path = params["path"].as_str().unwrap();
        let answer = if !path.starts_with("/memory/user") {
            json!({ "error": format!("{path} is not in this session's Memory") })
        } else {
            json!({ "ok": {
                "path": path,
                "content": "likes tea",
                "encoding": "text",
                "is_directory": false,
            } })
        };
        Ok(everruns_contracts::host_calls::HostCallAnswer::Value(
            answer,
        ))
    }
}

#[tokio::test]
async fn memory_is_served_as_files_through_the_memory_file_handler() {
    let signer = Arc::new(SessionTokenSigner::generate("https://dp.example").unwrap());
    let verifier =
        SessionTokenVerifier::from_jwks(&signer.jwks(), vec!["https://dp.example".into()]).unwrap();
    let org = OrgId::new();
    let session = SessionId::new();
    let capabilities = InjectedCapabilities::new(BASE_URL, Some(verifier));
    let runner = Arc::new(MemoryCalls {
        org,
        session,
        asked: Default::default(),
    });
    capabilities.host_calls().bind(runner.clone());
    let service = hosted::memory::service(BASE_URL, capabilities.host_calls());
    let capabilities = capabilities.register(Arc::new(service));
    let definition = capabilities
        .definitions()
        .into_iter()
        .find(|d| d.name() == hosted::memory::NAME)
        .unwrap();
    assert_eq!(
        definition.server.url,
        "http://8.8.8.8/v1/injected/memory_files/mcp"
    );
    let fixture = Fixture {
        signer,
        router: routes(Arc::new(capabilities), "/v1"),
        org,
        session,
        definition,
    };
    let client = client(&fixture, org, session);
    let connection = connection(&fixture.definition);

    let read = client
        .files(
            &connection,
            "everruns/files/read",
            json!({ "path": "/memory/user/notes.md" }),
        )
        .await
        .unwrap();
    assert_eq!(read["content"], "likes tea");
    assert_eq!(
        read["etag"],
        everruns_contracts::runtime::mcp_extensions::file_etag("likes tea", "text").as_str()
    );
    assert_eq!(runner.asked.lock()[0]["op"], "read");

    // The handler's refusal, and a path that escapes, are the caller's errors.
    let refused = client
        .files(
            &connection,
            "everruns/files/read",
            json!({ "path": "/memory/agent/x.md" }),
        )
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("not in this session's Memory"),
        "{refused}"
    );
    let asked = runner.asked.lock().len();
    assert!(
        client
            .files(
                &connection,
                "everruns/files/read",
                json!({ "path": "/memory/user/../agent/x.md" }),
            )
            .await
            .is_err()
    );
    assert_eq!(runner.asked.lock().len(), asked);
}

/// The `platform_command` handler: says who ran what.
struct PlatformCalls {
    org: OrgId,
    session: SessionId,
}

#[async_trait]
impl hosted::HostCallRunner for PlatformCalls {
    async fn answer(
        &self,
        org: OrgId,
        call: &'static everruns_contracts::host_calls::HostCall,
        params: Value,
    ) -> anyhow::Result<everruns_contracts::host_calls::HostCallAnswer> {
        assert_eq!(call.name, everruns_contracts::host_calls::PLATFORM_COMMAND);
        assert_eq!(org, self.org);
        assert_eq!(params["session_id"], self.session.to_string());
        Ok(everruns_contracts::host_calls::HostCallAnswer::Value(
            Value::String(format!(
                "{} ran {} {}",
                params["acting_user"].as_str().unwrap(),
                params["operation"].as_str().unwrap(),
                params["arguments"]
            )),
        ))
    }
}

/// Mints as a data plane does for a turn `actor` opened.
struct Acting {
    signer: Arc<SessionTokenSigner>,
    org: OrgId,
    actor: Option<String>,
}

#[async_trait]
impl everruns_core::mcp::session_token::SessionTokenMinter for Acting {
    async fn mint(&self, request: &SessionTokenRequest) -> anyhow::Result<String> {
        self.signer.sign_as(
            request,
            Some(self.org.to_string()),
            self.actor.clone().map(|sub| {
                everruns_contracts::runtime::capabilities::injected::SessionActor { sub }
            }),
        )
    }
}

#[tokio::test]
async fn platform_commands_run_as_the_person_who_opened_the_turn() {
    let signer = Arc::new(SessionTokenSigner::generate("https://dp.example").unwrap());
    let verifier =
        SessionTokenVerifier::from_jwks(&signer.jwks(), vec!["https://dp.example".into()]).unwrap();
    let org = OrgId::new();
    let session = SessionId::new();
    let capabilities = InjectedCapabilities::new(BASE_URL, Some(verifier));
    capabilities
        .host_calls()
        .bind(Arc::new(PlatformCalls { org, session }));
    let service = hosted::platform::service(BASE_URL, capabilities.host_calls());
    let capabilities = capabilities.register(Arc::new(service));
    let definition = capabilities.definitions().remove(0);
    assert_eq!(definition.name(), hosted::platform::NAME);
    let router = routes(Arc::new(capabilities), "/v1");
    let call = |actor: Option<&str>| {
        let client = McpClient::new(
            Arc::new(RouterEgress {
                router: router.clone(),
            }),
            Arc::new(SessionTokenAuthProvider::new(
                Arc::new(NoAuthProvider),
                Arc::new(Acting {
                    signer: signer.clone(),
                    org,
                    actor: actor.map(str::to_string),
                }),
            )),
        )
        .for_session(session.uuid());
        let connection = connection(&definition);
        async move {
            client
                .call(
                    &connection,
                    hosted::platform::TOOL,
                    json!({ "operation": "execute", "arguments": { "command": "agents list" } }),
                    None,
                )
                .await
                .unwrap()
        }
    };
    let text = |result: &everruns_core::McpToolCallResult| match &result.content[0] {
        everruns_core::McpContent::Text { text, .. } => text.clone(),
        other => panic!("{other:?}"),
    };

    let ran = call(Some("usr_1")).await;
    assert!(!ran.is_error);
    assert!(
        text(&ran).starts_with("usr_1 ran execute"),
        "{}",
        text(&ran)
    );
    // No person opened the turn: nothing runs.
    let refused = call(None).await;
    assert!(refused.is_error);
    assert!(text(&refused).contains("a turn a person started"));
}
