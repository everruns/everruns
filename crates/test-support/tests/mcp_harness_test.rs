#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use everruns_core::{
    EgressRequest, EgressRequestKind, EgressService, McpProtocolMode, McpServerAuthMode,
};
use everruns_mcp::oauth::{OAuthClient, OAuthError, RegisteredClient, prepare_login};
use everruns_mcp::{
    ElicitationAction, McpClient, McpConnection, NoAuthProvider, StaticAuthProvider,
    UrlElicitation, UrlElicitationHandler,
};
use everruns_test_support::{
    MockCallResponse, MockMcpOAuthServer, MockMcpProtocolEra, MockOAuthError,
};
use serde_json::json;
use sha2::{Digest, Sha256};

fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()).as_slice())
}

fn connection(mock: &MockMcpOAuthServer, era: MockMcpProtocolEra) -> McpConnection {
    let mode = match era {
        MockMcpProtocolEra::V2025March => McpProtocolMode::V2025March,
        MockMcpProtocolEra::V2025June => McpProtocolMode::V2025June,
        MockMcpProtocolEra::V2026July => McpProtocolMode::V2026July,
    };
    McpConnection::http("linear", mock.mcp_url()).with_protocol_mode(mode)
}

#[tokio::test]
async fn records_credentials_and_absence_at_the_mcp_boundary() {
    let mock = MockMcpOAuthServer::default();
    let authenticated = McpClient::new(
        Arc::new(mock.clone()),
        Arc::new(StaticAuthProvider::new().with_bearer("linear", "user-token")),
    );
    let mut authenticated_connection = connection(&mock, MockMcpProtocolEra::V2026July);
    authenticated_connection.auth_mode = McpServerAuthMode::OAuth;
    authenticated.discover(&authenticated_connection).await.unwrap();
    mock.assert_called_as_user("user-token");

    let identity = McpClient::new(
        Arc::new(mock.clone()),
        Arc::new(StaticAuthProvider::new().with_bearer("linear", "identity-token")),
    );
    let mut identity_connection = connection(&mock, MockMcpProtocolEra::V2026July);
    identity_connection.auth_mode = McpServerAuthMode::OAuth;
    identity.discover(&identity_connection).await.unwrap();
    mock.assert_called_as_identity("identity-token");

    let anonymous = McpClient::new(Arc::new(mock.clone()), Arc::new(NoAuthProvider));
    anonymous
        .discover(&connection(&mock, MockMcpProtocolEra::V2026July))
        .await
        .unwrap();
    mock.assert_no_authorization_header();
}

#[tokio::test]
async fn accepts_and_distinguishes_all_supported_protocol_eras() {
    for era in [
        MockMcpProtocolEra::V2025March,
        MockMcpProtocolEra::V2025June,
        MockMcpProtocolEra::V2026July,
    ] {
        let mock = MockMcpOAuthServer::new(era);
        let client = McpClient::new(Arc::new(mock.clone()), Arc::new(NoAuthProvider));
        let tools = client.discover(&connection(&mock, era)).await.unwrap();
        assert_eq!(tools.len(), 1);

        let requests = mock.mcp_requests();
        assert_eq!(
            requests
                .last()
                .and_then(|request| request.header("MCP-Protocol-Version")),
            Some(era.version())
        );
        let initialize_count = requests
            .iter()
            .filter(|request| request.method == "initialize")
            .count();
        assert_eq!(
            initialize_count,
            usize::from(era != MockMcpProtocolEra::V2026July)
        );
    }
}

#[tokio::test]
async fn serves_cache_hints_call_results_and_mrtr() {
    let mock = MockMcpOAuthServer::default();
    mock.set_tools(vec![json!({
        "name": "search",
        "description": "Search",
        "inputSchema": {"type": "object"}
    })]);
    mock.set_cache_hints(60_000, "private");
    mock.set_call_response(
        "search",
        MockCallResponse::InputRequired {
            request_state: "opaque".to_string(),
            result: json!({
                "resultType": "complete",
                "content": [{"type": "text", "text": "found"}],
                "isError": false
            }),
        },
    );
    let client = McpClient::new(Arc::new(mock.clone()), Arc::new(NoAuthProvider));
    let connection = connection(&mock, MockMcpProtocolEra::V2026July);

    assert_eq!(client.discover(&connection).await.unwrap()[0].name, "search");
    assert_eq!(client.discover(&connection).await.unwrap()[0].name, "search");
    assert_eq!(
        mock.mcp_requests()
            .iter()
            .filter(|request| request.method == "tools/list")
            .count(),
        1,
        "positive cache hints must serve the second discovery"
    );
    let result = client
        .call(&connection, "search", json!({"query": "issue"}))
        .await
        .unwrap();
    assert!(!result.is_error);

    let calls: Vec<_> = mock
        .mcp_requests()
        .into_iter()
        .filter(|request| request.method == "tools/call")
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].body["params"]["requestState"], "opaque");
}

struct AcceptElicitation;

#[async_trait]
impl UrlElicitationHandler for AcceptElicitation {
    async fn request_url_consent(
        &self,
        _elicitation: &UrlElicitation,
    ) -> anyhow::Result<ElicitationAction> {
        Ok(ElicitationAction::Accept)
    }
}

