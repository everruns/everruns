// GitHub tokens for one declared use (knowledge/integrations/github-model.md,
// D4): the identity comes only from `acts_as`, and the token is minted down to
// the repositories and permissions the use asked for.

use super::*;
use everruns_contracts::runtime::GitHubAccess;
use wiremock::matchers::{body_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const AGENT_INSTALLATION: i64 = 4242;
const PERSON_INSTALLATION: i64 = 777;

fn test_key() -> String {
    std::fs::read_to_string(format!(
        "{}/tests/fixtures/test-server-key.pem",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// The agent has its own App installed; the person connected the shared App
/// (`oauth`, as the setup callback stores it).
async fn github_setup(owner_kind: &str) -> (McpFixture, MockServer) {
    let fixture = mcp_setup(owner_kind, false, false).await;
    let github = MockServer::start().await;
    let scoped = serde_json::json!({
        "repositories": ["app"],
        "permissions": {"pull_requests": "read"},
    });
    for (installation, token) in [
        (AGENT_INSTALLATION, "ghs_agent"),
        (PERSON_INSTALLATION, "ghs_person"),
    ] {
        Mock::given(method("POST"))
            .and(path(format!(
                "/app/installations/{installation}/access_tokens"
            )))
            .and(body_json(&scoped))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(serde_json::json!({ "token": token })),
            )
            .mount(&github)
            .await;
    }
    fixture
        .db
        .create_github_app(crate::storage::CreateGitHubAppRow {
            id: Uuid::now_v7(),
            org_id: DEFAULT_ORG_ID,
            virtual_user_id: fixture.identity_id,
            app_id: 99,
            slug: "pr-bot".to_string(),
            name: "pr-bot".to_string(),
            html_url: "https://github.com/apps/pr-bot".to_string(),
            owner_login: None,
            client_id: None,
            client_secret_encrypted: None,
            private_key_encrypted: fixture.encryption.encrypt_string(&test_key()).unwrap(),
            webhook_secret_encrypted: None,
            created_by_user_id: None,
        })
        .await
        .unwrap();
    for (owner, connection_type, installation) in [
        (fixture.identity_id, "github_app", AGENT_INSTALLATION),
        (
            VirtualUserId::from_uuid(fixture.user_id),
            "oauth",
            PERSON_INSTALLATION,
        ),
    ] {
        fixture
            .db
            .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
                virtual_user_id: owner,
                provider: "github".to_string(),
                connection_type: connection_type.to_string(),
                provider_user_id: None,
                provider_username: Some("acme".to_string()),
                access_token_encrypted: None,
                refresh_token_encrypted: None,
                scopes: None,
                expires_at: None,
                installation_id: Some(installation),
                provider_metadata: None,
            })
            .await
            .unwrap();
    }
    (fixture, github)
}

fn github_resolver(fixture: &McpFixture, github: &MockServer) -> DbConnectionResolver {
    let shared = GitHubAppTokenMinter::new("5".to_string(), test_key());
    DbConnectionResolver::with_oauth_refresh(
        fixture.db.clone(),
        fixture.encryption.clone(),
        Some(shared),
        Arc::new(FakeRefreshExchange {
            calls: AtomicUsize::new(0),
            delay: StdDuration::ZERO,
            result: FakeRefreshResult::Success,
        }),
    )
    .with_github_apps_api(crate::github_apps::GitHubAppApi::new(
        crate::github_apps::GitHubEndpoints {
            api_url: github.uri(),
            web_url: "https://github.com".to_string(),
        },
    ))
    .bound_to_input_message(INPUT_MESSAGE)
}

fn access(acts_as: McpServerActsAs) -> GitHubAccess {
    GitHubAccess::new(acts_as, &[("pull_requests", "read")]).for_repository("acme/app")
}

async fn resolve(
    fixture: &McpFixture,
    github: &MockServer,
    acts_as: McpServerActsAs,
) -> Option<(String, McpServerActsAs)> {
    github_resolver(fixture, github)
        .get_github_token(fixture.session_id, &access(acts_as))
        .await
        .unwrap()
        .map(|credential| (credential.token, credential.acted_as))
}

/// THREAT[TM-GITHUB-006]: a person's turn does not make the agent use the
/// person's connection, and the reverse; each token is narrowed.
#[tokio::test]
async fn each_identity_answers_only_for_itself_with_a_narrowed_token() {
    let (fixture, github) = github_setup(ATTENDED).await;
    assert_eq!(
        resolve(&fixture, &github, McpServerActsAs::Service).await,
        Some(("ghs_agent".to_string(), McpServerActsAs::Service))
    );
    assert_eq!(
        resolve(&fixture, &github, McpServerActsAs::User).await,
        Some(("ghs_person".to_string(), McpServerActsAs::User))
    );
    assert_eq!(
        resolve(&fixture, &github, McpServerActsAs::UserOrService).await,
        Some(("ghs_person".to_string(), McpServerActsAs::User))
    );
    assert_eq!(
        resolve(&fixture, &github, McpServerActsAs::None).await,
        None
    );
}

#[tokio::test]
async fn an_unattended_run_has_no_person() {
    let (fixture, github) = github_setup(UNATTENDED).await;
    assert_eq!(
        resolve(&fixture, &github, McpServerActsAs::User).await,
        None
    );
    assert_eq!(
        resolve(&fixture, &github, McpServerActsAs::UserOrService).await,
        Some(("ghs_agent".to_string(), McpServerActsAs::Service))
    );
}

#[tokio::test]
async fn a_token_cannot_span_owners() {
    let (fixture, github) = github_setup(ATTENDED).await;
    let mut access = access(McpServerActsAs::Service);
    access.repositories = vec!["acme/app".to_string(), "other/app".to_string()];
    assert!(
        github_resolver(&fixture, &github)
            .get_github_token(fixture.session_id, &access)
            .await
            .is_err()
    );
}

/// A source sync acts as the source's owner, with read access to the one
/// repository it syncs, whichever App owns that owner's installation.
#[tokio::test]
async fn a_source_sync_gets_a_read_token_for_its_owner_and_repository() {
    let (fixture, github) = github_setup(UNATTENDED).await;
    for (installation, token) in [
        (AGENT_INSTALLATION, "ghs_agent_sync"),
        (PERSON_INSTALLATION, "ghs_person_sync"),
    ] {
        Mock::given(method("POST"))
            .and(path(format!(
                "/app/installations/{installation}/access_tokens"
            )))
            .and(body_json(serde_json::json!({
                "repositories": ["app"],
                "permissions": {"contents": "read"},
            })))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(serde_json::json!({ "token": token })),
            )
            .mount(&github)
            .await;
    }
    let resolver = github_resolver(&fixture, &github);
    let read = crate::domains::git_sources::github_read_access("acme/app");
    assert_eq!(
        resolver
            .get_github_token_for_owner(DEFAULT_ORG_ID, fixture.identity_id.uuid(), &read)
            .await
            .unwrap()
            .as_deref(),
        Some("ghs_agent_sync")
    );
    assert_eq!(
        resolver
            .get_github_token_for_owner(DEFAULT_ORG_ID, fixture.user_id, &read)
            .await
            .unwrap()
            .as_deref(),
        Some("ghs_person_sync")
    );
    assert_eq!(
        resolver
            .get_github_token_for_owner(DEFAULT_ORG_ID, Uuid::now_v7(), &read)
            .await
            .unwrap(),
        None
    );
}

