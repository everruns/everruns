use super::mcp_oauth::discover_oauth_server_metadata;
use super::*;
use crate::oauth_client::egress_oauth_json;
use crate::storage::{CreateAgentRow, CreateMcpServerRow, UpdateMcpServer};
use everruns_contracts::typed_id::{AgentId, HarnessId};
use everruns_core::{
    EgressRequest, EgressResponse, EgressService, OrgRole, Permission, PermissionResolver,
};
use std::collections::BTreeMap;
use uuid::Uuid;

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

#[test]
fn setup_return_target_stays_on_the_console_origin() {
    let fallback = "/settings/connections";
    for target in [
        "//attacker.example",
        "/\\attacker.example",
        "/path\n",
        "https://attacker.example",
    ] {
        assert_eq!(normalize_return_to(Some(target), fallback), fallback);
    }
    assert_eq!(
        normalize_return_to(Some("/virtual-users/me?tab=connections"), fallback),
        "/virtual-users/me?tab=connections"
    );
}

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
        read_only: false,
    }
}

async fn identity_oauth_fixture(configured: bool) -> (AppState, ResolvedOrg, Uuid, String, Uuid) {
    let db = Arc::new(StorageBackend::test_database());
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
            transport_type: "http".to_string(),
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
                max_output_tokens: None,
                communication: Default::default(),
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    let user_id = Uuid::now_v7();
    db.create_user_with_id(
        user_id,
        crate::storage::CreateUserRow {
            email: format!("{user_id}@example.com"),
            name: "Owner".into(),
            avatar_url: None,
            roles: vec![],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        },
    )
    .await
    .unwrap();
    db.add_organization_member(everruns_core::DEFAULT_ORG_ID, user_id, "owner")
        .await
        .unwrap();
    (
        state,
        test_org(user_id),
        server_id,
        agent.public_id,
        user_id,
    )
}

