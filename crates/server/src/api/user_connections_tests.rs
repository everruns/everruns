use super::mcp_oauth::discover_oauth_server_metadata;
use super::*;
use crate::oauth_client::egress_oauth_json;
use crate::storage::models::{CreateAgentRow, CreateMcpServerRow, UpdateMcpServer};
use everruns_core::{
    EgressRequest, EgressResponse, EgressService, OrgRole, Permission, PermissionResolver,
};
use everruns_provider::typed_id::{AgentId, HarnessId};
use std::collections::BTreeMap;
use uuid::Uuid;

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

struct FakeOAuthEgress;

#[async_trait::async_trait]
impl EgressService for FakeOAuthEgress {
    async fn send(&self, request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        if request.method == "POST" && request.url.ends_with("/token") {
            return Ok(EgressResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: serde_json::to_vec(&serde_json::json!({
                    "access_token": "identity-access",
                    "refresh_token": "identity-refresh",
                    "expires_in": 3600,
                    "scope": "issues.write"
                }))
                .unwrap(),
            });
        }
        Ok(EgressResponse {
            status: 502,
            headers: BTreeMap::new(),
            body: Vec::new(),
        })
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("streaming egress is not used by OAuth handlers")
    }
}

#[test]
fn service_authorization_params_cannot_override_reserved_oauth_fields() {
    let params = BTreeMap::from([
        ("actor".to_string(), "app".to_string()),
        (
            "resource".to_string(),
            "https://attacker.example".to_string(),
        ),
    ]);

    let error = validate_authorization_params(&params).unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(error.1.contains("'resource' is reserved"));
}

struct MismatchedIssuerEgress;

#[async_trait::async_trait]
impl EgressService for MismatchedIssuerEgress {
    async fn send(&self, _request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        Ok(EgressResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&serde_json::json!({
                "issuer": "https://1.1.1.1",
                "authorization_endpoint": "https://8.8.8.8/authorize",
                "token_endpoint": "https://8.8.8.8/token"
            }))
            .unwrap(),
        })
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("streaming egress is not used by OAuth discovery")
    }
}

struct DenyAllResolver;

impl PermissionResolver for DenyAllResolver {
    fn has_permission(&self, _caller: &Caller, _permission: &Permission) -> bool {
        false
    }

    fn caller_permissions(&self, _caller: &Caller) -> Vec<Permission> {
        Vec::new()
    }
}

fn test_org(user_id: Uuid) -> ResolvedOrg {
    ResolvedOrg {
        org_id: everruns_core::DEFAULT_ORG_ID,
        public_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        name: "Test".to_string(),
        user_id: Some(user_id),
        role: OrgRole::Owner,
        is_platform_user: false,
        feature_flags: crate::domains::common::all_feature_flags_for_test(),
    }
}

