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

/// The agent has its own App installed; the person connected the shared App.
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
    for (owner, installation) in [
        (fixture.identity_id, AGENT_INSTALLATION),
        (
            VirtualUserId::from_uuid(fixture.user_id),
            PERSON_INSTALLATION,
        ),
    ] {
        fixture
            .db
            .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
                virtual_user_id: owner,
                provider: "github".to_string(),
                connection_type: "github_app".to_string(),
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
