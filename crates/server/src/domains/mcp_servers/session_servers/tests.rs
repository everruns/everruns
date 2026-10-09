//! The session API's chat-only MCP servers: listed with the viewer's sign-in
//! state, removable, and confined to the caller's session and organization.

use std::sync::Arc;

use everruns_contracts::typed_id::SessionId;
use everruns_core::{
    Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, DefaultPermissionResolver, OrgRole, Permission,
    PermissionResolver,
};
use uuid::Uuid;

use super::*;
use crate::domains::common::{CommandErrorKind, dispatch};
use crate::kernel_imports::{McpServerActsAs, ScopedMcpServer};
use crate::storage::CreateVirtualUserConnectionRow;
use crate::storage::StorageBackend;

const OAUTH_PROVIDER: &str = "mcp_oauth_chat_only_test";

struct DenySessionAccess;

impl PermissionResolver for DenySessionAccess {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        permission != &Permission::OrgSessionsManage
    }

    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|permission| self.has_permission(caller, permission))
            .collect()
    }
}

struct Fixture {
    db: Arc<StorageBackend>,
    session: SessionId,
    viewer: Uuid,
}

impl Fixture {
    async fn new() -> Self {
        let db = Arc::new(StorageBackend::test_database());
        let viewer = db.create_test_user(Uuid::now_v7()).await;
        db.add_organization_member(DEFAULT_ORG_ID, viewer, "member")
            .await
            .unwrap();
        let session = db.create_test_session().await;
        let fixture = Self {
            db,
            session,
            viewer,
        };
        fixture
            .put(
                session,
                "notes",
                plain("https://notes.example.com/mcp"),
                user_mcp(),
            )
            .await;
        fixture
            .put(
                session,
                "linear",
                ScopedMcpServer {
                    url: "https://mcp.linear.example.com/mcp".into(),
                    acts_as: McpServerActsAs::User,
                    oauth_provider_id: Some(OAUTH_PROVIDER.into()),
                    ..Default::default()
                },
                user_mcp(),
            )
            .await;
        fixture
            .put(
                session,
                "docs",
                plain("https://docs.example.com/mcp"),
                SessionMcpServerSource::Ard {
                    urn: "urn:ard:docs".into(),
                },
            )
            .await;
        fixture
    }

    fn store(&self) -> crate::storage::DbSessionStorageStore {
        crate::storage::DbSessionStorageStore::new_without_encryption(self.db.database().clone())
    }

    async fn put(
        &self,
        session: SessionId,
        name: &str,
        server: ScopedMcpServer,
        source: SessionMcpServerSource,
    ) {
        everruns_core::put_session_mcp_server(
            &self.store(),
            session,
            &SessionMcpServer {
                name: name.into(),
                server,
                source,
            },
        )
        .await
        .unwrap();
    }

    async fn has(&self, session: SessionId, name: &str) -> bool {
        everruns_core::get_session_mcp_server(&self.store(), session, name)
            .await
            .is_some()
    }

    fn ctx_as(&self, org_id: i64, resolver: Arc<dyn PermissionResolver>) -> Ctx {
        Ctx::minimal(
            Caller {
                org_id,
                org_public_id: if org_id == DEFAULT_ORG_ID {
                    DEFAULT_ORG_PUBLIC_ID.to_string()
                } else {
                    everruns_core::org_public_id_from_internal(org_id)
                },
                user_id: Some(self.viewer),
                role: OrgRole::Owner,
                is_platform_user: false,
                is_internal: false,
            },
            self.db.clone(),
            None,
            resolver,
        )
    }

    fn ctx(&self) -> Ctx {
        self.ctx_as(DEFAULT_ORG_ID, Arc::new(DefaultPermissionResolver))
    }

    async fn list(
        &self,
        ctx: &Ctx,
        session: SessionId,
    ) -> Result<Vec<ChatMcpServer>, CommandError> {
        ListChatMcpServers {
            session_id: session.to_string(),
        }
        .run(ctx)
        .await
    }

    async fn remove(
        &self,
        ctx: &Ctx,
        session: SessionId,
        name: &str,
    ) -> Result<bool, CommandError> {
        RemoveChatMcpServer {
            session_id: session.to_string(),
            name: name.into(),
        }
        .run(ctx)
        .await
    }
}

fn plain(url: &str) -> ScopedMcpServer {
    ScopedMcpServer {
        url: url.into(),
        ..Default::default()
    }
}

fn user_mcp() -> SessionMcpServerSource {
    SessionMcpServerSource::UserMcp
}

