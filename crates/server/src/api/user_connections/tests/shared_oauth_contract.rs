//! Exercise the same login contract through embedded core and browser Server adapters.
use super::*;
use everruns_core::mcp::oauth;
use std::sync::Mutex;

#[derive(Default)]
struct ContractEgress {
    requests: Mutex<Vec<EgressRequest>>,
    fail_token: bool,
}
#[async_trait::async_trait]
impl EgressService for ContractEgress {
    async fn send(&self, request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        let (status, body) = match (request.method.as_str(), request.url.as_str()) {
            // Exercise the root-resource and path-appended OpenID fallbacks in both hosts.
            ("GET", "https://8.8.8.8/.well-known/oauth-protected-resource") => (
                200,
                serde_json::json!({
                    "resource": "https://8.8.8.8/audience", "authorization_servers": ["https://1.1.1.1/tenant"], "scopes_supported": ["tools:read"]
                }),
            ),
            ("GET", "https://1.1.1.1/tenant/.well-known/openid-configuration") => (
                200,
                serde_json::json!({
                    "issuer": "https://1.1.1.1/tenant", "authorization_endpoint": "https://1.1.1.1/authorize",
                    "token_endpoint": "https://1.1.1.1/token", "registration_endpoint": "https://1.1.1.1/register"
                }),
            ),
            ("POST", "https://1.1.1.1/register") => (
                200,
                serde_json::json!({"client_id": "contract-client", "client_secret": "client-secret"}),
            ),
            ("POST", "https://1.1.1.1/token") if self.fail_token => (
                400,
                serde_json::json!({"error": "invalid_grant", "error_description": "client-secret refresh-secret code-secret"}),
            ),
            ("POST", "https://1.1.1.1/token") => (
                200,
                serde_json::json!({"access_token": "contract-access", "refresh_token": "contract-refresh", "expires_in": 3600}),
            ),
            _ => (404, serde_json::Value::Null),
        };
        self.requests.lock().unwrap().push(request);
        Ok(EgressResponse {
            status,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }
    async fn send_stream(
        &self,
        _: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        unreachable!()
    }
}
fn params(url: &str) -> BTreeMap<String, String> {
    url::Url::parse(url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}
fn authorization_contract(url: &str) {
    let params = params(url);
    assert_eq!(params["client_id"], "contract-client");
    assert_eq!(params["resource"], "https://8.8.8.8/audience");
    assert_eq!(params["code_challenge_method"], "S256");
    assert!(!params["state"].is_empty());
    assert!(!params["code_challenge"].is_empty());
}
fn request_contract(egress: &ContractEgress, org: Option<i64>) {
    let requests = egress.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .all(|r| r.dns_pinning_required || r.pinned_addrs.is_some())
    );
    let expected_scope =
        org.and_then(|id| crate::domains::mcp_servers::presentation::org_scope(id).org_id);
    assert!(
        requests
            .iter()
            .all(|r| r.scope.as_ref().and_then(|scope| scope.org_id) == expected_scope)
    );
    for request in requests.iter().filter(|r| r.url.ends_with("/token")) {
        let form: BTreeMap<String, String> = serde_urlencoded::from_bytes(&request.body).unwrap();
        assert_eq!(form["resource"], "https://8.8.8.8/audience");
        assert_eq!(form["client_secret"], "client-secret");
    }
}

#[tokio::test]
async fn framework_and_server_share_discovery_registration_pkce_exchange_and_audience() {
    // Embedded login: the same entry points used by Framework's OAuthLogin.
    let embedded = Arc::new(ContractEgress::default());
    let prepared = oauth::prepare_login(
        embedded.as_ref(),
        "https://8.8.8.8/mcp",
        "http://localhost/callback",
        "contract",
        None,
        Some("tools:read"),
    )
    .await
    .unwrap();
    authorization_contract(&prepared.authorization_url);
    let tokens = oauth::complete_login(
        embedded.as_ref(),
        &prepared,
        "code",
        &prepared.state,
        Some("https://1.1.1.1/tenant"),
    )
    .await
    .unwrap();
    assert_eq!(tokens.access_token, "contract-access");
    request_contract(&embedded, None);

    // Browser login: real registration persistence, state cookie, callback and encrypted grant.
    let browser = Arc::new(ContractEgress::default());
    let (mut state, org, server_id, agent_id, _) = identity_oauth_fixture(false).await;
    state.mcp_service = Arc::new(McpServerService::with_egress_service(
        state.db.clone(),
        state.encryption.clone(),
        browser.clone(),
    ));
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, redirect) = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_id)
        .await
        .unwrap();
    let target = redirect.into_response().headers()[axum::http::header::LOCATION]
        .to_str()
        .unwrap()
        .to_string();
    authorization_contract(&target);
    let pending = pending_state(&jar, &provider);
    let identity: VirtualUserId = pending.virtual_user_id.as_deref().unwrap().parse().unwrap();
    complete_mcp_oauth_callback(
        &state,
        Ok(org.clone()),
        server_id,
        &provider,
        &pending,
        &OAuthCallbackQuery {
            code: Some("code".into()),
            state: Some(pending.state.clone()),
            iss: Some("https://1.1.1.1/tenant".into()),
            error: None,
            error_description: None,
        },
    )
    .await
    .unwrap();
    let grant = state
        .db
        .get_virtual_user_connection(identity, &provider)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state
            .encryption
            .as_ref()
            .unwrap()
            .decrypt_to_string(grant.access_token_encrypted.as_deref().unwrap())
            .unwrap(),
        tokens.access_token
    );
    let saved = state
        .encryption
        .as_ref()
        .unwrap()
        .decrypt_to_string(grant.refresh_token_encrypted.as_deref().unwrap())
        .unwrap();
    let saved: McpRefreshGrant = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved.resource, tokens.resource.unwrap());
    request_contract(&browser, Some(org.org_id));

    // DCR authentication policy differs intentionally; request implementation is shared.
    for (egress, expected) in [(&embedded, "none"), (&browser, "client_secret_post")] {
        let sent = egress.requests.lock().unwrap();
        let registration = sent.iter().find(|r| r.url.ends_with("/register")).unwrap();
        let body: serde_json::Value = serde_json::from_slice(&registration.body).unwrap();
        assert_eq!(body["token_endpoint_auth_method"], expected);
    }
}