async fn identity_oauth_fixture(configured: bool) -> (AppState, ResolvedOrg, Uuid, String, Uuid) {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption = Arc::new(EncryptionService::new(TEST_KEY, &[]).unwrap());
    let auth_config = AuthConfig::default();
    let auth = AuthState::builtin(auth_config.clone(), db.clone());
    let mcp_service = Arc::new(McpServerService::with_egress_service(
        db.clone(),
        Some(encryption.clone()),
        Arc::new(FakeOAuthEgress),
    ));
    let state = AppState::new(
        db.clone(),
        Some(encryption),
        auth,
        auth_config,
        ConnectorRegistry::new(),
        mcp_service,
    );
    let server_id = Uuid::now_v7();
    let oauth = if configured {
        serde_json::json!({
            "authorization_endpoint": "https://8.8.8.8/authorize",
            "token_endpoint": "https://8.8.8.8/token",
            "client_id": "test-client"
        })
    } else {
        serde_json::json!({})
    };
    db.create_mcp_server_with_id(
        everruns_core::DEFAULT_ORG_ID,
        server_id,
        CreateMcpServerRow {
            name: "linear".to_string(),
            description: None,
            url: "https://8.8.8.8/mcp".to_string(),
            transport_type: "streamable_http".to_string(),
            api_key_encrypted: None,
            headers: None,
            settings: Some(serde_json::json!({
                "auth_mode": "oauth",
                "oauth": oauth
            })),
        },
    )
    .await
    .unwrap();
    let agent = db
        .create_agent(
            everruns_core::DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: AgentId::new().to_string(),
                name: "service-agent".to_string(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: String::new(),
                default_model_id: None,
                harness_id: HarnessId::from_uuid(Uuid::nil()),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    let user_id = Uuid::now_v7();
    (
        state,
        test_org(user_id),
        server_id,
        agent.public_id,
        user_id,
    )
}

fn pending_state(jar: &CookieJar, provider: &str) -> PendingOAuthState {
    let cookie = jar.get(&oauth_state_cookie_name(provider)).unwrap();
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(cookie.value()).unwrap()).unwrap()
}

async fn begin_identity_oauth(
    state: AppState,
    org: ResolvedOrg,
    server_id: Uuid,
    agent_id: String,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    authorize_connection(
        State(state),
        org,
        CookieJar::new(),
        Path(mcp_oauth_provider_id_for_uuid(server_id)),
        Query(OAuthAuthorizeQuery {
            return_to: None,
            mode: Some("identity".to_string()),
            session_id: None,
            agent_id: Some(agent_id),
            popup: None,
        }),
    )
    .await
}

#[tokio::test]
async fn identity_oauth_callback_stores_only_an_identity_grant() {
    let (state, org, server_id, agent_id, user_id) = identity_oauth_fixture(true).await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, _) = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_id.clone())
        .await
        .unwrap();
    let pending = pending_state(&jar, &provider);
    let identity_id = state
        .db
        .get_agent_by_public_id(org.org_id, &agent_id)
        .await
        .unwrap()
        .unwrap()
        .agent_identity_id
        .unwrap();

    let _ = connection_oauth_callback(
        State(state.clone()),
        org,
        jar,
        Path(provider.clone()),
        Query(OAuthCallbackQuery {
            code: Some("code".to_string()),
            state: Some(pending.state),
            error: None,
            error_description: None,
        }),
    )
    .await
    .unwrap();

    let grant = state
        .db
        .get_agent_identity_connection(identity_id, &provider)
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
        "identity-access"
    );
    assert!(
        state
            .db
            .get_user_connection(user_id, &provider)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn identity_oauth_permission_denial_creates_no_identity() {
    let (mut state, org, server_id, agent_id, _) = identity_oauth_fixture(true).await;
    state.auth.permission_resolver = Arc::new(DenyAllResolver);

    let error = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_id.clone())
        .await
        .unwrap_err();

    assert_eq!(error.0, StatusCode::FORBIDDEN);
    assert!(
        state
            .db
            .get_agent_by_public_id(org.org_id, &agent_id)
            .await
            .unwrap()
            .unwrap()
            .agent_identity_id
            .is_none()
    );
}

#[tokio::test]
async fn identity_oauth_discovery_failure_creates_no_identity() {
    let (state, org, server_id, agent_id, _) = identity_oauth_fixture(false).await;

    let error = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_id.clone())
        .await
        .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_GATEWAY);
    assert!(
        state
            .db
            .get_agent_by_public_id(org.org_id, &agent_id)
            .await
            .unwrap()
            .unwrap()
            .agent_identity_id
            .is_none()
    );
}

