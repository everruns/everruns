//! OAuth catalog presets are checked when they are saved and on "Check again".

use super::*;
use crate::domains::common::{Command, Ctx};
use crate::domains::mcp_servers::record::{McpConnectionCheckStatus, McpServer};
use crate::domains::mcp_servers::{CheckMcpServerConnection, CreateMcpServer, UpdateMcpServerCmd};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A reachable MCP server whose sign-in service supports dynamic client
/// registration: protected-resource metadata on the server, authorization
/// server metadata and registration on a separate issuer host.
#[derive(Default)]
struct DiscoverableEgress {
    requests: AtomicUsize,
}

#[async_trait::async_trait]
impl EgressService for DiscoverableEgress {
    async fn send(&self, request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        let body = match (request.method.as_str(), request.url.as_str()) {
            ("GET", "https://8.8.8.8/.well-known/oauth-protected-resource") => {
                serde_json::json!({ "authorization_servers": ["https://1.1.1.1"] })
            }
            ("GET", "https://1.1.1.1/.well-known/oauth-authorization-server") => {
                serde_json::json!({
                    "issuer": "https://1.1.1.1",
                    "authorization_endpoint": "https://1.1.1.1/authorize",
                    "token_endpoint": "https://1.1.1.1/token",
                    "registration_endpoint": "https://1.1.1.1/register"
                })
            }
            ("POST", "https://1.1.1.1/register") => {
                serde_json::json!({ "client_id": "dyn-client" })
            }
            _ => {
                return Ok(EgressResponse {
                    status: 404,
                    headers: BTreeMap::new(),
                    body: Vec::new(),
                });
            }
        };
        Ok(EgressResponse {
            status: 200,
            headers: BTreeMap::new(),
            body: serde_json::to_vec(&body).unwrap(),
        })
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("streaming egress is not used by the connection check")
    }
}

/// The organization's egress policy refuses the issuer host only.
struct IssuerBlockedEgress(DiscoverableEgress);

#[async_trait::async_trait]
impl EgressService for IssuerBlockedEgress {
    async fn send(&self, request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        if request.url.starts_with("https://1.1.1.1/") {
            return Err(everruns_core::EgressError::NetworkAccessDenied { url: request.url });
        }
        self.0.send(request).await
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("streaming egress is not used by the connection check")
    }
}

/// Discovery answers with a server error.
struct BrokenEgress;

#[async_trait::async_trait]
impl EgressService for BrokenEgress {
    async fn send(&self, _request: EgressRequest) -> everruns_core::EgressResult<EgressResponse> {
        Ok(EgressResponse {
            status: 503,
            headers: BTreeMap::new(),
            body: b"<html>upstream secret page</html>".to_vec(),
        })
    }

    async fn send_stream(
        &self,
        _request: EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        panic!("streaming egress is not used by the connection check")
    }
}

