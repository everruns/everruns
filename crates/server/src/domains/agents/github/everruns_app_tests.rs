// An agent's choice of the shared Everruns App (D3): offered from the org's
// admin links, written only for an agent without its own App, by a caller
// who manages agents and identities, and only for a link of the same org.

use super::tests::{agent_with_app, ctx_for, disconnect, seed_agent, status};
use super::*;
use crate::storage::{CreateOrganizationConnectionRow, CreateOrganizationRow};
use everruns_core::{Caller, DEFAULT_ORG_ID, OrgRole, Permission, PermissionResolver};
use std::sync::Arc;
use wiremock::MockServer;

const INSTALLATION: i64 = 9200;

async fn link(db: &StorageBackend, org_id: i64, installation_id: i64) -> Uuid {
    db.create_organization_connection(CreateOrganizationConnectionRow {
        org_id,
        name: "acme".to_string(),
        provider: GITHUB_PROVIDER.to_string(),
        connection_type: Some(EVERRUNS_APP_CONNECTION_TYPE.to_string()),
        provider_user_id: Some("77".to_string()),
        provider_username: Some("acme".to_string()),
        scopes: Some("contents: read".to_string()),
        installation_id: Some(installation_id),
        provider_metadata: Some(serde_json::json!({
            "account_type": "Organization",
            "repository_selection": "selected",
        })),
        ..Default::default()
    })
    .await
    .unwrap()
    .id
}

async fn choose(
    ctx: &Ctx,
    agent_id: &str,
    connection_id: Uuid,
) -> Result<AgentGitHubStatus, CommandError> {
    UseAgentEverrunsApp {
        agent_id: agent_id.to_string(),
        req: UseEverrunsAppRequest { connection_id },
    }
    .run(ctx)
    .await
}

#[tokio::test]
async fn an_agent_chooses_a_linked_installation_and_acts_as_the_everruns_app() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Member);
    let link_id = link(&db, DEFAULT_ORG_ID, INSTALLATION).await;
    let agent_id = seed_agent(&db).await;

    let before = status(&ctx, &agent_id).await;
    assert!(!before.connected && before.kind.is_none());
    assert_eq!(
        before.everruns_app_installations,
        vec![EverrunsAppInstallation {
            connection_id: link_id,
            account: "acme".to_string(),
            account_type: Some("Organization".to_string()),
            repository_selection: Some("selected".to_string()),
        }]
    );

    let after = choose(&ctx, &agent_id, link_id).await.unwrap();
    assert!(after.connected);
    assert!(!after.app_created, "no App of the agent's own");
    assert_eq!(after.kind.as_deref(), Some(EVERRUNS_APP_KIND));
    assert_eq!(after.bot_login.as_deref(), Some("everruns[bot]"));
    assert_eq!(after.account.as_deref(), Some("acme"));
    assert_eq!(after.repository_selection.as_deref(), Some("selected"));
    assert!(after.missing_permissions.is_empty());

    let identity_id: VirtualUserId = after.identity_id.unwrap().parse().unwrap();
    let row = db
        .get_virtual_user_connection(identity_id, GITHUB_PROVIDER)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.connection_type, EVERRUNS_APP_CONNECTION_TYPE);
    assert_eq!(row.installation_id, Some(INSTALLATION));
    assert_eq!(
        row.provider_metadata.unwrap()["org_connection_id"],
        serde_json::json!(link_id)
    );
    assert_eq!(
        db.list_everruns_app_identities(DEFAULT_ORG_ID, INSTALLATION)
            .await
            .unwrap(),
        vec![identity_id]
    );
}

