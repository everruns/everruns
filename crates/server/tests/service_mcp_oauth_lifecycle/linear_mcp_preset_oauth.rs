use std::collections::BTreeMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use everruns_core::OrgRole;
use everruns_platform::connector::ConnectorRegistry;
use everruns_provider::typed_id::{AgentId, HarnessId};
use everruns_server::api::user_connections::{
    AppState, OAuthAuthorizeQuery, OAuthCallbackQuery, authorize_connection,
    connection_oauth_callback,
};
use everruns_server::auth::{AuthConfig, AuthState, ResolvedOrg};
use everruns_server::domains::mcp_servers::McpServerService;
use everruns_server::storage::models::{CreateAgentRow, CreateMcpServerRow};
use everruns_server::storage::{EncryptionService, StorageBackend};
use everruns_test_support::MockMcpOAuthServer;
use serde::Deserialize;
use uuid::Uuid;

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

#[derive(Deserialize)]
struct PendingState {
    state: String,
}

fn resolved_org(user_id: Uuid) -> ResolvedOrg {
    ResolvedOrg {
        org_id: everruns_core::DEFAULT_ORG_ID,
        project_id: everruns_core::DEFAULT_PROJECT_ID,
        public_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
        name: "Default Organization".to_string(),
        user_id: Some(user_id),
        role: OrgRole::Owner,
        is_platform_user: false,
        feature_flags: everruns_platform::FeatureFlags::current(),
    }
}

fn pending_state(jar: &CookieJar, provider: &str) -> PendingState {
    let cookie = jar
        .get(&format!("oauth_connection_state_{provider}"))
        .expect("OAuth state cookie");
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(cookie.value()).unwrap()).unwrap()
}