fn connections_state(db: Arc<StorageBackend>, egress: Arc<dyn EgressService>) -> AppState {
    let encryption = Arc::new(EncryptionService::new(TEST_KEY, &[]).unwrap());
    let auth_config = AuthConfig::default();
    let auth = AuthState::builtin(auth_config.clone(), db.clone());
    let mcp_service = Arc::new(McpServerService::with_egress_service(
        db.clone(),
        Some(encryption.clone()),
        egress,
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

/// The OSS role mapping minus MCP catalog management.
struct WithoutMcpManage;

impl PermissionResolver for WithoutMcpManage {
    fn has_permission(&self, caller: &Caller, permission: &Permission) -> bool {
        *permission != Permission::OrgMcpServersManage
            && everruns_core::DefaultPermissionResolver.has_permission(caller, permission)
    }

    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|permission| self.has_permission(caller, permission))
            .collect()
    }
}

fn ctx_with(state: &AppState, role: everruns_core::OrgRole) -> Ctx {
    Ctx::minimal_for_test(
        Caller {
            org_id: everruns_core::DEFAULT_ORG_ID,
            org_public_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            user_id: Some(Uuid::now_v7()),
            role,
            is_platform_user: false,
            is_internal: false,
        },
        state.db.clone(),
        state.encryption.clone(),
    )
    .with_mcp_oauth_checker(Some(OAuthConnectionChecker::shared(state.clone())))
}

fn admin_ctx(state: &AppState) -> Ctx {
    ctx_with(state, everruns_core::OrgRole::Admin)
}

async fn create(ctx: &Ctx, auth_mode: &str) -> McpServer {
    let req = serde_json::from_value(serde_json::json!({
        "name": format!("preset_{}", Uuid::now_v7().simple()),
        "url": "https://8.8.8.8/mcp",
        "auth_mode": auth_mode,
    }))
    .unwrap();
    CreateMcpServer(req)
        .run(ctx)
        .await
        .expect("saving the preset")
}

async fn stored_settings(
    state: &AppState,
    server: &McpServer,
) -> crate::domains::mcp_servers::McpServerSettings {
    let row = state
        .db
        .get_mcp_server(everruns_core::DEFAULT_ORG_ID, server.id.uuid())
        .await
        .unwrap()
        .unwrap();
    McpServerService::settings_from_row(&row)
}

#[tokio::test]
async fn adding_an_oauth_preset_registers_it_and_reports_ready() {
    let db = Arc::new(StorageBackend::test_database());
    let state = connections_state(db, Arc::new(DiscoverableEgress::default()));

    let server = create(&admin_ctx(&state), "oauth").await;

    let check = server
        .clone()
        .connection_check
        .expect("OAuth presets carry a check");
    assert_eq!(check.status, McpConnectionCheckStatus::Ready);
    assert!(check.checked_at.is_some());
    assert_eq!(check.reason, None);
    // The registration is kept, so the first sign-in does not repeat it.
    let oauth = stored_settings(&state, &server).await.oauth.unwrap();
    assert_eq!(oauth.client_id.as_deref(), Some("dyn-client"));
    assert_eq!(
        oauth.token_endpoint.as_deref(),
        Some("https://1.1.1.1/token")
    );
}

#[tokio::test]
async fn a_blocked_sign_in_host_is_recorded_and_the_save_still_succeeds() {
    let db = Arc::new(StorageBackend::test_database());
    let state = connections_state(
        db,
        Arc::new(IssuerBlockedEgress(DiscoverableEgress::default())),
    );

    let server = create(&admin_ctx(&state), "oauth").await;

    let check = server.connection_check.clone().unwrap();
    assert_eq!(
        check.status,
        McpConnectionCheckStatus::BlockedByNetworkPolicy
    );
    // The refused host is the issuer, not the MCP server's own host.
    assert_eq!(check.host.as_deref(), Some("1.1.1.1"));
    assert!(check.reason.unwrap().contains("allowed network list"));
    assert!(
        stored_settings(&state, &server)
            .await
            .oauth
            .and_then(|oauth| oauth.client_id)
            .is_none()
    );
}

#[tokio::test]
async fn an_unreachable_sign_in_service_is_recorded_without_its_body() {
    let db = Arc::new(StorageBackend::test_database());
    let state = connections_state(db, Arc::new(BrokenEgress));

    let server = create(&admin_ctx(&state), "oauth").await;

    let check = server.connection_check.clone().unwrap();
    assert_eq!(check.status, McpConnectionCheckStatus::Unreachable);
    assert_eq!(check.host.as_deref(), Some("8.8.8.8"));
    let recorded = serde_json::to_string(&check).unwrap();
    assert!(!recorded.contains("upstream secret page"), "{recorded}");
}

#[tokio::test]
async fn presets_without_oauth_are_not_checked() {
    let db = Arc::new(StorageBackend::test_database());
    let egress = Arc::new(DiscoverableEgress::default());
    let state = connections_state(db, egress.clone());

    let server = create(&admin_ctx(&state), "none").await;

    assert!(server.connection_check.is_none());
    assert_eq!(egress.requests.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn switching_a_preset_to_oauth_checks_it() {
    let db = Arc::new(StorageBackend::test_database());
    let state = connections_state(db, Arc::new(DiscoverableEgress::default()));
    let ctx = admin_ctx(&state);
    let server = create(&ctx, "none").await;

    let updated = UpdateMcpServerCmd {
        id: server.id.to_string(),
        req: serde_json::from_value(serde_json::json!({ "auth_mode": "oauth" })).unwrap(),
    }
    .run(&ctx)
    .await
    .unwrap();

    assert_eq!(
        updated.connection_check.unwrap().status,
        McpConnectionCheckStatus::Ready
    );
}

#[tokio::test]
async fn check_again_records_a_host_allowed_since_and_needs_manage_permission() {
    let db = Arc::new(StorageBackend::test_database());
    let blocked = connections_state(
        db.clone(),
        Arc::new(IssuerBlockedEgress(DiscoverableEgress::default())),
    );
    let server = create(&admin_ctx(&blocked), "oauth").await;
    assert_eq!(
        server.connection_check.unwrap().status,
        McpConnectionCheckStatus::BlockedByNetworkPolicy
    );

    // The admin allows the host, then checks again.
    let allowed = connections_state(db, Arc::new(DiscoverableEgress::default()));
    // Same policy as editing the catalog: without manage permission, refused.
    let member = ctx_with(&allowed, everruns_core::OrgRole::Member)
        .with_permission_resolver(Arc::new(WithoutMcpManage));
    let denied = CheckMcpServerConnection {
        id: server.id.to_string(),
    }
    .run(&member)
    .await
    .expect_err("callers without manage permission cannot run the check");
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let rechecked = CheckMcpServerConnection {
        id: server.id.to_string(),
    }
    .run(&admin_ctx(&allowed))
    .await
    .unwrap();
    let check = rechecked.connection_check.unwrap();
    assert_eq!(check.status, McpConnectionCheckStatus::Ready);
    assert_eq!(check.host, None);
}

#[tokio::test]
async fn check_again_refuses_presets_without_oauth() {
    let db = Arc::new(StorageBackend::test_database());
    let state = connections_state(db, Arc::new(DiscoverableEgress::default()));
    let ctx = admin_ctx(&state);
    let server = create(&ctx, "none").await;

    let error = CheckMcpServerConnection {
        id: server.id.to_string(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert_eq!(error.status(), StatusCode::BAD_REQUEST);
}
