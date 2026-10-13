use std::sync::{Arc, Mutex};

use everruns_contracts::error::Result as ContractResult;
use everruns_contracts::runtime::GitHubAccess;
use everruns_contracts::runtime::McpResolvedCredential;
use everruns_contracts::runtime::connection_services::UserConnectionResolver;
use everruns_contracts::runtime::mcp_server::McpServerActsAs;
use everruns_contracts::session_sandbox::SessionSandboxExecResponse;
use everruns_contracts::typed_id::SessionId;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

const TOKEN: &str = "ghs_narrowtoken123";
const HEAD: &str = "1111111111111111111111111111111111111111";
const BASE: &str = "2222222222222222222222222222222222222222";

#[tokio::test]
async fn checkout_refuses_unverified_credentials_before_fetch() {
    let resolver = Arc::new(RecordingResolver {
        unverified: true,
        ..Default::default()
    });
    let workspace = Arc::new(FakeWorkspace::default());
    let tool = CheckoutGitHubPullRequestTool {
        acts_as: GitHubActsAs::Person,
        api_base: "http://127.0.0.1:1".to_string(),
    };
    let result = tool
        .execute_with_context(
            json!({"repo": "acme/app", "number": 7}),
            &context(&resolver, &workspace),
        )
        .await;
    let ToolExecutionResult::ToolError(message) = result else {
        panic!("{result:?}");
    };
    assert!(message.contains("verified installation token"), "{message}");
    assert!(workspace.calls.lock().unwrap().is_empty());
}

#[derive(Default)]
struct RecordingResolver {
    requests: Mutex<Vec<GitHubAccess>>,
    unverified: bool,
}

#[async_trait]
impl UserConnectionResolver for RecordingResolver {
    async fn get_connection_token(&self, _: SessionId, _: &str) -> ContractResult<Option<String>> {
        panic!("GitHub tools resolve through get_github_token");
    }

    async fn get_github_token(
        &self,
        _: SessionId,
        access: &GitHubAccess,
    ) -> ContractResult<Option<McpResolvedCredential>> {
        self.requests.lock().unwrap().push(access.clone());
        Ok(Some(McpResolvedCredential {
            token: TOKEN.to_string(),
            acted_as: access.acts_as,
            github_installation_scope: (!self.unverified).then(|| {
                everruns_contracts::runtime::GitHubInstallationScope {
                    repositories: access.repositories.clone(),
                    permissions: access.permissions.clone(),
                    expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
                }
            }),
        }))
    }
}

/// Records every command and the secret it was handed.
#[derive(Default)]
struct FakeWorkspace {
    calls: Mutex<Vec<(String, String, String)>>,
    exit_code: i32,
}

#[async_trait]
impl SandboxWorkspace for FakeWorkspace {
    async fn exec_with_secret_file(
        &self,
        _context: &ToolContext,
        command: &str,
        cwd: &str,
        secret: &str,
    ) -> std::result::Result<SessionSandboxExecResponse, ToolExecutionResult> {
        self.calls
            .lock()
            .unwrap()
            .push((command.to_string(), cwd.to_string(), secret.to_string()));
        Ok(SessionSandboxExecResponse {
            exit_code: self.exit_code,
            stdout: format!("head_sha={HEAD}\nbase_sha={BASE}\n"),
            stderr: if self.exit_code == 0 {
                String::new()
            } else {
                "fatal: could not read Username".to_string()
            },
            success: self.exit_code == 0,
            truncated: false,
            total_lines: 2,
            raw_output: None,
            hint: None,
        })
    }
}

async fn github() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/app/pulls/7"))
        .and(header("authorization", format!("Bearer {TOKEN}").as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "number": 7, "title": "t", "state": "open", "html_url": "https://github.com/acme/app/pull/7",
            "base": {"ref": "main", "sha": BASE},
            "head": {"ref": "feature/x", "sha": HEAD}
        })))
        .mount(&server)
        .await;
    server
}

fn context(resolver: &Arc<RecordingResolver>, workspace: &Arc<FakeWorkspace>) -> ToolContext {
    ToolContext::new(SessionId::new())
        .with_connection_resolver(resolver.clone())
        .with_extension(Arc::new(SandboxWorkspaceExt(workspace.clone())))
}

fn tool(server: &MockServer) -> CheckoutGitHubPullRequestTool {
    CheckoutGitHubPullRequestTool {
        acts_as: GitHubActsAs::Agent,
        api_base: server.uri(),
    }
}