#[tokio::test]
async fn browser_rejects_wrong_callback_issuer_before_exchange_and_consumes_setup() {
    let browser = Arc::new(ContractEgress::default());
    let (mut state, org, server_id, agent_id, _) = identity_oauth_fixture(false).await;
    state.mcp_service = Arc::new(McpServerService::with_egress_service(
        state.db.clone(),
        state.encryption.clone(),
        browser.clone(),
    ));
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, _) = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_id)
        .await
        .unwrap();
    let pending = pending_state(&jar, &provider);
    let query = OAuthCallbackQuery {
        code: Some("code".into()),
        state: Some(pending.state.clone()),
        iss: Some("https://attacker.example".into()),
        error: None,
        error_description: None,
    };
    let error = complete_mcp_oauth_callback(
        &state,
        Ok(org.clone()),
        server_id,
        &provider,
        &pending,
        &query,
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(error.1.contains("issuer mismatch"));
    assert!(
        browser
            .requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| !r.url.ends_with("/token"))
    );
    let identity: VirtualUserId = pending.virtual_user_id.as_deref().unwrap().parse().unwrap();
    assert!(
        state
            .db
            .get_virtual_user_connection(identity, &provider)
            .await
            .unwrap()
            .is_none()
    );
    // A failed callback cannot be replayed with corrected parameters.
    assert!(
        complete_mcp_oauth_callback(
            &state,
            Ok(org),
            server_id,
            &provider,
            &pending,
            &OAuthCallbackQuery {
                iss: Some("https://1.1.1.1/tenant".into()),
                ..query
            }
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn both_exchange_adapters_redact_provider_secrets() {
    let egress = ContractEgress {
        fail_token: true,
        ..Default::default()
    };
    let client = oauth::RegisteredClient {
        client_id: "contract-client".into(),
        client_secret: Some("client-secret".into()),
    };
    let embedded = oauth::OAuthClient::new(&egress, everruns_core::EgressRequestKind::Mcp)
        .exchange_code(
            "https://1.1.1.1/token",
            &client,
            "code-secret",
            "verifier",
            "http://localhost/callback",
            Some("https://8.8.8.8/audience"),
        )
        .await
        .unwrap_err();
    let browser = crate::oauth_client::exchange_oauth_code(
        &egress,
        OAuthCodeExchangeRequest {
            token_endpoint: "https://1.1.1.1/token",
            client_id: &client.client_id,
            client_secret: client.client_secret.as_deref(),
            code: "code-secret",
            code_verifier: "verifier",
            redirect_uri: "https://app.example/callback",
            resource: Some("https://8.8.8.8/audience"),
        },
    )
    .await
    .unwrap_err();
    let errors = format!("{embedded} {embedded:?} {browser:?}");
    assert!(errors.contains("invalid_grant"));
    for secret in ["client-secret", "refresh-secret", "code-secret"] {
        assert!(!errors.contains(secret));
    }
}