#[tokio::test]
async fn identity_oauth_rejects_cross_origin_preconfigured_resource() {
    let (state, org, server_id, agent_id, _) = identity_oauth_fixture(true).await;
    state
        .db
        .update_mcp_server(
            org.org_id,
            server_id,
            UpdateMcpServer {
                settings: Some(serde_json::json!({
                    "auth_mode": "oauth",
                    "oauth": {
                        "authorization_endpoint": "https://8.8.8.8/authorize",
                        "token_endpoint": "https://8.8.8.8/token",
                        "client_id": "test-client",
                        "resource": "https://1.1.1.1/legitimate-api"
                    }
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let error = begin_identity_oauth(state, org, server_id, agent_id)
        .await
        .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(error.1.contains("resource origin"));
}

#[tokio::test]
async fn identity_oauth_rejects_insecure_preconfigured_resource() {
    let (state, org, server_id, agent_id, _) = identity_oauth_fixture(true).await;
    state
        .db
        .update_mcp_server(
            org.org_id,
            server_id,
            UpdateMcpServer {
                settings: Some(serde_json::json!({
                    "auth_mode": "oauth",
                    "oauth": {
                        "authorization_endpoint": "https://8.8.8.8/authorize",
                        "token_endpoint": "https://8.8.8.8/token",
                        "client_id": "test-client",
                        "resource": "http://8.8.8.8/mcp"
                    }
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let error = begin_identity_oauth(state, org, server_id, agent_id)
        .await
        .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(error.1.contains("must use HTTPS"));
}

#[tokio::test]
async fn identity_oauth_missing_registration_creates_no_identity() {
    let (state, org, server_id, agent_id, _) = identity_oauth_fixture(true).await;
    state
        .db
        .update_mcp_server(
            org.org_id,
            server_id,
            UpdateMcpServer {
                settings: Some(serde_json::json!({
                    "auth_mode": "oauth",
                    "oauth": {
                        "authorization_endpoint": "https://8.8.8.8/authorize",
                        "token_endpoint": "https://8.8.8.8/token"
                    }
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let error = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_id.clone())
        .await
        .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(
        state
            .db
            .get_agent_by_public_id(org.org_id, &agent_id)
            .await
            .unwrap()
            .unwrap()
            .agent_identity_id
            .is_none()
    );
}

#[tokio::test]
async fn identity_oauth_callback_rejects_mismatched_state_without_grant() {
    let (state, org, server_id, agent_id, _) = identity_oauth_fixture(true).await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, _) = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_id)
        .await
        .unwrap();
    let identity_id: AgentIdentityId = pending_state(&jar, &provider)
        .agent_identity_id
        .as_deref()
        .unwrap()
        .parse()
        .unwrap();

    let error = connection_oauth_callback(
        State(state.clone()),
        org,
        jar,
        Path(provider.clone()),
        Query(OAuthCallbackQuery {
            code: Some("code".to_string()),
            state: Some("wrong-state".to_string()),
            error: None,
            error_description: None,
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(
        state
            .db
            .get_agent_identity_connection(identity_id, &provider)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn identity_oauth_callback_rejects_archived_server_without_grant() {
    let (state, org, server_id, agent_id, _) = identity_oauth_fixture(true).await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, _) = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_id)
        .await
        .unwrap();
    let pending = pending_state(&jar, &provider);
    let identity_id: AgentIdentityId = pending
        .agent_identity_id
        .as_deref()
        .unwrap()
        .parse()
        .unwrap();
    state
        .db
        .update_mcp_server(
            org.org_id,
            server_id,
            UpdateMcpServer {
                status: Some("archived".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let error = connection_oauth_callback(
        State(state.clone()),
        org,
        jar,
        Path(provider.clone()),
        Query(OAuthCallbackQuery {
            code: Some("code".to_string()),
            state: Some(pending.state),
            error: None,
            error_description: None,
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(
        state
            .db
            .get_agent_identity_connection(identity_id, &provider)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn identity_oauth_callback_rechecks_permission_before_writing_grant() {
    let (mut state, org, server_id, agent_id, _) = identity_oauth_fixture(true).await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, _) = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_id)
        .await
        .unwrap();
    let pending = pending_state(&jar, &provider);
    let identity_id: AgentIdentityId = pending
        .agent_identity_id
        .as_deref()
        .unwrap()
        .parse()
        .unwrap();
    state.auth.permission_resolver = Arc::new(DenyAllResolver);

    let error = connection_oauth_callback(
        State(state.clone()),
        org,
        jar,
        Path(provider.clone()),
        Query(OAuthCallbackQuery {
            code: Some("code".to_string()),
            state: Some(pending.state),
            error: None,
            error_description: None,
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(error.0, StatusCode::FORBIDDEN);
    assert!(
        state
            .db
            .get_agent_identity_connection(identity_id, &provider)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn identity_oauth_callback_rejects_deleted_authorized_agent_without_grant() {
    let (state, org, server_id, agent_public_id, _) = identity_oauth_fixture(true).await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let (jar, _) = begin_identity_oauth(state.clone(), org.clone(), server_id, agent_public_id)
        .await
        .unwrap();
    let pending = pending_state(&jar, &provider);
    let agent_id: AgentId = pending.agent_id.as_deref().unwrap().parse().unwrap();
    let identity_id: AgentIdentityId = pending
        .agent_identity_id
        .as_deref()
        .unwrap()
        .parse()
        .unwrap();
    state.db.delete_agent(org.org_id, agent_id).await.unwrap();

    let error = connection_oauth_callback(
        State(state.clone()),
        org,
        jar,
        Path(provider.clone()),
        Query(OAuthCallbackQuery {
            code: Some("code".to_string()),
            state: Some(pending.state),
            error: None,
            error_description: None,
        }),
    )
    .await
    .unwrap_err();

    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(
        state
            .db
            .get_agent_identity_connection(identity_id, &provider)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test]
async fn oauth_discovery_rejects_mismatched_issuer() {
    let error = discover_oauth_server_metadata(&MismatchedIssuerEgress, "https://8.8.8.8").await;

    assert_eq!(error.unwrap_err().0, StatusCode::BAD_GATEWAY);
}
#[test]
fn valid_state_accepted() {
    let state_value = "abc123deadbeef";
    let jar = CookieJar::new().add(Cookie::new(
        GITHUB_INSTALL_STATE_COOKIE,
        state_value.to_string(),
    ));
    let result = validate_install_state(&jar, Some(state_value));
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), state_value);
}

#[test]
fn missing_cookie_rejected() {
    let jar = CookieJar::new();
    let (status, msg) = validate_install_state(&jar, Some("abc123")).unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(msg.contains("expired"));
}

#[test]
fn missing_query_state_rejected() {
    let jar = CookieJar::new().add(Cookie::new(
        GITHUB_INSTALL_STATE_COOKIE,
        "abc123".to_string(),
    ));
    let (status, msg) = validate_install_state(&jar, None).unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(msg.contains("Missing state"));
}

#[test]
fn mismatched_state_rejected() {
    let jar = CookieJar::new().add(Cookie::new(
        GITHUB_INSTALL_STATE_COOKIE,
        "correct_state".to_string(),
    ));
    let (status, msg) = validate_install_state(&jar, Some("wrong_state")).unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(msg.contains("Invalid installation state"));
}

#[test]
fn both_missing_reports_expired() {
    let jar = CookieJar::new();
    // Cookie checked first — reports expired state
    let (status, _) = validate_install_state(&jar, None).unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[test]
fn state_cookie_has_secure_properties() {
    let state_value = "test_state";
    let cookie = Cookie::build((GITHUB_INSTALL_STATE_COOKIE, state_value))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::minutes(10))
        .build();

    assert_eq!(cookie.name(), GITHUB_INSTALL_STATE_COOKIE);
    assert_eq!(cookie.value(), state_value);
    assert!(cookie.http_only().unwrap_or(false));
    assert!(cookie.secure().unwrap_or(false));
    assert_eq!(cookie.same_site(), Some(SameSite::Lax));
    assert_eq!(cookie.max_age(), Some(time::Duration::minutes(10)));
    assert_eq!(cookie.path(), Some("/"));
}

#[test]
fn callback_query_deserialize_with_state() {
    let json = r#"{"installation_id": 12345, "state": "abc123"}"#;
    let query: GitHubInstallationCallbackQuery = serde_json::from_str(json).unwrap();
    assert_eq!(query.installation_id, 12345);
    assert_eq!(query.state, Some("abc123".to_string()));
}

#[test]
fn callback_query_deserialize_without_state() {
    let json = r#"{"installation_id": 12345}"#;
    let query: GitHubInstallationCallbackQuery = serde_json::from_str(json).unwrap();
    assert_eq!(query.installation_id, 12345);
    assert_eq!(query.state, None);
}

// =========================================================================
// GitHub installation callback security negative tests (EVE-54 / EVE-61)
// =========================================================================

#[test]
fn empty_cookie_does_not_match_nonempty_query() {
    let jar = CookieJar::new().add(Cookie::new(GITHUB_INSTALL_STATE_COOKIE, "".to_string()));
    let result = validate_install_state(&jar, Some("attacker_state"));
    assert!(
        result.is_err(),
        "empty cookie must not match non-empty query"
    );
}

#[test]
fn nonempty_cookie_does_not_match_empty_query() {
    let jar = CookieJar::new().add(Cookie::new(
        GITHUB_INSTALL_STATE_COOKIE,
        "real_state".to_string(),
    ));
    let result = validate_install_state(&jar, Some(""));
    assert!(
        result.is_err(),
        "non-empty cookie must not match empty query"
    );
}

#[test]
fn whitespace_padded_state_rejected() {
    let jar = CookieJar::new().add(Cookie::new(
        GITHUB_INSTALL_STATE_COOKIE,
        "abc123".to_string(),
    ));
    let result = validate_install_state(&jar, Some(" abc123 "));
    assert!(result.is_err(), "whitespace-padded state must not match");
}

#[test]
fn all_state_failures_return_bad_request() {
    let (status, _) = validate_install_state(&CookieJar::new(), Some("x")).unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let jar = CookieJar::new().add(Cookie::new(GITHUB_INSTALL_STATE_COOKIE, "x".to_string()));
    let (status, _) = validate_install_state(&jar, None).unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = validate_install_state(&jar, Some("y")).unwrap_err();
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// =========================================================================
// VerifyConnectionResponse serialization tests
// =========================================================================

#[test]
fn verify_response_valid_serializes_without_error() {
    let resp = VerifyConnectionResponse {
        valid: true,
        error: None,
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["valid"], true);
    assert!(
        json.get("error").is_none(),
        "error field should be skipped when None"
    );
}

#[test]
fn existing_plugin_anchor_uses_manifest_connection_display_name() {
    let settings = serde_json::json!({
        "plugin_anchor": {"plugin": "resend", "server": "resend"}
    });
    let display_names = HashMap::from([("resend".to_string(), "Resend".to_string())]);

    assert_eq!(
        mcp_connection_display_name(&settings, "resend", &display_names),
        "Resend"
    );
}

#[test]
fn plugin_connection_label_distinguishes_multiple_servers() {
    let settings = serde_json::json!({
        "plugin_anchor": {"plugin": "mail-suite", "server": "transactional"}
    });

    assert_eq!(
        mcp_connection_display_name(&settings, "plugin-mail", &HashMap::new()),
        "Mail Suite — transactional"
    );
}

#[test]
fn verify_response_invalid_serializes_with_error() {
    let resp = VerifyConnectionResponse {
        valid: false,
        error: Some("Invalid API key".to_string()),
    };
    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["valid"], false);
    assert_eq!(json["error"], "Invalid API key");
}

#[test]
fn normalize_oauth_mode_defaults_to_user() {
    assert_eq!(normalize_oauth_mode(None).unwrap(), "user");
    assert_eq!(normalize_oauth_mode(Some("session")).unwrap(), "session");
}

#[test]
fn normalize_oauth_mode_rejects_unknown_values() {
    let err = normalize_oauth_mode(Some("admin")).unwrap_err();
    assert_eq!(err.0, StatusCode::BAD_REQUEST);
}

#[test]
fn resource_origin_preserves_non_default_port() {
    let url = reqwest::Url::parse("https://example.com:8443/v1/mcp").unwrap();
    assert_eq!(resource_origin(&url).unwrap(), "https://example.com:8443");
}

#[test]
fn resource_origin_brackets_ipv6_hosts() {
    let url = reqwest::Url::parse("https://[::1]:8443/v1/mcp").unwrap();
    assert_eq!(resource_origin(&url).unwrap(), "https://[::1]:8443");
}

/// Egress that fails the test if it is ever asked to send — used to prove
/// that a blocked URL is rejected by `egress_oauth_json` before any request
/// leaves the boundary (EVE-623).
struct NeverSendEgress;

#[async_trait::async_trait]
impl EgressService for NeverSendEgress {
    async fn send(
        &self,
        request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
        panic!(
            "egress_oauth_json must not send blocked URL: {}",
            request.url
        );
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("send_stream should not be called");
    }
}

#[tokio::test]
async fn oauth_egress_blocks_private_ip_literal_before_send() {
    // A token/registration/discovery endpoint that resolves to a private or
    // metadata address must be refused at the egress boundary, not fetched.
    let egress = NeverSendEgress;
    for url in [
        "http://127.0.0.1/token",
        "http://169.254.169.254/latest/meta-data/",
        "http://10.0.0.1/oauth/register",
    ] {
        let result: Result<serde_json::Value, _> =
            egress_oauth_json(&egress, "GET", url, &[], Vec::new()).await;
        let (status, _msg) = result.expect_err("blocked URL must error");
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "url {url} should be blocked"
        );
    }
}

#[tokio::test]
async fn oauth_egress_blocks_localhost_hostname_before_send() {
    let egress = NeverSendEgress;
    let result: Result<serde_json::Value, _> = egress_oauth_json(
        &egress,
        "POST",
        "http://localhost:9000/token",
        &[],
        Vec::new(),
    )
    .await;
    let (status, _) = result.expect_err("localhost must be blocked");
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