#[tokio::test]
async fn a_link_of_another_org_cannot_be_chosen() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Owner);
    let other_org = db
        .create_organization(CreateOrganizationRow {
            public_id: "org_000000000000000000000000000000d4".to_string(),
            name: "Other Org".to_string(),
            created_by: None,
        })
        .await
        .unwrap()
        .org_id;
    let foreign = link(&db, other_org, INSTALLATION).await;
    let agent_id = seed_agent(&db).await;

    assert!(
        status(&ctx, &agent_id)
            .await
            .everruns_app_installations
            .is_empty(),
        "another org's links are not offered"
    );
    let err = choose(&ctx, &agent_id, foreign).await.unwrap_err();
    assert_eq!(err.status(), axum::http::StatusCode::NOT_FOUND);
    assert!(
        db.list_everruns_app_identities(other_org, INSTALLATION)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Grants every permission except one.
struct Without(Permission);

impl PermissionResolver for Without {
    fn has_permission(&self, _: &Caller, permission: &Permission) -> bool {
        permission != &self.0
    }

    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|p| self.has_permission(caller, p))
            .collect()
    }
}

#[tokio::test]
async fn choosing_needs_agent_and_identity_management() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let link_id = link(&db, DEFAULT_ORG_ID, INSTALLATION).await;
    let agent_id = seed_agent(&db).await;

    for missing in [
        Permission::OrgAgentsManage,
        Permission::OrgVirtualUsersManage,
    ] {
        let member = ctx_for(&db, &github, OrgRole::Member)
            .with_permission_resolver(Arc::new(Without(missing)));
        let err = choose(&member, &agent_id, link_id).await.unwrap_err();
        assert_eq!(err.status(), axum::http::StatusCode::FORBIDDEN, "{missing}");
    }
    assert!(
        db.list_everruns_app_identities(DEFAULT_ORG_ID, INSTALLATION)
            .await
            .unwrap()
            .is_empty(),
        "nothing was written"
    );
}

#[tokio::test]
async fn an_agent_with_its_own_app_must_disconnect_first() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Owner);
    let link_id = link(&db, DEFAULT_ORG_ID, INSTALLATION).await;
    let (agent_id, _) = agent_with_app(&ctx).await;

    let err = choose(&ctx, &agent_id, link_id).await.unwrap_err();
    assert_eq!(err.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(
        status(&ctx, &agent_id).await.kind.as_deref(),
        Some(AGENT_APP_KIND)
    );
}

#[tokio::test]
async fn disconnect_and_unlink_remove_the_choice_without_uninstalling() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Owner);
    let link_id = link(&db, DEFAULT_ORG_ID, INSTALLATION).await;
    let first = seed_agent(&db).await;
    let second = seed_agent(&db).await;
    choose(&ctx, &first, link_id).await.unwrap();
    choose(&ctx, &second, link_id).await.unwrap();

    let result = disconnect(&ctx, &first).await.unwrap();
    assert_eq!(result.outcome, GitHubDisconnectOutcome::Disconnected);
    assert!(!status(&ctx, &first).await.connected);
    assert!(
        github.received_requests().await.unwrap().is_empty(),
        "the shared installation stays installed on GitHub"
    );

    // Unlinking the org's installation takes every remaining choice with it.
    assert_eq!(
        db.unlink_everruns_app(DEFAULT_ORG_ID, link_id)
            .await
            .unwrap(),
        Some(1)
    );
    let after = status(&ctx, &second).await;
    assert!(!after.connected && after.kind.is_none());
    assert!(after.everruns_app_installations.is_empty());
}

#[tokio::test]
async fn without_a_shared_app_there_is_nothing_to_choose() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let mut ctx = ctx_for(&db, &github, OrgRole::Owner);
    ctx.github_apps.shared_app_slug = None;
    let link_id = link(&db, DEFAULT_ORG_ID, INSTALLATION).await;
    let agent_id = seed_agent(&db).await;

    assert!(
        status(&ctx, &agent_id)
            .await
            .everruns_app_installations
            .is_empty()
    );
    let err = choose(&ctx, &agent_id, link_id).await.unwrap_err();
    assert_eq!(err.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
}
