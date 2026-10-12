// Agent GitHub commands, run the way the CLI, MCP and REST run them
// (`Command::run`: policy, then execute), against in-memory storage and a
// mock GitHub.

use super::*;
use crate::github_apps::{GitHubAppContext, GitHubEndpoints};
use crate::storage::encryption::generate_encryption_key;
use crate::storage::{
    CreateAgentRow, CreateGitHubAppRow, CreateHarnessRow, CreateVirtualUserConnectionRow,
};
use everruns_core::{Caller, DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID, OrgRole};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn caller(role: OrgRole) -> Caller {
    Caller {
        org_id: DEFAULT_ORG_ID,
        org_public_id: DEFAULT_ORG_PUBLIC_ID.to_string(),
        user_id: Some(Uuid::now_v7()),
        role,
        is_platform_user: false,
        is_internal: false,
    }
}

fn ctx_for(db: &Arc<StorageBackend>, github: &MockServer, role: OrgRole) -> Ctx {
    let encryption =
        EncryptionService::new(&generate_encryption_key("agent-github-commands"), &[]).unwrap();
    Ctx::minimal_for_test(caller(role), db.clone(), Some(Arc::new(encryption))).with_github_apps(
        GitHubAppContext {
            api: GitHubAppApi::new(GitHubEndpoints {
                api_url: github.uri(),
                web_url: "https://github.example".to_string(),
            }),
            api_base_url: "https://api.example/api".to_string(),
            frontend_url: "https://app.example".to_string(),
        },
    )
}

async fn seed_agent(db: &Arc<StorageBackend>) -> String {
    // Unique names: one test seeds two agents.
    let suffix = Uuid::now_v7().simple().to_string();
    let harness = db
        .create_harness(
            DEFAULT_ORG_ID,
            CreateHarnessRow {
                name: format!("gh-harness-{suffix}"),
                display_name: None,
                icon: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: Some(String::new()),
                parent_harness_id: None,
                default_model_id: None,
                tags: vec![],
                initial_files: serde_json::json!([]),
                mcp_servers: serde_json::json!({}),
                network_access: None,
                embedder_metadata: serde_json::json!({}),
                is_built_in: false,
            },
        )
        .await
        .unwrap();
    let public_id = crate::kernel_imports::contracts::typed_id::AgentId::new().to_string();
    db.create_agent(
        DEFAULT_ORG_ID,
        CreateAgentRow {
            public_id: public_id.clone(),
            name: format!("pr-summarizer-{suffix}"),
            display_name: Some("PR Summarizer".to_string()),
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: serde_json::json!([]),
            system_prompt: String::new(),
            default_model_id: None,
            harness_id: harness.id,
            tags: vec![],
            initial_files: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            communication: Default::default(),
            environments: None,
            is_built_in: false,
        },
    )
    .await
    .unwrap();
    public_id
}

/// Connect the agent (which gives it an identity), then store an App for that
/// identity as the manifest callback would.
async fn agent_with_app(ctx: &Ctx) -> (String, VirtualUserId) {
    let agent_id = seed_agent(&ctx.db).await;
    connect(ctx, &agent_id).await;
    let identity_id: VirtualUserId = status(ctx, &agent_id)
        .await
        .identity_id
        .unwrap()
        .parse()
        .unwrap();
    let pem = std::fs::read_to_string(format!(
        "{}/tests/fixtures/test-server-key.pem",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    let encryption = ctx.encryption.as_deref().unwrap();
    ctx.db
        .create_github_app(CreateGitHubAppRow {
            id: Uuid::now_v7(),
            org_id: DEFAULT_ORG_ID,
            virtual_user_id: identity_id,
            app_id: 555,
            slug: "pr-summarizer-1a2b3c".to_string(),
            name: "PR-Summarizer-1a2b3c".to_string(),
            html_url: "https://github.example/apps/pr-summarizer-1a2b3c".to_string(),
            owner_login: Some("acme".to_string()),
            client_id: None,
            client_secret_encrypted: None,
            private_key_encrypted: encryption.encrypt_string(&pem).unwrap(),
            webhook_secret_encrypted: None,
            created_by_user_id: None,
        })
        .await
        .unwrap();
    (agent_id, identity_id)
}

async fn install(ctx: &Ctx, identity_id: VirtualUserId, installation_id: i64) {
    ctx.db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: identity_id,
            provider: GITHUB_PROVIDER.to_string(),
            connection_type: GITHUB_APP_CONNECTION_TYPE.to_string(),
            provider_user_id: Some("7".to_string()),
            provider_username: Some("acme".to_string()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some("contents:read,pull_requests:write".to_string()),
            expires_at: None,
            installation_id: Some(installation_id),
            provider_metadata: Some(serde_json::json!({ "repository_selection": "selected" })),
        })
        .await
        .unwrap();
}

async fn status(ctx: &Ctx, agent_id: &str) -> AgentGitHubStatus {
    GetAgentGitHub {
        agent_id: agent_id.to_string(),
    }
    .run(ctx)
    .await
    .unwrap()
}

async fn connect(ctx: &Ctx, agent_id: &str) -> BeginConnectResponse {
    ConnectAgentGitHub {
        agent_id: agent_id.to_string(),
        req: BeginConnectRequest::default(),
    }
    .run(ctx)
    .await
    .unwrap()
}

async fn disconnect(ctx: &Ctx, agent_id: &str) -> Result<AgentGitHubDisconnect, CommandError> {
    DisconnectAgentGitHub {
        agent_id: agent_id.to_string(),
    }
    .run(ctx)
    .await
}

#[tokio::test]
async fn get_reports_nothing_until_connect_gives_the_agent_an_identity() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Owner);
    let agent_id = seed_agent(&db).await;

    assert_eq!(status(&ctx, &agent_id).await, AgentGitHubStatus::default());

    let BeginConnectResponse::CreateApp {
        action,
        manifest,
        next_step,
    } = connect(&ctx, &agent_id).await
    else {
        panic!("an agent without an App is asked to create one");
    };
    assert!(action.starts_with("https://github.example/settings/apps/new"));
    // The manifest names this server's public URLs, not GitHub's.
    assert!(manifest.contains("https://api.example/api"), "{manifest}");
    assert!(next_step.contains("Integrations"), "{next_step}");

    let after = status(&ctx, &agent_id).await;
    assert!(after.identity_id.is_some(), "connect creates the identity");
    assert!(!after.connected && !after.app_created);

    let missing = GetAgentGitHub {
        agent_id: "agent_missing".to_string(),
    }
    .run(&ctx)
    .await
    .unwrap_err();
    assert_eq!(missing.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn connect_answers_an_install_link_once_the_app_exists() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Owner);
    let (agent_id, _) = agent_with_app(&ctx).await;

    let BeginConnectResponse::Install { url, app_slug } = connect(&ctx, &agent_id).await else {
        panic!("an existing App is installed, not created again");
    };
    assert_eq!(app_slug, "pr-summarizer-1a2b3c");
    assert!(
        url.starts_with("https://github.example/apps/pr-summarizer-1a2b3c/installations/new"),
        "{url}"
    );
}

