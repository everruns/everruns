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