#[tokio::test]
async fn serves_url_mode_elicitation_until_the_client_accepts() {
    let mock = MockMcpOAuthServer::default();
    mock.set_call_response(
        "connect",
        MockCallResponse::UrlElicitation {
            request_state: "connect-state".to_string(),
            url: "https://linear.example/connect".to_string(),
            message: "Connect Linear".to_string(),
            result: json!({
                "resultType": "complete",
                "content": [{"type": "text", "text": "connected"}],
                "isError": false
            }),
        },
    );
    let client = McpClient::with_url_elicitation(
        Arc::new(mock.clone()),
        Arc::new(NoAuthProvider),
        Arc::new(AcceptElicitation),
    );

    let result = client
        .call(
            &connection(&mock, MockMcpProtocolEra::V2026July),
            "connect",
            json!({}),
        )
        .await
        .unwrap();
    assert!(!result.is_error);

    let calls: Vec<_> = mock
        .mcp_requests()
        .into_iter()
        .filter(|request| request.method == "tools/call")
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[1].body["params"]["inputResponses"]["connect"]["action"],
        "accept"
    );
}

#[tokio::test]
async fn oauth_login_discovers_registers_and_verifies_pkce() {
    let mock = MockMcpOAuthServer::default();
    let verifier = "correct-verifier";
    let prepared = prepare_login(
        &mock,
        &mock.mcp_url(),
        "http://127.0.0.1:7777/callback",
        "everruns-test",
        None,
        Some("issues.write"),
    )
    .await
    .unwrap();
    let code = mock
        .authorize(challenge(verifier), None, Some("issues.write"))
        .unwrap();
    let oauth = OAuthClient::new(&mock, EgressRequestKind::Mcp);

    let wrong = oauth
        .exchange_code(
            &mock.token_endpoint(),
            prepared.client(),
            &code,
            "wrong-verifier",
            "http://127.0.0.1:7777/callback",
            Some(&mock.mcp_url()),
        )
        .await
        .expect_err("a wrong verifier must fail");
    assert!(matches!(wrong, OAuthError::Http { status: 400, .. }));

    let prepared = prepare_login(
        &mock,
        &mock.mcp_url(),
        "http://127.0.0.1:7777/callback",
        "everruns-test",
        Some(RegisteredClient {
            client_id: "test-client".to_string(),
            client_secret: None,
        }),
        Some("issues.write"),
    )
    .await
    .unwrap();
    let code = mock
        .authorize(challenge(verifier), None, Some("issues.write"))
        .unwrap();
    let tokens = oauth
        .exchange_code(
            &mock.token_endpoint(),
            prepared.client(),
            &code,
            verifier,
            "http://127.0.0.1:7777/callback",
            Some(&mock.mcp_url()),
        )
        .await
        .unwrap();
    assert_eq!(tokens.access_token, "access-1");
    assert_eq!(tokens.refresh_token.as_deref(), Some("refresh-1"));
}

#[tokio::test]
async fn refresh_rotates_and_revoke_invalidates_the_latest_token() {
    let mock = MockMcpOAuthServer::default();
    let oauth = OAuthClient::new(&mock, EgressRequestKind::Mcp);
    let verifier = "rotation-verifier";
    let code = mock.authorize(challenge(verifier), None, None).unwrap();
    let client = RegisteredClient {
        client_id: "test-client".to_string(),
        client_secret: None,
    };
    let first = oauth
        .exchange_code(
            &mock.token_endpoint(),
            &client,
            &code,
            verifier,
            "http://127.0.0.1:7777/callback",
            None,
        )
        .await
        .unwrap();
    let second = oauth.refresh(&first).await.unwrap();
    assert_eq!(second.refresh_token.as_deref(), Some("refresh-2"));

    let old_error = oauth.refresh(&first).await.expect_err("old token is rotated");
    assert!(matches!(old_error, OAuthError::Http { status: 400, .. }));

    let revoke = EgressRequest::new(
        "POST",
        mock.revocation_endpoint(),
        EgressRequestKind::Mcp,
    )
    .header("content-type", "application/x-www-form-urlencoded")
    .body(
        serde_urlencoded::to_string(BTreeMap::from([(
            "token",
            second.refresh_token.as_deref().unwrap(),
        )]))
        .unwrap(),
    );
    assert_eq!(mock.send(revoke).await.unwrap().status, 200);

    let revoked = oauth
        .refresh(&second)
        .await
        .expect_err("revoked refresh token must fail");
    assert!(matches!(revoked, OAuthError::Http { status: 400, .. }));
}

#[test]
fn configurable_actor_and_scope_rejections_are_explicit() {
    let mock = MockMcpOAuthServer::default();
    mock.reject_actor();
    assert_eq!(
        mock.authorize("challenge", Some("application"), None),
        Err(MockOAuthError::ActorRejected)
    );

    let mock = MockMcpOAuthServer::default();
    mock.reject_scope("admin");
    assert_eq!(
        mock.authorize("challenge", None, Some("admin")),
        Err(MockOAuthError::ScopeRejected("admin".to_string()))
    );
}