#[tokio::test]
async fn checks_out_with_a_narrow_token_that_never_reaches_a_command() {
    let server = github().await;
    let resolver = Arc::new(RecordingResolver::default());
    let workspace = Arc::new(FakeWorkspace::default());
    let result = tool(&server)
        .execute_with_context(
            json!({"repo": "acme/app", "number": 7}),
            &context(&resolver, &workspace),
        )
        .await;

    let ToolExecutionResult::Success(value) = result else {
        panic!("{result:?}");
    };
    assert_eq!(value["path"], "/workspace/app");
    assert_eq!(value["head_sha"], HEAD);
    assert_eq!(value["base_ref"], "main");
    assert_eq!(value["base_sha"], BASE);

    let requests = resolver.requests.lock().unwrap().clone();
    assert_eq!(
        requests,
        vec![
            GitHubAccess::new(
                McpServerActsAs::Service,
                &[("contents", "read"), ("pull_requests", "read")]
            )
            .for_repository("acme/app")
        ]
    );

    let calls = workspace.calls.lock().unwrap().clone();
    assert_eq!(calls.len(), 1);
    let (command, cwd, secret) = &calls[0];
    assert_eq!(secret, TOKEN);
    assert_eq!(cwd, "/workspace");
    assert!(!command.contains(TOKEN), "{command}");
    assert!(
        command.contains("'https://github.com/acme/app.git'"),
        "{command}"
    );
    for line in command.lines().filter(|line| line.contains("github.com")) {
        assert!(!line.contains('@'), "remote URL carries user info: {line}");
    }
    assert!(command.contains("-c credential.helper= -c credential.helper="));
    assert!(command.contains("$EVR_SECRET_FILE"));
    assert!(command.contains("GIT_TERMINAL_PROMPT=0"));
    assert!(command.contains("+refs/pull/7/head:refs/remotes/origin/pull/7"));
    assert!(command.contains("+refs/heads/main:refs/remotes/origin/main"));
    assert!(command.contains("--depth=100"));
    assert!(command.contains("checkout -q --detach 'refs/remotes/origin/pull/7'"));
    assert!(!command.contains("credential.helper store"));
}

#[tokio::test]
async fn honours_path_and_acting_identity() {
    let server = github().await;
    let resolver = Arc::new(RecordingResolver::default());
    let workspace = Arc::new(FakeWorkspace::default());
    let tool = CheckoutGitHubPullRequestTool {
        acts_as: GitHubActsAs::Person,
        api_base: server.uri(),
    };
    let result = tool
        .execute_with_context(
            json!({"repo": "acme/app", "number": 7, "path": "/workspace/review/app", "depth": 5}),
            &context(&resolver, &workspace),
        )
        .await;
    let ToolExecutionResult::Success(value) = result else {
        panic!("{result:?}");
    };
    assert_eq!(value["path"], "/workspace/review/app");
    assert_eq!(
        resolver.requests.lock().unwrap()[0].acts_as,
        McpServerActsAs::User
    );
    let command = workspace.calls.lock().unwrap()[0].0.clone();
    assert!(command.contains("mkdir -p -- '/workspace/review/app'"));
    assert!(command.contains("--depth=5"));
}

#[tokio::test]
async fn git_failure_is_a_tool_error() {
    let server = github().await;
    let resolver = Arc::new(RecordingResolver::default());
    let workspace = Arc::new(FakeWorkspace {
        exit_code: 128,
        ..Default::default()
    });
    let result = tool(&server)
        .execute_with_context(
            json!({"repo": "acme/app", "number": 7}),
            &context(&resolver, &workspace),
        )
        .await;
    let ToolExecutionResult::ToolError(message) = result else {
        panic!("{result:?}");
    };
    assert!(message.contains("exit 128"), "{message}");
    assert!(message.contains("could not read Username"), "{message}");
}

#[tokio::test]
async fn without_a_sandbox_it_says_so_and_mints_no_token() {
    let resolver = Arc::new(RecordingResolver::default());
    let context = ToolContext::new(SessionId::new()).with_connection_resolver(resolver.clone());
    let result = CheckoutGitHubPullRequestTool::default()
        .execute_with_context(json!({"repo": "acme/app", "number": 7}), &context)
        .await;
    let ToolExecutionResult::ToolError(message) = result else {
        panic!("{result:?}");
    };
    assert!(message.contains("managed sandbox"), "{message}");
    assert!(resolver.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rejects_bad_repos_and_paths_before_anything_else() {
    let resolver = Arc::new(RecordingResolver::default());
    let workspace = Arc::new(FakeWorkspace::default());
    let context = context(&resolver, &workspace);
    for arguments in [
        json!({"repo": "acme", "number": 7}),
        json!({"repo": "acme/../etc", "number": 7}),
        json!({"repo": "acme/app", "number": 0}),
        json!({"repo": "acme/app", "number": 7, "path": "/etc/app"}),
        json!({"repo": "acme/app", "number": 7, "path": "/workspace/../etc"}),
        json!({"repo": "acme/app", "number": 7, "path": "../x"}),
        json!({"repo": "acme/app", "number": 7, "path": "/workspace"}),
        json!({"repo": "acme/app", "number": 7, "path": "a b"}),
        json!({"repo": "acme/app", "number": 7, "path": "x/-rf"}),
        json!({"repo": "acme/app", "number": 7, "path": "x';id;'"}),
        json!({"repo": "acme/app", "number": 7, "depth": 0}),
    ] {
        let result = CheckoutGitHubPullRequestTool::default()
            .execute_with_context(arguments.clone(), &context)
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(_)),
            "{arguments}: {result:?}"
        );
    }
    assert!(resolver.requests.lock().unwrap().is_empty());
    assert!(workspace.calls.lock().unwrap().is_empty());
}

#[test]
fn checkout_paths_stay_in_the_workspace() {
    assert_eq!(checkout_path(None, "acme/app").unwrap(), "/workspace/app");
    assert_eq!(
        checkout_path(Some("src/app/"), "acme/app").unwrap(),
        "/workspace/src/app"
    );
    assert_eq!(
        checkout_path(Some("/workspace/a.b_c-d"), "acme/app").unwrap(),
        "/workspace/a.b_c-d"
    );
}

#[test]
fn reads_shas_from_the_script_output() {
    let stdout = format!("noise\nhead_sha={HEAD}\nbase_sha=nothex\n");
    assert_eq!(output_value(&stdout, "head_sha"), Some(HEAD));
    assert_eq!(output_value(&stdout, "base_sha"), None);
}