/// An installation stored as an agent App's mints only with that App's key: a
/// person connection marked `github_app` has no App behind it, so it gets no
/// token rather than one minted by the shared App.
#[tokio::test]
async fn an_agent_app_installation_never_mints_with_the_shared_app() {
    let (fixture, github) = github_setup(ATTENDED).await;
    fixture
        .db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: VirtualUserId::from_uuid(fixture.user_id),
            provider: "github".to_string(),
            connection_type: "github_app".to_string(),
            provider_user_id: None,
            provider_username: Some("acme".to_string()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: Some(PERSON_INSTALLATION),
            provider_metadata: None,
        })
        .await
        .unwrap();
    assert_eq!(
        resolve(&fixture, &github, McpServerActsAs::User).await,
        None
    );
    assert_eq!(
        resolve(&fixture, &github, McpServerActsAs::Service).await,
        Some(("ghs_agent".to_string(), McpServerActsAs::Service))
    );
}

/// An agent that chose the org's shared App link (`everruns_app`, D3) mints
/// with the deployment's shared key, never with an App of its own, and gets
/// nothing when the deployment has no shared App configured.
#[tokio::test]
async fn an_everruns_app_row_mints_with_the_shared_key_only() {
    let (fixture, github) = github_setup(ATTENDED).await;
    let app = fixture
        .db
        .get_github_app_for_identity(DEFAULT_ORG_ID, fixture.identity_id)
        .await
        .unwrap()
        .unwrap();
    fixture.db.delete_github_app(app.id).await.unwrap();
    fixture
        .db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: fixture.identity_id,
            provider: "github".to_string(),
            connection_type: crate::github_apps::EVERRUNS_APP_CONNECTION_TYPE.to_string(),
            provider_user_id: None,
            provider_username: Some("acme".to_string()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: Some(PERSON_INSTALLATION),
            provider_metadata: Some(serde_json::json!({ "org_connection_id": Uuid::now_v7() })),
        })
        .await
        .unwrap();
    assert_eq!(
        resolve(&fixture, &github, McpServerActsAs::Service).await,
        Some(("ghs_person".to_string(), McpServerActsAs::Service))
    );

    let without_shared_app = DbConnectionResolver::with_oauth_refresh(
        fixture.db.clone(),
        fixture.encryption.clone(),
        None,
        Arc::new(FakeRefreshExchange {
            calls: AtomicUsize::new(0),
            delay: StdDuration::ZERO,
            result: FakeRefreshResult::Success,
        }),
    )
    .with_github_apps_api(crate::github_apps::GitHubAppApi::new(
        crate::github_apps::GitHubEndpoints {
            api_url: github.uri(),
            web_url: "https://github.com".to_string(),
        },
    ))
    .bound_to_input_message(INPUT_MESSAGE);
    assert!(
        without_shared_app
            .get_github_token(fixture.session_id, &access(McpServerActsAs::Service))
            .await
            .unwrap()
            .is_none()
    );
}
