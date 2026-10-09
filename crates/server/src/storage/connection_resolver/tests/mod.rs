use super::*;
use crate::kernel_imports::{DEFAULT_ORG_ID, PrincipalId};
use crate::storage::{CreateMcpServerRow, CreateSessionRow, CreateUserConnectionRow};
use everruns_core::connection_services::UserConnectionResolver;
use std::sync::atomic::{AtomicUsize, Ordering};

const INPUT_MESSAGE: Uuid = Uuid::from_u128(71);
const TEST_KEY: &str = "kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

mod sandbox;

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
        playground_user_id: None,
        source: crate::records::SessionSource::Api,
        org_id: DEFAULT_ORG_ID,
        workspace_id: None,
        app_id: None,
        channel_id: None,
        trigger_id: None,
        harness_id: None,
        agent_id: None,
        agent_revision: None,
        virtual_user_id: None,
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
    }
}

async fn setup(
    owner_user_id: Option<Uuid>,
) -> (StorageBackend, EncryptionService, SessionId, Uuid, String) {
    let db = StorageBackend::test_database();
    let server_id = Uuid::now_v7();
    db.create_mcp_server_with_id(
        DEFAULT_ORG_ID,
        server_id,
        CreateMcpServerRow {
            name: "resend".to_string(),
            description: None,
            url: "https://mcp.resend.com/mcp".to_string(),
            transport_type: "http".to_string(),
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
    if let Some(id) = owner_user_id {
        db.create_test_user(id).await;
        seed_runtime_user(&db, VirtualUserId::from_uuid(id), "end_user").await;
    }
    let session = db
        .create_session(session_input(owner_user_id))
        .await
        .unwrap();
    db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        session.id,
        INPUT_MESSAGE,
        owner_user_id.map(VirtualUserId::from_uuid),
        None,
        None,
    )
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

use crate::kernel_imports::VirtualUserId;
use crate::storage::{CreatePrincipalRow, CreateVirtualUserConnectionRow};
use everruns_core::McpServerActsAs;

/// A session whose owner principal really is a person.
const ATTENDED: &str = "user";
/// A session fired by a trigger/schedule: the owner principal is the
/// agent's own identity, which may still *resolve* to a human by lineage.
const UNATTENDED: &str = "virtual_user";

struct McpFixture {
    db: StorageBackend,
    encryption: EncryptionService,
    session_id: SessionId,
    provider: String,
    user_id: Uuid,
    identity_id: VirtualUserId,
    agent_id: everruns_contracts::typed_id::AgentId,
}

/// Seed a session with both stores populated unless told otherwise, so a
/// test that asserts "did not read X" is meaningful: X is always there to
/// be read incorrectly.
async fn mcp_setup(
    owner_kind: &str,
    with_user_grant: bool,
    with_identity_grant: bool,
) -> McpFixture {
    let db = StorageBackend::test_database();
    let encryption = encryption();
    let server_id = Uuid::now_v7();
    db.create_mcp_server_with_id(
        DEFAULT_ORG_ID,
        server_id,
        CreateMcpServerRow {
            name: "linear".to_string(),
            description: None,
            url: "https://mcp.linear.app/mcp".to_string(),
            transport_type: "http".to_string(),
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

    let user_id = db.create_test_user(Uuid::now_v7()).await;
    let identity_id = VirtualUserId::from_seed(7);
    seed_runtime_user(&db, VirtualUserId::from_uuid(user_id), "end_user").await;
    seed_runtime_user(&db, identity_id, "service").await;
    let agent = db
        .create_agent(
            DEFAULT_ORG_ID,
            crate::storage::CreateAgentRow {
                public_id: everruns_contracts::typed_id::AgentId::new().to_string(),
                name: "Responder".into(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: "".into(),
                default_model_id: None,
                harness_id: everruns_contracts::typed_id::HarnessId::from_seed(1),
                tags: vec![],
                initial_files: serde_json::json!([]),
                tools: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                environments: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    db.set_virtual_user_id(DEFAULT_ORG_ID, agent.id, identity_id)
        .await
        .unwrap();
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
    input.virtual_user_id = Some(identity_id);
    input.agent_id = Some(agent.id);
    let session = db.create_session(input).await.unwrap();
    db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        session.id,
        INPUT_MESSAGE,
        (owner_kind == ATTENDED).then_some(VirtualUserId::from_uuid(user_id)),
        None,
        Some(agent.id.uuid()),
    )
    .await
    .unwrap();

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
        db.upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: identity_id,
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
        agent_id: agent.id,
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
    .bound_to_input_message(INPUT_MESSAGE)
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
        .get_virtual_user_connection_for_session(fixture.session_id, &fixture.provider)
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
    // The owner principal is the virtual user, but its lineage resolves
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
            .get_virtual_user_connection_for_session(fixture.session_id, &fixture.provider)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn runtime_subject_is_decided_by_the_invocation_not_the_resolved_owner() {
    // Both fixtures carry the same resolved_owner_user_id; only the owner
    // principal's kind differs. If resolution ever regresses to reading the
    // denormalized column, these two agree and this test fails.
    let attended = mcp_setup(ATTENDED, true, false).await;
    let unattended = mcp_setup(UNATTENDED, true, false).await;

    assert!(
        attended
            .db
            .runtime_invocation_has_subject(attended.session_id, INPUT_MESSAGE)
            .await
            .unwrap()
    );
    assert!(
        !unattended
            .db
            .runtime_invocation_has_subject(unattended.session_id, INPUT_MESSAGE)
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
    let second_user_id = first.db.create_test_user(Uuid::now_v7()).await;
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
    second_input.virtual_user_id = Some(first.identity_id);
    second_input.agent_id = Some(first.agent_id);
    let second_session = first.db.create_session(second_input).await.unwrap();
    seed_runtime_user(
        &first.db,
        VirtualUserId::from_uuid(second_user_id),
        "end_user",
    )
    .await;
    let second_message = Uuid::new_v4();
    first
        .db
        .record_runtime_invocation(
            DEFAULT_ORG_ID,
            second_session.id,
            second_message,
            Some(VirtualUserId::from_uuid(second_user_id)),
            None,
            Some(first.agent_id.uuid()),
        )
        .await
        .unwrap();
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
        .bound_to_input_message(second_message)
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
            .list_virtual_user_connections(first.identity_id)
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
            .delete_virtual_user_connection(fixture.identity_id, &fixture.provider)
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
            .get_virtual_user_connection_for_session(other.session_id, &other.provider)
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
    )
    .bound_to_input_message(INPUT_MESSAGE);

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
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: fixture.identity_id,
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
    )
    .bound_to_input_message(INPUT_MESSAGE);

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
        .get_virtual_user_connection(fixture.identity_id, &fixture.provider)
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
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: fixture.identity_id,
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
    )
    .bound_to_input_message(INPUT_MESSAGE);

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
            .get_virtual_user_connection(fixture.identity_id, &fixture.provider)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn expired_session_grant_refreshes_and_persists_rotated_grant() {
    let (db, encryption, session_id, server_id, provider) = setup(Some(Uuid::from_u128(77))).await;
    db.upsert_mcp_oauth_session_credentials(UpsertMcpOAuthSessionCredentials {
        virtual_user_id: Some(VirtualUserId::from_uuid(Uuid::from_u128(77))),
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
    )
    .bound_to_input_message(INPUT_MESSAGE);

    assert_eq!(
        resolver
            .get_mcp_connection_token(session_id, &provider, McpServerActsAs::User)
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
    let resolver = Arc::new(
        DbConnectionResolver::with_oauth_refresh(db, encryption, None, exchange.clone())
            .bound_to_input_message(INPUT_MESSAGE),
    );

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
        DbConnectionResolver::with_oauth_refresh(db.clone(), encryption, None, exchange.clone())
            .bound_to_input_message(INPUT_MESSAGE);

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

// ---------------------------------------------------------------------------
// Per-agent GitHub App: the identity's own App mints the token
// ---------------------------------------------------------------------------

#[tokio::test]
async fn github_token_is_minted_from_the_identitys_own_app() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let github = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/app/installations/4242/access_tokens"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({ "token": "ghs_agent" })),
        )
        .expect(1)
        .mount(&github)
        .await;

    let fixture = mcp_setup(UNATTENDED, false, false).await;
    let db = fixture.db.clone();
    let encryption = fixture.encryption.clone();
    let identity_id = fixture.identity_id;
    let pem = std::fs::read_to_string(format!(
        "{}/tests/fixtures/test-server-key.pem",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    db.create_github_app(crate::storage::CreateGitHubAppRow {
        id: Uuid::now_v7(),
        org_id: DEFAULT_ORG_ID,
        virtual_user_id: identity_id,
        app_id: 99,
        slug: "pr-bot".to_string(),
        name: "pr-bot".to_string(),
        html_url: "https://github.com/apps/pr-bot".to_string(),
        owner_login: None,
        client_id: None,
        client_secret_encrypted: None,
        private_key_encrypted: encryption.encrypt_string(&pem).unwrap(),
        webhook_secret_encrypted: None,
        created_by_user_id: None,
    })
    .await
    .unwrap();
    db.upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
        virtual_user_id: identity_id,
        provider: "github".to_string(),
        connection_type: "github_app".to_string(),
        provider_user_id: None,
        provider_username: Some("acme".to_string()),
        access_token_encrypted: None,
        refresh_token_encrypted: None,
        scopes: None,
        expires_at: None,
        installation_id: Some(4242),
        provider_metadata: None,
    })
    .await
    .unwrap();
    // Session configuration is not the responder's authority.
    let unrelated = VirtualUserId::from_seed(99);
    seed_runtime_user(&db, unrelated, "service").await;
    let mut input = session_input(None);
    input.virtual_user_id = Some(unrelated);
    let session = db.create_session(input).await.unwrap();
    let message_id = Uuid::new_v4();
    db.record_runtime_invocation(
        DEFAULT_ORG_ID,
        session.id,
        message_id,
        None,
        None,
        Some(fixture.agent_id.uuid()),
    )
    .await
    .unwrap();

    let exchange = Arc::new(FakeRefreshExchange {
        calls: AtomicUsize::new(0),
        delay: StdDuration::ZERO,
        result: FakeRefreshResult::Success,
    });
    // A deployment-wide App is configured too: the agent's own App must win.
    let global = GitHubAppTokenMinter::new("1".to_string(), "not-a-key".to_string());
    let resolver =
        DbConnectionResolver::with_oauth_refresh(db.clone(), encryption, Some(global), exchange)
            .with_github_apps_api(crate::github_apps::GitHubAppApi::new(
                crate::github_apps::GitHubEndpoints {
                    api_url: github.uri(),
                    web_url: "https://github.com".to_string(),
                },
            ));

    let token = resolver
        .bound_to_input_message(message_id)
        .get_connection_token(session.id, "github")
        .await
        .unwrap();
    assert_eq!(token.as_deref(), Some("ghs_agent"));

    // A session without that identity falls through to the other paths.
    let other = db.create_session(session_input(None)).await.unwrap();
    assert_eq!(
        resolver
            .get_connection_token(other.id, "github")
            .await
            .unwrap(),
        None
    );
}

async fn seed_runtime_user(db: &StorageBackend, id: VirtualUserId, usage: &str) {
    db.create_virtual_user(crate::storage::CreateVirtualUserRow {
        org_id: DEFAULT_ORG_ID,
        id,
        usage: usage.into(),
        name: "Runtime user".into(),
        description: None,
        avatar_url: None,
        locale: None,
        timezone: None,
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn playground_and_delegated_runs_never_resolve_private_user_grants() {
    let fixture = mcp_setup(ATTENDED, true, true).await;
    let subject = fixture
        .db
        .runtime_invocation_subject(fixture.session_id, INPUT_MESSAGE)
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE sessions SET source = 'playground', playground_user_id = $2 WHERE id = $1")
        .bind(fixture.session_id)
        .bind(subject)
        .execute(fixture.db.database().pool())
        .await
        .unwrap();
    let resolver = resolver_for(&fixture);
    assert_eq!(
        resolver
            .get_mcp_connection_token(fixture.session_id, &fixture.provider, McpServerActsAs::User)
            .await
            .unwrap(),
        None
    );
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
    let child = fixture
        .db
        .create_session(CreateSessionRow {
            org_id: DEFAULT_ORG_ID,
            parent_session_id: Some(fixture.session_id),
            source: crate::records::SessionSource::Subagent,
            agent_id: Some(fixture.agent_id),
            virtual_user_id: Some(fixture.identity_id),
            owner_principal_id: PrincipalId::from_seed(1),
            ..Default::default()
        })
        .await
        .unwrap();
    let input = Uuid::now_v7();
    fixture
        .db
        .record_runtime_invocation(
            DEFAULT_ORG_ID,
            child.id,
            input,
            Some(subject),
            None,
            Some(fixture.agent_id.uuid()),
        )
        .await
        .unwrap();
    assert!(fixture.db.is_playground_session(child.id).await.unwrap());
    assert_eq!(
        resolver
            .bound_to_input_message(input)
            .get_mcp_connection_token(child.id, &fixture.provider, McpServerActsAs::User)
            .await
            .unwrap(),
        None
    );
}

mod user_or_service;

async fn agentmail_grant(fixture: &McpFixture, service: bool, key: &str) {
    let token = Some(fixture.encryption.encrypt_string(key).unwrap());
    let metadata = Some(serde_json::json!({ "inbox_id": format!("{key}@agentmail.to") }));
    if service {
        fixture
            .db
            .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
                virtual_user_id: fixture.identity_id,
                provider: "agentmail".into(),
                connection_type: "api_key".into(),
                provider_user_id: None,
                provider_username: None,
                access_token_encrypted: token,
                refresh_token_encrypted: None,
                scopes: None,
                expires_at: None,
                installation_id: None,
                provider_metadata: metadata,
            })
            .await
            .unwrap();
    } else {
        fixture
            .db
            .upsert_user_connection(CreateUserConnectionRow {
                user_id: fixture.user_id,
                provider: "agentmail".into(),
                connection_type: "api_key".into(),
                provider_user_id: None,
                provider_username: None,
                access_token_encrypted: token,
                refresh_token_encrypted: None,
                scopes: None,
                expires_at: None,
                installation_id: None,
                provider_metadata: metadata,
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn a_service_api_key_comes_from_the_agent_and_never_from_the_invoking_user() {
    let fixture = mcp_setup(ATTENDED, false, false).await;
    agentmail_grant(&fixture, false, "user-key").await;
    let resolver = resolver_for(&fixture);
    assert!(
        resolver
            .get_service_api_key_connection(fixture.session_id, "agentmail")
            .await
            .unwrap()
            .is_none(),
        "the invoking user's key must never stand in for the agent's"
    );

    agentmail_grant(&fixture, true, "agent-key").await;
    let connection = resolver
        .get_service_api_key_connection(fixture.session_id, "agentmail")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(connection.api_key, "agent-key");
    assert_eq!(
        connection.metadata,
        Some(serde_json::json!({ "inbox_id": "agent-key@agentmail.to" }))
    );
}
