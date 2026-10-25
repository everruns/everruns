use super::*;
use crate::github_apps::{GitHubAppApi, GitHubAppContext, GitHubEndpoints};
use crate::storage::{CreateVirtualUserConnectionRow, StorageBackend};
use base64::{Engine, engine::general_purpose::STANDARD};
use everruns_core::{DEFAULT_ORG_ID, connection_services::UserConnectionResolver};
use std::sync::Arc;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

pub(crate) async fn run(ctx: &Ctx, name: &str, params: Value) -> Result<Value, CommandError> {
    dispatch(name, params, ctx)
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
}

struct Tokens;
#[async_trait::async_trait]
impl UserConnectionResolver for Tokens {
    async fn get_connection_token(
        &self,
        _: everruns_contracts::typed_id::SessionId,
        _: &str,
    ) -> crate::kernel_imports::Result<Option<String>> {
        Ok(None)
    }
    async fn get_github_token_for_owner(
        &self,
        org: i64,
        _: Uuid,
        access: &everruns_contracts::runtime::GitHubAccess,
    ) -> crate::kernel_imports::Result<Option<String>> {
        assert_eq!(org, DEFAULT_ORG_ID);
        assert_eq!(access.repositories, vec!["acme/agents"]);
        assert_eq!(access.permissions.len(), 1);
        assert!(matches!(
            access.permissions.get("contents").map(String::as_str),
            Some("read" | "write")
        ));
        Ok(Some("source-test-token".into()))
    }
}
pub(crate) async fn setup() -> (Ctx, MockServer, String) {
    let db = Arc::new(StorageBackend::test_database());
    crate::setup::org_init::initialize_org_harnesses(&db, DEFAULT_ORG_ID)
        .await
        .unwrap();
    let remote = MockServer::start().await;
    let ctx = Ctx::minimal_for_test(Caller::internal(DEFAULT_ORG_ID), db, None)
        .with_connection_resolver(Some(Arc::new(Tokens)))
        .with_github_apps(GitHubAppContext {
            api: GitHubAppApi::new(GitHubEndpoints {
                api_url: remote.uri(),
                web_url: "https://github.com".into(),
            }),
            api_base_url: "https://api.example".into(),
            frontend_url: "https://app.example".into(),
            shared_app_slug: None,
        });
    let agent=super::super::CreateAgent(serde_json::from_value(json!({"name":"source-agent","system_prompt":"Original instructions","harness_name":"base"})).unwrap()).run(&ctx).await.unwrap();
    let row = ctx
        .db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &agent.public_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let (identity, _) = crate::domains::virtual_users::lifecycle::ensure_identity_for_agent(
        &ctx.db,
        DEFAULT_ORG_ID,
        &row,
    )
    .await
    .unwrap();
    ctx.db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: identity,
            provider: "github".into(),
            connection_type: crate::github_apps::GITHUB_APP_CONNECTION_TYPE.into(),
            provider_user_id: None,
            provider_username: Some("acme".into()),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some("contents:read".into()),
            expires_at: None,
            installation_id: Some(42),
            provider_metadata: None,
        })
        .await
        .unwrap();
    (ctx, remote, agent.public_id.to_string())
}
pub(crate) fn location() -> SourceLocation {
    SourceLocation {
        repository: "acme/agents".into(),
        branch: "main".into(),
        path: String::new(),
    }
}
pub(crate) async fn get(remote: &MockServer, p: &str, value: Value) {
    Mock::given(method("GET"))
        .and(path(p))
        .and(header("authorization", "Bearer source-test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(remote)
        .await;
}
pub(crate) async fn package(
    remote: &MockServer,
    revision: char,
    instructions: &str,
    capability: Option<&str>,
) {
    package_with_skills(remote, revision, instructions, capability, true).await;
}
pub(crate) async fn package_with_skills(
    remote: &MockServer,
    revision: char,
    instructions: &str,
    capability: Option<&str>,
    include_skills: bool,
) {
    let commit = revision.to_string().repeat(40);
    let tree = "b".repeat(40);
    get(
        remote,
        "/repos/acme/agents",
        json!({"id":123,"full_name":"acme/agents","default_branch":"main"}),
    )
    .await;
    get(
        remote,
        "/repos/acme/agents/git/ref/heads/main",
        json!({"object":{"sha":commit}}),
    )
    .await;
    get(
        remote,
        &format!("/repos/acme/agents/git/commits/{commit}"),
        json!({"tree":{"sha":tree}}),
    )
    .await;
    let manifest = format!(
        "schema_version=1\nname='source-agent'\nharness='base'\nfiles=['data/**']\n{}",
        capability
            .map(|c| format!("capabilities=['{c}']\n"))
            .unwrap_or_default()
    );
    let manifest = format!("{manifest}\n[channels.chat]\ntype='ag_ui'\n");
    let skill = "---\nname: helper\ndescription: A helper skill.\n---\nUse the runbook.";
    let mut files = vec![
        ("agent.toml", manifest.as_bytes()),
        ("instructions.md", instructions.as_bytes()),
        ("data/runbook.txt", b"Follow this runbook.".as_slice()),
    ];
    if include_skills {
        files.push((".agents/skills/helper/SKILL.md", skill.as_bytes()));
    }
    let entries:Vec<_>=files.iter().enumerate().map(|(i,(p,b))|json!({"path":p,"mode":"100644","type":"blob","size":b.len(),"sha":format!("{:040x}",i+1)})).collect();
    get(
        remote,
        &format!("/repos/acme/agents/git/trees/{tree}"),
        json!({"truncated":false,"tree":entries}),
    )
    .await;
    for (i, (_, bytes)) in files.iter().enumerate() {
        get(
            remote,
            &format!("/repos/acme/agents/git/blobs/{:040x}", i + 1),
            json!({"encoding":"base64","content":STANDARD.encode(bytes)}),
        )
        .await;
    }
}
pub(crate) async fn connect(ctx: &Ctx, id: &str) -> AgentSourceStatus {
    let preview = PreviewAgentSource {
        agent_id: id.into(),
        req: location(),
    }
    .run(ctx)
    .await
    .unwrap();
    assert_eq!(preview["valid"], true, "{preview}");
    ConnectAgentSource {
        agent_id: id.into(),
        req: ConnectSourceRequest {
            mode: SourceMode::Pull,
            location: location(),
            commit: preview["commit"].as_str().unwrap().into(),
            agent_digest: preview["agent_digest"].as_str().unwrap().into(),
            expected_generation: None,
        },
    }
    .run(ctx)
    .await
    .unwrap()
}