/// A callback failure past a valid state returns to `return_to` as `failed`.
fn assert_connect_failed((_, redirect): (CookieJar, Redirect)) {
    let response = redirect.into_response();
    let target = response
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(target.contains("connect_error=failed"), "{target}");
    assert!(!target.contains("connected="), "{target}");
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
        org.clone(),
        ConnectionUser {
            id: org.user_id.unwrap(),
            management_user_id: org.user_id.unwrap(),
            org_id: org.org_id,
        },
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
        .virtual_user_id
        .unwrap();

    let _ = connection_oauth_callback(
        State(state.clone()),
        Ok(org),
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
        .get_virtual_user_connection(identity_id, &provider)
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
            .virtual_user_id
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
            .virtual_user_id
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
            .virtual_user_id
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
    let identity_id: VirtualUserId = pending_state(&jar, &provider)
        .virtual_user_id
        .as_deref()
        .unwrap()
        .parse()
        .unwrap();

    let error = connection_oauth_callback(
        State(state.clone()),
        Ok(org),
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

    // A mismatched state is never trusted for a redirect.
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    assert!(
        state
            .db
            .get_virtual_user_connection(identity_id, &provider)
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
    let identity_id: VirtualUserId = pending.virtual_user_id.as_deref().unwrap().parse().unwrap();
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
        Ok(org),
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
    .expect("a valid state returns the browser to its page");
    // Past a valid state the failure returns to `return_to`; no grant follows.
    assert_connect_failed(error);
    assert!(
        state
            .db
            .get_virtual_user_connection(identity_id, &provider)
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
    let identity_id: VirtualUserId = pending.virtual_user_id.as_deref().unwrap().parse().unwrap();
    state.auth.permission_resolver = Arc::new(DenyAllResolver);

    let error = connection_oauth_callback(
        State(state.clone()),
        Ok(org),
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
    .expect("a valid state returns the browser to its page");
    // Past a valid state the failure returns to `return_to`; no grant follows.
    assert_connect_failed(error);
    assert!(
        state
            .db
            .get_virtual_user_connection(identity_id, &provider)
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
    let identity_id: VirtualUserId = pending.virtual_user_id.as_deref().unwrap().parse().unwrap();
    state.db.delete_agent(org.org_id, agent_id).await.unwrap();

    let error = connection_oauth_callback(
        State(state.clone()),
        Ok(org),
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
    .expect("a valid state returns the browser to its page");
    // Past a valid state the failure returns to `return_to`; no grant follows.
    assert_connect_failed(error);
    assert!(
        state
            .db
            .get_virtual_user_connection(identity_id, &provider)
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
fn github_setup_validates_the_canonical_pending_state() {
    let pending = PendingOAuthState {
        state: "unguessable-state".into(),
        org_id: 1,
        management_user_id: Some(Uuid::new_v4()),
        runtime_credential: None,
        provider: "github".into(),
        return_to: "/settings/connections".into(),
        mode: "virtual_user".into(),
        session_id: None,
        agent_id: None,
        virtual_user_id: Some(VirtualUserId::new().to_string()),
        popup: false,
        code_verifier: String::new(),
        github_installation_id: None,
    };
    let jar = CookieJar::new().add(Cookie::new(
        oauth_state_cookie_name("github"),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&pending).unwrap()),
    ));
    let validated =
        validate_pending_oauth_state(&jar, "github", Some("unguessable-state")).unwrap();
    assert_eq!(validated.virtual_user_id, pending.virtual_user_id);
    assert_eq!(validated.management_user_id, pending.management_user_id);
    for query in [
        None,
        Some(""),
        Some("wrong-state"),
        Some(" unguessable-state "),
    ] {
        assert_eq!(
            validate_pending_oauth_state(&jar, "github", query)
                .unwrap_err()
                .0,
            StatusCode::BAD_REQUEST
        );
    }
    assert!(
        validate_pending_oauth_state(&CookieJar::new(), "github", Some("unguessable-state"))
            .is_err()
    );
    assert!(
        validate_pending_oauth_state(&jar, "different-provider", Some("unguessable-state"))
            .is_err()
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

// EVE-1193: a valid Everruns setup state must not let one user claim a GitHub
// App installation that their GitHub account cannot access.
mod github_installation_ownership {
    use super::*;
    use crate::auth::config::GitHubConnectionConfig;
    use crate::github_apps::GitHubEndpoints;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    pub(super) const VICTIM_INSTALLATION: i64 = 4242;
    pub(super) const ATTACKER_INSTALLATION: i64 = 1717;

    pub(super) async fn github_state(github: &MockServer) -> AppState {
        let db = Arc::new(StorageBackend::test_database());
        let encryption = Arc::new(EncryptionService::new(TEST_KEY, &[]).unwrap());
        let auth_config = AuthConfig {
            github_connection: Some(GitHubConnectionConfig {
                app_id: "99".into(),
                private_key: std::fs::read_to_string(format!(
                    "{}/tests/fixtures/test-server-key.pem",
                    env!("CARGO_MANIFEST_DIR")
                ))
                .unwrap(),
                app_slug: "everruns-test".into(),
                setup_url: "https://everruns.example/api/v1/user/connections/github/callback"
                    .into(),
                client_id: Some("Iv1.test".into()),
                client_secret: Some("client-secret".into()),
                webhook_secret: None,
                endpoints: GitHubEndpoints {
                    api_url: github.uri(),
                    web_url: github.uri(),
                },
            }),
            ..AuthConfig::default()
        };
        let auth = AuthState::builtin(auth_config.clone(), db.clone());
        let mcp_service = Arc::new(McpServerService::with_egress_service(
            db.clone(),
            Some(encryption.clone()),
            Arc::new(FakeOAuthEgress),
        ));
        AppState::new(
            db,
            Some(encryption),
            auth,
            auth_config,
            ConnectorRegistry::new(),
            mcp_service,
        )
    }

    /// GitHub as seen by the App JWT (both installations exist) and by each
    /// GitHub user's token (each user can only access their own installation).
    pub(super) async fn mock_github(github: &MockServer) {
        for id in [VICTIM_INSTALLATION, ATTACKER_INSTALLATION] {
            Mock::given(method("GET"))
                .and(path(format!("/app/installations/{id}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": id,
                    "account": { "id": id * 10, "login": format!("account-{id}"), "type": "Organization" },
                    "permissions": { "contents": "read" }
                })))
                .mount(github)
                .await;
        }
        for (code, token, installation) in [
            ("attacker-code", "attacker-token", ATTACKER_INSTALLATION),
            ("owner-code", "owner-token", VICTIM_INSTALLATION),
        ] {
            Mock::given(method("POST"))
                .and(path("/login/oauth/access_token"))
                .and(body_partial_json(serde_json::json!({
                    "client_id": "Iv1.test",
                    "client_secret": "client-secret",
                    "code": code
                })))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "access_token": token,
                    "token_type": "bearer"
                })))
                .mount(github)
                .await;
            Mock::given(method("GET"))
                .and(path("/user/installations"))
                .and(header("authorization", format!("Bearer {token}").as_str()))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "total_count": 1,
                    "installations": [{ "id": installation }]
                })))
                .mount(github)
                .await;
        }
        Mock::given(method("POST"))
            .and(path("/login/oauth/access_token"))
            .and(body_partial_json(
                serde_json::json!({ "code": "expired-code" }),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "error": "bad_verification_code"
            })))
            .mount(github)
            .await;
    }

    pub(super) fn location(redirect: Redirect) -> String {
        redirect
            .into_response()
            .headers()
            .get(axum::http::header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string()
    }

    /// A member of the default org, and their own virtual user.
    async fn person(state: &AppState) -> (ResolvedOrg, Uuid) {
        let user_id = state.db.create_test_user(Uuid::now_v7()).await;
        state
            .db
            .add_organization_member(everruns_core::DEFAULT_ORG_ID, user_id, "member")
            .await
            .unwrap();
        let target = state
            .db
            .default_virtual_user(everruns_core::DEFAULT_ORG_ID, user_id)
            .await
            .unwrap()
            .id
            .uuid();
        (test_org(user_id), target)
    }

    async fn start_sign_in(
        state: &AppState,
        org: &ResolvedOrg,
        target: Uuid,
    ) -> (CookieJar, String) {
        let (jar, redirect) = github_authorize_inner(
            state.clone(),
            OAuthAuthority {
                org_id: org.org_id,
                caller: None,
                target_id: target,
                management_user_id: org.user_id,
                runtime_credential: None,
            },
            CookieJar::new(),
            None,
        )
        .await
        .unwrap();
        assert!(location(redirect).contains("/login/oauth/authorize?client_id=Iv1.test&state="));
        let setup_state = pending_state(&jar, "github").state;
        (jar, setup_state)
    }

    async fn finish(
        state: &AppState,
        org: &ResolvedOrg,
        jar: CookieJar,
        setup_state: String,
        installation_id: Option<i64>,
        code: Option<&str>,
    ) -> Result<String, (StatusCode, String)> {
        github_callback(
            State(state.clone()),
            Ok(org.clone()),
            jar,
            Query(GitHubInstallationCallbackQuery {
                installation_id,
                setup_action: installation_id.map(|_| "install".to_string()),
                state: Some(setup_state),
                code: code.map(str::to_string),
            }),
        )
        .await
        .map(|(_, redirect)| location(redirect))
    }

    /// Step 4 (D1): a person signs in as themselves; the connection holds
    /// their user token and the account GitHub reads back, no installation.
    #[tokio::test]
    async fn a_person_signs_in_and_keeps_their_own_token() {
        let github = MockServer::start().await;
        mock_github(&github).await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("authorization", "Bearer owner-token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "id": 31, "login": "octo" })),
            )
            .mount(&github)
            .await;
        let state = github_state(&github).await;
        let (org, target) = person(&state).await;

        // A rejected code stores nothing.
        let (jar, setup_state) = start_sign_in(&state, &org, target).await;
        let error = finish(&state, &org, jar, setup_state, None, Some("expired-code"))
            .await
            .unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert!(
            state
                .db
                .get_user_connection(target, "github")
                .await
                .unwrap()
                .is_none()
        );

        let (jar, setup_state) = start_sign_in(&state, &org, target).await;
        let back = finish(&state, &org, jar, setup_state, None, Some("owner-code"))
            .await
            .unwrap();
        assert!(
            back.ends_with("/settings/connections?connected=github"),
            "{back}"
        );
        let row = state
            .db
            .get_user_connection(target, "github")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.provider_username.as_deref(), Some("octo"));
        assert_eq!(row.provider_user_id.as_deref(), Some("31"));
        assert_eq!(row.installation_id, None);
        let encryption = state.encryption.as_ref().unwrap();
        assert_eq!(
            encryption
                .decrypt_to_string(row.access_token_encrypted.as_deref().unwrap())
                .unwrap(),
            "owner-token"
        );
    }

    /// Step 3b: a setup state from the older install flow cannot put an
    /// installation on a person's own connection, and an agent never signs in.
    #[tokio::test]
    async fn no_installation_lands_on_a_virtual_users_own_connection() {
        let github = MockServer::start().await;
        mock_github(&github).await;
        let state = github_state(&github).await;
        let (org, target) = person(&state).await;

        let (jar, setup_state) = start_sign_in(&state, &org, target).await;
        let error = finish(
            &state,
            &org,
            jar,
            setup_state,
            Some(VICTIM_INSTALLATION),
            Some("owner-code"),
        )
        .await
        .unwrap_err();
        assert_eq!(error.0, StatusCode::BAD_REQUEST);
        assert!(
            state
                .db
                .get_user_connection(target, "github")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .db
                .find_everruns_app_link(VICTIM_INSTALLATION)
                .await
                .unwrap()
                .is_none()
        );
    }
}