async fn fixture() -> (AppState, ResolvedOrg, Uuid, String, MockMcpOAuthServer) {
    let db = Arc::new(StorageBackend::in_memory());
    let encryption = Arc::new(EncryptionService::new(TEST_KEY, &[]).unwrap());
    let mock = MockMcpOAuthServer::default();
    let auth_config = AuthConfig::default();
    let state = AppState::new(
        db.clone(),
        Some(encryption.clone()),
        AuthState::builtin(auth_config.clone(), db.clone()),
        auth_config,
        ConnectorRegistry::new(),
        Arc::new(McpServerService::with_egress_service(
            db.clone(),
            Some(encryption),
            Arc::new(mock.clone()),
        )),
    );
    let server_id = Uuid::now_v7();
    db.create_mcp_server_with_id(
        everruns_core::DEFAULT_ORG_ID,
        server_id,
        CreateMcpServerRow {
            name: "linear".to_string(),
            description: None,
            url: mock.mcp_url(),
            transport_type: "http".to_string(),
            api_key_encrypted: None,
            headers: None,
            settings: Some(serde_json::json!({
                "auth_mode": "oauth",
                "protocol_mode": "auto",
                "oauth": {
                    "scope": "read,write",
                    "service_authorization_params": {
                        "actor": "app"
                    }
                }
            })),
        },
    )
    .await
    .unwrap();
    let agent = db
        .create_agent(
            everruns_core::DEFAULT_ORG_ID,
            CreateAgentRow {
                project_id: everruns_core::DEFAULT_PROJECT_ID,
                public_id: AgentId::new().to_string(),
                name: "linear-service-agent".to_string(),
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
    let org = resolved_org(Uuid::now_v7());

    (state, org, server_id, agent.public_id, mock)
}

async fn begin_service_oauth(
    state: AppState,
    org: ResolvedOrg,
    provider: String,
    agent_id: String,
) -> (CookieJar, axum::response::Redirect) {
    authorize_connection(
        State(state),
        org,
        CookieJar::new(),
        Path(provider),
        Query(OAuthAuthorizeQuery {
            return_to: None,
            mode: Some("identity".to_string()),
            session_id: None,
            agent_id: Some(agent_id),
            popup: None,
        }),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn linear_preset_oauth_differentiates_user_and_service_actors() {
    let (state, org, server_id, agent_id, mock) = fixture().await;
    let provider = everruns_core::mcp_oauth_provider_id_for_uuid(server_id);

    let (_user_jar, user_redirect) = authorize_connection(
        State(state.clone()),
        org.clone(),
        CookieJar::new(),
        Path(provider.clone()),
        Query(OAuthAuthorizeQuery {
            return_to: None,
            mode: None,
            session_id: None,
            agent_id: None,
            popup: None,
        }),
    )
    .await
    .unwrap();
    let user_response = user_redirect.into_response();
    let user_url = reqwest::Url::parse(
        user_response
            .headers()
            .get(axum::http::header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    )
    .unwrap();
    let user_params: BTreeMap<_, _> = user_url.query_pairs().into_owned().collect();
    assert_eq!(
        user_params.get("scope").map(String::as_str),
        Some("read,write")
    );
    assert_eq!(
        user_params.get("resource").map(String::as_str),
        Some(mock.mcp_url().as_str())
    );
    assert!(!user_params.contains_key("actor"));

    let (service_jar, service_redirect) = begin_service_oauth(
        state.clone(),
        org.clone(),
        provider.clone(),
        agent_id.clone(),
    )
    .await;
    let pending = pending_state(&service_jar, &provider);
    let service_response = service_redirect.into_response();
    let service_url = reqwest::Url::parse(
        service_response
            .headers()
            .get(axum::http::header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap(),
    )
    .unwrap();
    let service_params: BTreeMap<_, _> = service_url.query_pairs().into_owned().collect();
    assert_eq!(service_params.get("actor").map(String::as_str), Some("app"));
    assert_eq!(
        service_params.get("scope").map(String::as_str),
        Some("read,write")
    );
    assert_eq!(
        service_params.get("resource").map(String::as_str),
        Some(mock.mcp_url().as_str())
    );
    assert_eq!(
        service_params
            .get("code_challenge_method")
            .map(String::as_str),
        Some("S256")
    );

    let code = mock
        .authorize_with_resource(
            service_params.get("code_challenge").unwrap(),
            service_params.get("actor").map(String::as_str),
            service_params.get("scope").map(String::as_str),
            service_params.get("resource").map(String::as_str),
        )
        .unwrap();
    let _callback = connection_oauth_callback(
        State(state.clone()),
        org.clone(),
        service_jar,
        Path(provider.clone()),
        Query(OAuthCallbackQuery {
            code: Some(code),
            state: Some(pending.state),
            error: None,
            error_description: None,
        }),
    )
    .await
    .unwrap();

    let oauth_requests = mock.oauth_requests();
    assert!(oauth_requests.iter().any(|request| {
        request.method == "GET"
            && request
                .url
                .contains("/.well-known/oauth-protected-resource")
    }));
    assert!(oauth_requests.iter().any(|request| {
        request.method == "GET"
            && request
                .url
                .contains("/.well-known/oauth-authorization-server")
    }));
    assert!(
        oauth_requests
            .iter()
            .any(|request| request.method == "POST" && request.url.ends_with("/register"))
    );
    let token_request = oauth_requests
        .iter()
        .find(|request| request.method == "POST" && request.url.ends_with("/token"))
        .expect("authorization code should be exchanged");
    let token_form: BTreeMap<String, String> =
        serde_urlencoded::from_bytes(&token_request.body).unwrap();
    assert_eq!(
        token_form.get("resource").map(String::as_str),
        Some(mock.mcp_url().as_str())
    );
    assert!(token_form.contains_key("code_verifier"));

    let (refusal_jar, _) =
        begin_service_oauth(state.clone(), org.clone(), provider.clone(), agent_id).await;
    let refusal_state = pending_state(&refusal_jar, &provider);
    let refusal = connection_oauth_callback(
        State(state),
        org,
        refusal_jar,
        Path(provider),
        Query(OAuthCallbackQuery {
            code: None,
            state: Some(refusal_state.state),
            error: Some("invalid_scope".to_string()),
            error_description: Some("scope rejected".to_string()),
        }),
    )
    .await
    .unwrap_err();
    assert_eq!(refusal.0, axum::http::StatusCode::BAD_REQUEST);
    assert!(refusal.1.contains("invalid_scope"));
    assert!(refusal.1.contains("scope rejected"));
}