#[tokio::test]
async fn lists_chat_only_servers_with_the_viewers_sign_in() {
    let fixture = Fixture::new().await;
    let ctx = fixture.ctx();

    let listed = fixture.list(&ctx, fixture.session).await.unwrap();
    // ARD attachments are not chat-only servers; sorted by name.
    let names: Vec<_> = listed.iter().map(|server| server.name.as_str()).collect();
    assert_eq!(names, ["linear", "notes"]);
    assert_eq!(listed[0].url, "https://mcp.linear.example.com/mcp");
    assert_eq!(listed[0].connection, UserMcpConnectionStatus::NotConnected);
    assert_eq!(listed[1].connection, UserMcpConnectionStatus::NotNeeded);

    // Once the viewer signs in, the same listing says so.
    let viewer = fixture
        .db
        .default_virtual_user(DEFAULT_ORG_ID, fixture.viewer)
        .await
        .unwrap();
    fixture
        .db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: viewer.id,
            provider: OAUTH_PROVIDER.into(),
            connection_type: "oauth".into(),
            provider_user_id: None,
            provider_username: None,
            access_token_encrypted: Some(b"ciphertext".to_vec()),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .unwrap();
    let listed = fixture.list(&ctx, fixture.session).await.unwrap();
    assert_eq!(listed[0].connection, UserMcpConnectionStatus::Connected);

    // The agent-facing command tree answers the same.
    let dispatched = dispatch(
        "list_session_mcp_servers",
        serde_json::json!({ "session_id": fixture.session.to_string() }),
        &ctx,
    )
    .await
    .unwrap();
    assert!(dispatched.contains("notes") && !dispatched.contains("docs.example.com"));
}

#[tokio::test]
async fn remove_drops_only_a_chat_only_server() {
    let fixture = Fixture::new().await;
    let ctx = fixture.ctx();

    assert!(
        fixture
            .remove(&ctx, fixture.session, "notes")
            .await
            .unwrap()
    );
    assert!(!fixture.has(fixture.session, "notes").await);
    let names: Vec<_> = fixture
        .list(&ctx, fixture.session)
        .await
        .unwrap()
        .into_iter()
        .map(|server| server.name)
        .collect();
    assert_eq!(names, ["linear"]);

    // Already gone, never there, or an ARD attachment: nothing removed.
    assert!(
        !fixture
            .remove(&ctx, fixture.session, "notes")
            .await
            .unwrap()
    );
    assert!(!fixture.remove(&ctx, fixture.session, "nope").await.unwrap());
    assert!(!fixture.remove(&ctx, fixture.session, "docs").await.unwrap());
    assert!(fixture.has(fixture.session, "docs").await);
}

#[tokio::test]
async fn another_session_cannot_see_or_remove_the_server() {
    let fixture = Fixture::new().await;
    let ctx = fixture.ctx();
    let other = fixture.db.create_test_session().await;

    assert!(fixture.list(&ctx, other).await.unwrap().is_empty());
    assert!(!fixture.remove(&ctx, other, "notes").await.unwrap());
    assert!(fixture.has(fixture.session, "notes").await);
}

#[tokio::test]
async fn another_organization_cannot_see_or_remove_the_server() {
    let fixture = Fixture::new().await;
    let foreign = fixture.ctx_as(DEFAULT_ORG_ID + 1, Arc::new(DefaultPermissionResolver));

    let listed = fixture
        .list(&foreign, fixture.session)
        .await
        .expect_err("a session of another org is not found");
    assert!(
        matches!(listed.kind, CommandErrorKind::NotFound(_)),
        "{listed:?}"
    );
    let removed = fixture
        .remove(&foreign, fixture.session, "notes")
        .await
        .expect_err("a session of another org is not found");
    assert!(
        matches!(removed.kind, CommandErrorKind::NotFound(_)),
        "{removed:?}"
    );
    assert!(fixture.has(fixture.session, "notes").await);
}

#[tokio::test]
async fn listing_and_removing_require_session_permissions() {
    let fixture = Fixture::new().await;
    let denied = fixture.ctx_as(DEFAULT_ORG_ID, Arc::new(DenySessionAccess));

    let listed = fixture.list(&denied, fixture.session).await.unwrap_err();
    assert!(
        matches!(listed.kind, CommandErrorKind::Forbidden(_)),
        "{listed:?}"
    );
    let removed = fixture
        .remove(&denied, fixture.session, "notes")
        .await
        .unwrap_err();
    assert!(
        matches!(removed.kind, CommandErrorKind::Forbidden(_)),
        "{removed:?}"
    );
    assert!(fixture.has(fixture.session, "notes").await);
}