/// A user MCP server, with OAuth already discovered, owned by the fixture
/// owner's runtime account.
async fn owned_oauth_server(state: &AppState, owner: Uuid) -> Uuid {
    state
        .db
        .create_user_mcp_server(
            everruns_core::DEFAULT_ORG_ID,
            owner,
            None,
            true,
            CreateMcpServerRow {
                name: "notes".to_string(),
                description: None,
                url: "https://8.8.8.8/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(serde_json::json!({
                    "auth_mode": "oauth",
                    "oauth": {
                        "authorization_endpoint": "https://8.8.8.8/authorize",
                        "token_endpoint": "https://8.8.8.8/token",
                        "client_id": "test-client"
                    }
                })),
            },
        )
        .await
        .unwrap()
        .row
        .id
        .uuid()
}

async fn begin_user_oauth(
    state: &AppState,
    org: &ResolvedOrg,
    runtime_id: Uuid,
    server_id: Uuid,
    mode: &str,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    authorize_connection(
        State(state.clone()),
        org.clone(),
        ConnectionUser {
            id: runtime_id,
            management_user_id: org.user_id.unwrap(),
            org_id: org.org_id,
        },
        CookieJar::new(),
        Path(mcp_oauth_provider_id_for_uuid(server_id)),
        Query(OAuthAuthorizeQuery {
            return_to: None,
            mode: Some(mode.to_string()),
            session_id: None,
            agent_id: None,
            popup: None,
        }),
    )
    .await
}

