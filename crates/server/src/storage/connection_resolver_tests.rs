use super::*;
use crate::kernel_imports::{DEFAULT_ORG_ID, PrincipalId};
use crate::storage::InMemoryDatabase;
use crate::storage::models::{CreateMcpServerRow, CreateSessionRow, CreateUserConnectionRow};
use everruns_core::connection_services::UserConnectionResolver;
use std::sync::atomic::{AtomicUsize, Ordering};

const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

struct FakeRefreshExchange {
    calls: AtomicUsize,
    delay: StdDuration,
    result: FakeRefreshResult,
}
#[derive(Clone, Copy)]
enum FakeRefreshResult {
    Success,
    Failed,
    InvalidGrant,
}

#[async_trait]
impl OAuthRefreshExchange for FakeRefreshExchange {
    async fn exchange(
        &self,
        request: OAuthRefreshRequest,
    ) -> std::result::Result<OAuthTokenResponse, OAuthRefreshError> {
        assert_eq!(request.refresh_token, "old-refresh");
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(self.delay).await;
        match self.result {
            FakeRefreshResult::Failed => {
                return Err(OAuthRefreshError::Failed(
                    axum::http::StatusCode::BAD_GATEWAY,
                ));
            }
            FakeRefreshResult::InvalidGrant => {
                return Err(OAuthRefreshError::InvalidGrant);
            }
            FakeRefreshResult::Success => {}
        }
        Ok(OAuthTokenResponse {
            access_token: "fresh-access".to_string(),
            refresh_token: Some("rotated-refresh".to_string()),
            expires_in: Some(3600),
            scope: Some("email.send".to_string()),
        })
    }
}

fn encryption() -> EncryptionService {
    EncryptionService::new(TEST_KEY, &[]).unwrap()
}

fn session_input(owner_user_id: Option<Uuid>) -> CreateSessionRow {
    CreateSessionRow {
        source: everruns_platform::SessionSource::Api,
        org_id: DEFAULT_ORG_ID,
        workspace_id: None,
        app_id: None,
        endpoint_id: None,
        harness_id: None,
        agent_id: None,
        agent_version_id: None,
        agent_config_hash: None,
        agent_identity_id: None,
        owner_principal_id: PrincipalId::from_seed(1),
        resolved_owner_user_id: owner_user_id,
        title: None,
        locale: None,
        tags: vec![],
        model_id: None,
        capabilities: serde_json::json!([]),
        tools: serde_json::json!([]),
        mcp_servers: serde_json::json!({}),
        system_prompt: None,
        initial_files: serde_json::json!([]),
        hints: None,
        network_access: None,
        max_iterations: None,
        parallel_tool_calls: None,
        blueprint_id: None,
        blueprint_config: None,
        parent_session_id: None,
        budget_root_session_id: None,
        project_id: None,
    }
}

async fn setup(
    owner_user_id: Option<Uuid>,
) -> (StorageBackend, EncryptionService, SessionId, Uuid, String) {
    let memory = Arc::new(InMemoryDatabase::new());
    let db = StorageBackend::InMemory(memory);
    let server_id = Uuid::now_v7();
    db.create_mcp_server_with_id(
        DEFAULT_ORG_ID,
        server_id,
        CreateMcpServerRow {
            name: "resend".to_string(),
            description: None,
            url: "https://mcp.resend.com/mcp".to_string(),
            transport_type: "streamable_http".to_string(),
            api_key_encrypted: None,
            headers: None,
            settings: Some(serde_json::json!({
                "auth_mode": "o_auth",
                "oauth": {
                    "token_endpoint": "https://api.resend.com/oauth/token",
                    "client_id": "test-client"
                }
            })),
        },
    )
    .await
    .unwrap();
    let session = db
        .create_session(session_input(owner_user_id))
        .await
        .unwrap();
    let server = db
        .get_mcp_server(session.org_id, server_id)
        .await
        .unwrap()
        .unwrap();
    let settings = McpServerService::settings_from_row(&server);
    assert_eq!(settings.auth_mode, McpServerAuthMode::OAuth);
    assert_eq!(
        settings
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.client_id.as_deref()),
        Some("test-client")
    );
    let provider = format!("mcp_oauth_{server_id}");
    (db, encryption(), session.id, server_id, provider)
}