#[tokio::test]
async fn get_shows_the_installation() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Owner);
    let (agent_id, identity_id) = agent_with_app(&ctx).await;
    install(&ctx, identity_id, 4242).await;

    let status = status(&ctx, &agent_id).await;
    assert!(status.connected && status.app_created);
    assert_eq!(status.account.as_deref(), Some("acme"));
    assert_eq!(status.app_slug.as_deref(), Some("pr-summarizer-1a2b3c"));
    assert_eq!(status.repository_selection.as_deref(), Some("selected"));
}

#[tokio::test]
async fn disconnect_uninstalls_and_keeps_the_app() {
    let github = MockServer::start().await;
    Mock::given(method("DELETE"))
        .and(path("/app/installations/4242"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&github)
        .await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Owner);
    let (agent_id, identity_id) = agent_with_app(&ctx).await;
    install(&ctx, identity_id, 4242).await;

    let result = disconnect(&ctx, &agent_id).await.unwrap();
    assert_eq!(result.outcome, GitHubDisconnectOutcome::Disconnected);

    let after = status(&ctx, &agent_id).await;
    assert!(!after.connected, "the installation is gone");
    assert!(after.app_created, "the App stays so a reconnect reuses it");
}

#[tokio::test]
async fn disconnect_forgets_an_app_that_was_never_installed() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Owner);
    let (agent_id, _) = agent_with_app(&ctx).await;

    let result = disconnect(&ctx, &agent_id).await.unwrap();
    assert_eq!(result.outcome, GitHubDisconnectOutcome::StartedOver);
    assert!(
        github.received_requests().await.unwrap().is_empty(),
        "nothing to uninstall on GitHub"
    );
    assert!(!status(&ctx, &agent_id).await.app_created);
    assert!(
        matches!(
            connect(&ctx, &agent_id).await,
            BeginConnectResponse::CreateApp { .. }
        ),
        "the next connect creates a new App"
    );

    // Nothing left to remove.
    let again = disconnect(&ctx, &agent_id).await.unwrap_err();
    assert_eq!(again.status(), axum::http::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn disconnect_needs_a_connection_and_identity_management() {
    let github = MockServer::start().await;
    let db = Arc::new(StorageBackend::test_database());
    let ctx = ctx_for(&db, &github, OrgRole::Owner);

    // An agent that never connected has no identity, so nothing to remove.
    let fresh = seed_agent(&db).await;
    let err = disconnect(&ctx, &fresh).await.unwrap_err();
    assert_eq!(err.status(), axum::http::StatusCode::NOT_FOUND);

    let (agent_id, _) = agent_with_app(&ctx).await;
    let viewer = ctx_for(&db, &github, OrgRole::Member)
        .with_permission_resolver(Arc::new(WithoutIdentityManagement));
    let err = disconnect(&viewer, &agent_id).await.unwrap_err();
    assert_eq!(err.status(), axum::http::StatusCode::FORBIDDEN);
    assert!(status(&ctx, &agent_id).await.app_created, "nothing changed");
}

/// Can see agents but not manage their identities.
struct WithoutIdentityManagement;

impl everruns_core::PermissionResolver for WithoutIdentityManagement {
    fn has_permission(&self, _: &Caller, permission: &everruns_core::Permission) -> bool {
        permission != &everruns_core::Permission::OrgVirtualUsersManage
    }

    fn caller_permissions(&self, caller: &Caller) -> Vec<everruns_core::Permission> {
        everruns_core::Permission::ALL
            .iter()
            .copied()
            .filter(|p| self.has_permission(caller, p))
            .collect()
    }
}