#[tokio::test]
async fn user_mcp_server_oauth_writes_only_the_owners_grant() {
    let (state, org, _, agent_id, user_id) = identity_oauth_fixture(true).await;
    let owner = state
        .db
        .default_virtual_user(org.org_id, user_id)
        .await
        .unwrap()
        .id
        .uuid();
    let server_id = owned_oauth_server(&state, owner).await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);

    let (jar, _) = begin_user_oauth(&state, &org, owner, server_id, "user")
        .await
        .unwrap();
    let pending = pending_state(&jar, &provider);
    connection_oauth_callback(
        State(state.clone()),
        Ok(org.clone()),
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
    .map(|_| ())
    .unwrap();
    assert!(
        state
            .db
            .get_user_connection(owner, &provider)
            .await
            .unwrap()
            .is_some()
    );

    // An agent can never take a service grant on a person's own server.
    let error = authorize_connection(
        State(state.clone()),
        org.clone(),
        ConnectionUser {
            id: owner,
            management_user_id: user_id,
            org_id: org.org_id,
        },
        CookieJar::new(),
        Path(provider.clone()),
        Query(OAuthAuthorizeQuery {
            return_to: None,
            mode: Some("identity".to_string()),
            session_id: None,
            agent_id: Some(agent_id),
            popup: None,
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(error.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn user_mcp_server_oauth_is_hidden_from_other_people() {
    let (state, org, _, _, user_id) = identity_oauth_fixture(true).await;
    let owner = state
        .db
        .default_virtual_user(org.org_id, user_id)
        .await
        .unwrap()
        .id
        .uuid();
    let server_id = owned_oauth_server(&state, owner).await;
    let provider = mcp_oauth_provider_id_for_uuid(server_id);
    let someone_else = crate::storage::CreateVirtualUserRow {
        org_id: org.org_id,
        id: VirtualUserId::new(),
        usage: "end_user".to_string(),
        name: "Mallory".to_string(),
        description: None,
        avatar_url: None,
        locale: None,
        timezone: None,
    };
    let mallory = state
        .db
        .create_virtual_user(someone_else)
        .await
        .unwrap()
        .id
        .uuid();

    let error = begin_user_oauth(&state, &org, mallory, server_id, "user")
        .await
        .unwrap_err();
    assert_eq!(error.0, StatusCode::NOT_FOUND);

    // A state cookie re-aimed at another person cannot plant a grant either;
    // the pending-setup record refuses it first, the owner check backs it up.
    let (jar, _) = begin_user_oauth(&state, &org, owner, server_id, "user")
        .await
        .unwrap();
    let mut pending = pending_state(&jar, &provider);
    pending.virtual_user_id = Some(VirtualUserId::from_uuid(mallory).to_string());
    let forged = jar.add(Cookie::new(
        oauth_state_cookie_name(&provider),
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&pending).unwrap()),
    ));
    let error = connection_oauth_callback(
        State(state.clone()),
        Ok(org.clone()),
        forged,
        Path(provider.clone()),
        Query(OAuthCallbackQuery {
            code: Some("code".to_string()),
            state: Some(pending.state.clone()),
            error: None,
            error_description: None,
        }),
    )
    .await
    .expect("a valid state returns the browser to its page");
    assert_connect_failed(error);
    assert!(
        state
            .db
            .get_user_connection(mallory, &provider)
            .await
            .unwrap()
            .is_none()
    );
}

mod catalog_listing_flow;
mod connect_errors_flow;
mod connection_check_flow;
mod github_org_flow;
mod oauth_discovery_flow;