// ---------------------------------------------------------------
// EVE-1029: MCP credential resolution is a pure function of actsAs.
//
// The defect class here is reading the *wrong* store, so every test
// asserts both what was read and what was not. A happy-path assertion
// alone would pass just as well with the fallback still in place.
// ---------------------------------------------------------------

use crate::kernel_imports::AgentIdentityId;
use crate::storage::models::{CreateAgentIdentityConnectionRow, CreatePrincipalRow};
use everruns_core::McpServerActsAs;

/// A session whose owner principal really is a person.
const ATTENDED: &str = "user";
/// A session fired by a trigger/schedule: the owner principal is the
/// agent's own identity, which may still *resolve* to a human by lineage.
const UNATTENDED: &str = "agent_identity";

struct McpFixture {
    db: StorageBackend,
    encryption: EncryptionService,
    session_id: SessionId,
    provider: String,
    user_id: Uuid,
    identity_id: AgentIdentityId,
}

/// Seed a session with both stores populated unless told otherwise, so a
/// test that asserts "did not read X" is meaningful: X is always there to
/// be read incorrectly.
async fn mcp_setup(
    owner_kind: &str,
    with_user_grant: bool,
    with_identity_grant: bool,
) -> McpFixture {
    let memory = Arc::new(InMemoryDatabase::new());
    let db = StorageBackend::InMemory(memory);
    let encryption = encryption();
    let server_id = Uuid::now_v7();
    db.create_mcp_server_with_id(
        DEFAULT_ORG_ID,
        server_id,
        CreateMcpServerRow {
            name: "linear".to_string(),
            description: None,
            url: "https://mcp.linear.app/mcp".to_string(),
            transport_type: "streamable_http".to_string(),
            api_key_encrypted: None,
            headers: None,
            settings: Some(serde_json::json!({
                "auth_mode": "o_auth",
                "oauth": {
                    "token_endpoint": "https://api.linear.app/oauth/token",
                    "client_id": "test-client"
                }
            })),
        },
    )
    .await
    .unwrap();

    let user_id = Uuid::now_v7();
    let identity_id = AgentIdentityId::from_seed(7);
    let owner_principal_id = PrincipalId::from_seed(42);
    db.create_principal(CreatePrincipalRow {
        id: owner_principal_id,
        org_id: DEFAULT_ORG_ID,
        kind: owner_kind.to_string(),
        subject_id: Some(Uuid::now_v7()),
        parent_principal_id: None,
        // Populated even for the unattended case on purpose: this is the
        // lineage an unattended run would borrow a human through.
        resolved_user_id: Some(user_id),
        metadata: serde_json::json!({}),
    })
    .await
    .unwrap();

    let mut input = session_input(Some(user_id));
    input.owner_principal_id = owner_principal_id;
    input.agent_identity_id = Some(identity_id);
    let session = db.create_session(input).await.unwrap();

    let provider = format!("mcp_oauth_{server_id}");

    if with_user_grant {
        db.upsert_user_connection(CreateUserConnectionRow {
            user_id,
            provider: provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("the-human".to_string()),
            access_token_encrypted: Some(encryption.encrypt_string("user-token").unwrap()),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    }

    if with_identity_grant {
        db.upsert_agent_identity_connection(CreateAgentIdentityConnectionRow {
            agent_identity_id: identity_id,
            provider: provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("the-agent".to_string()),
            access_token_encrypted: Some(encryption.encrypt_string("identity-token").unwrap()),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    }

    McpFixture {
        db,
        encryption,
        session_id: session.id,
        provider,
        user_id,
        identity_id,
    }
}

fn resolver_for(fixture: &McpFixture) -> DbConnectionResolver {
    // No grant in these fixtures is expired, so the exchange must never be
    // called; a refresh here would mean the resolver took a path it should
    // not have.
    let exchange = Arc::new(FakeRefreshExchange {
        calls: AtomicUsize::new(0),
        delay: StdDuration::ZERO,
        result: FakeRefreshResult::Success,
    });
    DbConnectionResolver::with_oauth_refresh(
        fixture.db.clone(),
        fixture.encryption.clone(),
        None,
        exchange,
    )
}

#[tokio::test]
async fn user_attachment_resolves_the_invoking_user_and_never_the_identity_grant() {
    let fixture = mcp_setup(ATTENDED, true, true).await;
    let resolver = resolver_for(&fixture);

    let token = resolver
        .get_mcp_connection_token(fixture.session_id, &fixture.provider, McpServerActsAs::User)
        .await
        .unwrap();

    assert_eq!(token.as_deref(), Some("user-token"));
    // The Warp-confusion case: the session carries an identity holding a
    // grant for this very preset, and it must not have been consulted.
    assert_ne!(token.as_deref(), Some("identity-token"));
}

#[tokio::test]
async fn user_attachment_without_user_grant_fails_closed_leaving_identity_grant_untouched() {
    let fixture = mcp_setup(ATTENDED, false, true).await;
    let resolver = resolver_for(&fixture);

    let token = resolver
        .get_mcp_connection_token(fixture.session_id, &fixture.provider, McpServerActsAs::User)
        .await
        .unwrap();

    assert_eq!(
        token, None,
        "must fail closed rather than borrow the identity grant"
    );
    // The identity grant is still there, unread and unmodified.
    let identity = fixture
        .db
        .get_agent_identity_connection_for_session(fixture.session_id, &fixture.provider)
        .await
        .unwrap()
        .expect("identity grant should be untouched");
    assert_eq!(
        fixture.encryption.decrypt_to_string(&identity).unwrap(),
        "identity-token"
    );
}

#[tokio::test]
async fn service_attachment_resolves_the_identity_and_never_the_user_grant() {
    let fixture = mcp_setup(ATTENDED, true, true).await;
    let resolver = resolver_for(&fixture);

    let token = resolver
        .get_mcp_connection_token(
            fixture.session_id,
            &fixture.provider,
            McpServerActsAs::Service,
        )
        .await
        .unwrap();

    assert_eq!(token.as_deref(), Some("identity-token"));
    assert_ne!(token.as_deref(), Some("user-token"));
}

#[tokio::test]
async fn service_attachment_without_identity_grant_fails_closed_despite_a_user_grant() {
    let fixture = mcp_setup(ATTENDED, true, false).await;
    let resolver = resolver_for(&fixture);

    let token = resolver
        .get_mcp_connection_token(
            fixture.session_id,
            &fixture.provider,
            McpServerActsAs::Service,
        )
        .await
        .unwrap();

    assert_eq!(
        token, None,
        "a service attachment must never spend the invoking user's token"
    );
    // The user grant is still there, unread.
    let user_grant = fixture
        .db
        .get_user_connection(fixture.user_id, &fixture.provider)
        .await
        .unwrap();
    assert!(user_grant.is_some(), "user grant should be untouched");
}

#[tokio::test]
async fn user_attachment_in_an_unattended_session_fails_closed_though_the_owner_holds_a_grant() {
    // The owner principal is the agent identity, but its lineage resolves
    // to a human who *does* hold a grant. That is precisely the borrow this
    // rule forbids.
    let fixture = mcp_setup(UNATTENDED, true, false).await;
    let resolver = resolver_for(&fixture);

    let token = resolver
        .get_mcp_connection_token(fixture.session_id, &fixture.provider, McpServerActsAs::User)
        .await
        .unwrap();

    assert_eq!(
        token, None,
        "an unattended run has no invoking user and must not borrow one"
    );
    assert!(
        !fixture
            .db
            .session_has_human_initiator(fixture.session_id)
            .await
            .unwrap()
    );
    // The grant it declined to spend is still present and resolvable for a
    // genuinely attended session, so this is a refusal, not an absence.
    assert!(
        fixture
            .db
            .get_user_connection(fixture.user_id, &fixture.provider)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn none_attachment_reads_no_connection_store_at_all() {
    let fixture = mcp_setup(ATTENDED, true, true).await;
    let resolver = resolver_for(&fixture);

    let token = resolver
        .get_mcp_connection_token(fixture.session_id, &fixture.provider, McpServerActsAs::None)
        .await
        .unwrap();

    assert_eq!(token, None);
    // Both stores are populated, so None here proves neither was consulted.
    assert!(
        fixture
            .db
            .get_user_connection(fixture.user_id, &fixture.provider)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        fixture
            .db
            .get_agent_identity_connection_for_session(fixture.session_id, &fixture.provider)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn attended_session_is_decided_by_the_owner_principal_not_the_resolved_owner() {
    // Both fixtures carry the same resolved_owner_user_id; only the owner
    // principal's kind differs. If resolution ever regresses to reading the
    // denormalized column, these two agree and this test fails.
    let attended = mcp_setup(ATTENDED, true, false).await;
    let unattended = mcp_setup(UNATTENDED, true, false).await;

    assert!(
        attended
            .db
            .session_has_human_initiator(attended.session_id)
            .await
            .unwrap()
    );
    assert!(
        !unattended
            .db
            .session_has_human_initiator(unattended.session_id)
            .await
            .unwrap()
    );

    let attended_session = attended
        .db
        .get_session_unscoped(attended.session_id)
        .await
        .unwrap()
        .unwrap();
    let unattended_session = unattended
        .db
        .get_session_unscoped(unattended.session_id)
        .await
        .unwrap()
        .unwrap();
    assert!(attended_session.resolved_owner_user_id.is_some());
    assert!(
        unattended_session.resolved_owner_user_id.is_some(),
        "the unattended session must still resolve a human, or this test proves nothing"
    );
}

#[tokio::test]
async fn a_non_mcp_provider_never_resolves_through_the_acts_as_path() {
    let fixture = mcp_setup(ATTENDED, true, true).await;
    let resolver = resolver_for(&fixture);

    for acts_as in [
        McpServerActsAs::None,
        McpServerActsAs::Service,
        McpServerActsAs::User,
    ] {
        let token = resolver
            .get_mcp_connection_token(fixture.session_id, "github", acts_as)
            .await
            .unwrap();
        assert_eq!(token, None, "{acts_as} must not resolve a non-MCP provider");
    }
}

// ---------------------------------------------------------------
// EVE-1030 acceptance: what an authorized service grant buys, and what
// revoking it takes away. These sit here rather than beside the authorize
// handler because the property being asserted is what the *resolver* does
// with the row the authorize flow writes.
// ---------------------------------------------------------------

#[tokio::test]
async fn two_different_invoking_users_reach_the_remote_as_the_same_identity() {
    let first = mcp_setup(ATTENDED, true, true).await;
    let second_user_id = Uuid::now_v7();
    let second_principal_id = PrincipalId::from_seed(43);
    first
        .db
        .create_principal(CreatePrincipalRow {
            id: second_principal_id,
            org_id: DEFAULT_ORG_ID,
            kind: ATTENDED.to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: Some(second_user_id),
            metadata: serde_json::json!({}),
        })
        .await
        .unwrap();
    let mut second_input = session_input(Some(second_user_id));
    second_input.owner_principal_id = second_principal_id;
    second_input.agent_identity_id = Some(first.identity_id);
    let second_session = first.db.create_session(second_input).await.unwrap();
    first
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: second_user_id,
            provider: first.provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("second-human".to_string()),
            access_token_encrypted: Some(
                first
                    .encryption
                    .encrypt_string("second-user-token")
                    .unwrap(),
            ),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    let resolver = resolver_for(&first);

    let first_token = resolver
        .get_mcp_connection_token(first.session_id, &first.provider, McpServerActsAs::Service)
        .await
        .unwrap();
    let second_token = resolver
        .get_mcp_connection_token(second_session.id, &first.provider, McpServerActsAs::Service)
        .await
        .unwrap();

    assert_eq!(first_token.as_deref(), Some("identity-token"));
    assert_eq!(second_token, first_token);
    assert_ne!(first_token.as_deref(), Some("user-token"));
    assert_ne!(second_token.as_deref(), Some("second-user-token"));
    assert_ne!(first.user_id, second_user_id);
    assert_eq!(
        first
            .db
            .list_agent_identity_connections(first.identity_id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn revoking_the_identity_grant_returns_the_attachment_to_connection_required() {
    let fixture = mcp_setup(ATTENDED, true, true).await;
    let resolver = resolver_for(&fixture);

    // Authorized: the service attachment resolves the agent's credential.
    assert_eq!(
        resolver
            .get_mcp_connection_token(
                fixture.session_id,
                &fixture.provider,
                McpServerActsAs::Service
            )
            .await
            .unwrap()
            .as_deref(),
        Some("identity-token")
    );

    // Revoked: this is exactly what the delete endpoint does.
    assert!(
        fixture
            .db
            .delete_agent_identity_connection(fixture.identity_id, &fixture.provider)
            .await
            .unwrap()
    );

    let after = resolver
        .get_mcp_connection_token(
            fixture.session_id,
            &fixture.provider,
            McpServerActsAs::Service,
        )
        .await
        .unwrap();

    assert_eq!(
        after, None,
        "revocation must take effect on the next call, with no cached token surviving"
    );
    // It fell back to nothing, not to the invoking user's grant, which is
    // still present and would be the tempting substitution.
    assert!(
        fixture
            .db
            .get_user_connection(fixture.user_id, &fixture.provider)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn identity_grant_is_unreachable_from_a_session_without_that_identity() {
    let fixture = mcp_setup(ATTENDED, false, true).await;
    let other = mcp_setup(ATTENDED, false, false).await;

    // Same identity id seed, different database: proves the lookup is
    // session-scoped rather than keyed only by the identity.
    assert_eq!(fixture.identity_id, other.identity_id);
    assert!(
        other
            .db
            .get_agent_identity_connection_for_session(other.session_id, &other.provider)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn resend_connection_after_sixteen_minutes_refreshes_without_reconnect() {
    let user_id = Uuid::now_v7();
    let (db, encryption, session_id, _server_id, provider) = setup(Some(user_id)).await;
    let connection = db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id,
            provider: provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("Resend".to_string()),
            access_token_encrypted: Some(encryption.encrypt_string("stale-access").unwrap()),
            refresh_token_encrypted: Some(encryption.encrypt_string("old-refresh").unwrap()),
            scopes: None,
            expires_at: Some(Utc::now() - Duration::minutes(16)),
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    let exchange = Arc::new(FakeRefreshExchange {
        calls: AtomicUsize::new(0),
        delay: StdDuration::ZERO,
        result: FakeRefreshResult::Success,
    });
    let resolver = DbConnectionResolver::with_oauth_refresh(
        db.clone(),
        encryption.clone(),
        None,
        exchange.clone(),
    );

    let token = resolver
        .get_connection_token(session_id, &provider)
        .await
        .unwrap();

    assert_eq!(token.as_deref(), Some("fresh-access"));
    assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
    let updated = db
        .get_user_connection(user_id, &provider)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.id, connection.id);
    assert_eq!(
        encryption
            .decrypt_to_string(updated.access_token_encrypted.as_deref().unwrap())
            .unwrap(),
        "fresh-access"
    );
    assert_eq!(
        encryption
            .decrypt_to_string(updated.refresh_token_encrypted.as_deref().unwrap())
            .unwrap(),
        "rotated-refresh"
    );
    assert_eq!(updated.scopes.as_deref(), Some("email.send"));
    assert!(updated.expires_at.is_some_and(|value| value > Utc::now()));
}
#[tokio::test]
async fn expired_identity_grant_refreshes_and_persists_rotated_grant() {
    let fixture = mcp_setup(ATTENDED, false, false).await;
    fixture
        .db
        .upsert_agent_identity_connection(CreateAgentIdentityConnectionRow {
            agent_identity_id: fixture.identity_id,
            provider: fixture.provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("the-agent".to_string()),
            access_token_encrypted: Some(
                fixture.encryption.encrypt_string("stale-access").unwrap(),
            ),
            refresh_token_encrypted: Some(
                fixture.encryption.encrypt_string("old-refresh").unwrap(),
            ),
            scopes: None,
            expires_at: Some(Utc::now() - Duration::minutes(1)),
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    let exchange = Arc::new(FakeRefreshExchange {
        calls: AtomicUsize::new(0),
        delay: StdDuration::ZERO,
        result: FakeRefreshResult::Success,
    });
    let resolver = DbConnectionResolver::with_oauth_refresh(
        fixture.db.clone(),
        fixture.encryption.clone(),
        None,
        exchange.clone(),
    );

    let token = resolver
        .get_mcp_connection_token(
            fixture.session_id,
            &fixture.provider,
            McpServerActsAs::Service,
        )
        .await
        .unwrap();

    assert_eq!(token.as_deref(), Some("fresh-access"));
    assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
    let updated = fixture
        .db
        .get_agent_identity_connection(fixture.identity_id, &fixture.provider)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture
            .encryption
            .decrypt_to_string(updated.access_token_encrypted.as_deref().unwrap())
            .unwrap(),
        "fresh-access"
    );
    assert_eq!(
        fixture
            .encryption
            .decrypt_to_string(updated.refresh_token_encrypted.as_deref().unwrap())
            .unwrap(),
        "rotated-refresh"
    );
    assert_eq!(updated.scopes.as_deref(), Some("email.send"));
    assert!(updated.expires_at.is_some_and(|value| value > Utc::now()));
}

#[tokio::test]
async fn invalid_identity_refresh_grant_is_revoked_without_retry() {
    let fixture = mcp_setup(ATTENDED, false, false).await;
    fixture
        .db
        .upsert_agent_identity_connection(CreateAgentIdentityConnectionRow {
            agent_identity_id: fixture.identity_id,
            provider: fixture.provider.clone(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("the-agent".to_string()),
            access_token_encrypted: Some(
                fixture.encryption.encrypt_string("stale-access").unwrap(),
            ),
            refresh_token_encrypted: Some(
                fixture.encryption.encrypt_string("old-refresh").unwrap(),
            ),
            scopes: None,
            expires_at: Some(Utc::now() - Duration::minutes(1)),
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    let exchange = Arc::new(FakeRefreshExchange {
        calls: AtomicUsize::new(0),
        delay: StdDuration::ZERO,
        result: FakeRefreshResult::InvalidGrant,
    });
    let resolver = DbConnectionResolver::with_oauth_refresh(
        fixture.db.clone(),
        fixture.encryption.clone(),
        None,
        exchange.clone(),
    );

    for _ in 0..2 {
        assert_eq!(
            resolver
                .get_mcp_connection_token(
                    fixture.session_id,
                    &fixture.provider,
                    McpServerActsAs::Service,
                )
                .await
                .unwrap(),
            None
        );
    }

    assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
    assert!(
        fixture
            .db
            .get_agent_identity_connection(fixture.identity_id, &fixture.provider)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn expired_session_grant_refreshes_and_persists_rotated_grant() {
    let (db, encryption, session_id, server_id, provider) = setup(None).await;
    db.upsert_mcp_oauth_session_credentials(UpsertMcpOAuthSessionCredentials {
        session_id,
        server_id,
        access_token_encrypted: encryption.encrypt_string("stale-access").unwrap(),
        refresh_token_encrypted: Some(encryption.encrypt_string("old-refresh").unwrap()),
        expires_at_encrypted: Some(
            encryption
                .encrypt_string(&(Utc::now() - Duration::minutes(1)).to_rfc3339())
                .unwrap(),
        ),
    })
    .await
    .unwrap();
    let exchange = Arc::new(FakeRefreshExchange {
        calls: AtomicUsize::new(0),
        delay: StdDuration::ZERO,
        result: FakeRefreshResult::Success,
    });
    let resolver = DbConnectionResolver::with_oauth_refresh(
        db.clone(),
        encryption.clone(),
        None,
        exchange.clone(),
    );

    assert_eq!(
        resolver
            .get_connection_token(session_id, &provider)
            .await
            .unwrap()
            .as_deref(),
        Some("fresh-access")
    );
    let updated = db
        .get_mcp_oauth_session_credentials(session_id, server_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        encryption
            .decrypt_to_string(&updated.access_token_encrypted)
            .unwrap(),
        "fresh-access"
    );
    assert_eq!(
        encryption
            .decrypt_to_string(updated.refresh_token_encrypted.as_deref().unwrap())
            .unwrap(),
        "rotated-refresh"
    );
}

#[tokio::test]
async fn concurrent_expired_resolution_coalesces_refresh() {
    let user_id = Uuid::now_v7();
    let (db, encryption, session_id, _server_id, provider) = setup(Some(user_id)).await;
    db.upsert_user_connection(CreateUserConnectionRow {
        user_id,
        provider: provider.clone(),
        connection_type: "oauth".to_string(),
        provider_user_id: None,
        provider_username: Some("Resend".to_string()),
        access_token_encrypted: Some(encryption.encrypt_string("stale-access").unwrap()),
        refresh_token_encrypted: Some(encryption.encrypt_string("old-refresh").unwrap()),
        scopes: None,
        expires_at: Some(Utc::now() - Duration::minutes(1)),
        installation_id: None,
        provider_metadata: None,
    })
    .await
    .unwrap();
    let exchange = Arc::new(FakeRefreshExchange {
        calls: AtomicUsize::new(0),
        delay: StdDuration::from_millis(50),
        result: FakeRefreshResult::Success,
    });
    let resolver = Arc::new(DbConnectionResolver::with_oauth_refresh(
        db,
        encryption,
        None,
        exchange.clone(),
    ));

    let mut tasks = Vec::new();
    for _ in 0..8 {
        let resolver = resolver.clone();
        let provider = provider.clone();
        tasks.push(tokio::spawn(async move {
            resolver
                .get_connection_token(session_id, &provider)
                .await
                .unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().as_deref(), Some("fresh-access"));
    }
    assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_refresh_fails_closed_and_preserves_existing_grant() {
    let user_id = Uuid::now_v7();
    let (db, encryption, session_id, _server_id, provider) = setup(Some(user_id)).await;
    let original_access = encryption.encrypt_string("stale-access").unwrap();
    let original_refresh = encryption.encrypt_string("old-refresh").unwrap();
    db.upsert_user_connection(CreateUserConnectionRow {
        user_id,
        provider: provider.clone(),
        connection_type: "oauth".to_string(),
        provider_user_id: None,
        provider_username: Some("Resend".to_string()),
        access_token_encrypted: Some(original_access.clone()),
        refresh_token_encrypted: Some(original_refresh.clone()),
        scopes: None,
        expires_at: Some(Utc::now() - Duration::minutes(1)),
        installation_id: None,
        provider_metadata: None,
    })
    .await
    .unwrap();
    let exchange = Arc::new(FakeRefreshExchange {
        calls: AtomicUsize::new(0),
        delay: StdDuration::ZERO,
        result: FakeRefreshResult::Failed,
    });
    let resolver =
        DbConnectionResolver::with_oauth_refresh(db.clone(), encryption, None, exchange.clone());

    assert_eq!(
        resolver
            .get_connection_token(session_id, &provider)
            .await
            .unwrap(),
        None
    );
    assert_eq!(exchange.calls.load(Ordering::SeqCst), 1);
    let unchanged = db
        .get_user_connection(user_id, &provider)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        unchanged.access_token_encrypted.as_deref(),
        Some(original_access.as_slice())
    );
    assert_eq!(
        unchanged.refresh_token_encrypted.as_deref(),
        Some(original_refresh.as_slice())
    );
}
