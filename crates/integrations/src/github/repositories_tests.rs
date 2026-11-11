//! The repository tools against real git: bare repositories stand in for
//! GitHub (a `url.insteadOf` rule in the fake sandbox's environment points
//! `https://github.com/` at them), and a shell in a temporary directory stands
//! in for the sandbox.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use everruns_contracts::error::Result as ContractResult;
use everruns_contracts::runtime::McpResolvedCredential;
use everruns_contracts::runtime::connection_services::UserConnectionResolver;
use everruns_contracts::session_sandbox::SessionSandboxExecResponse;

use super::*;

const TOKEN: &str = "ghs_repositorytoken456";

#[derive(Default)]
struct RecordingResolver {
    requests: Mutex<Vec<GitHubAccess>>,
    unverified: bool,
    /// GitHub refuses to mint a token with Contents write, as it does for an
    /// App that does not hold the permission.
    refuses_write: bool,
}

#[async_trait]
impl UserConnectionResolver for RecordingResolver {
    async fn get_connection_token(&self, _: SessionId, _: &str) -> ContractResult<Option<String>> {
        panic!("repository tools resolve through get_github_token");
    }

    async fn get_github_token(
        &self,
        _: SessionId,
        access: &GitHubAccess,
    ) -> ContractResult<Option<McpResolvedCredential>> {
        self.requests.lock().unwrap().push(access.clone());
        if self.refuses_write
            && access.permissions.get("contents").map(String::as_str) == Some("write")
        {
            return Err(everruns_contracts::error::AgentLoopError::store(
                "GitHub API error (422): The permissions requested are not granted to this installation.",
            ));
        }
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

/// Runs commands with `sh` under `root`, as a sandbox would.
struct ShellSandbox {
    root: PathBuf,
    home: PathBuf,
    remotes: PathBuf,
    /// Every command that was handed a secret, with that secret.
    secret_commands: Mutex<Vec<(String, String)>>,
}

impl ShellSandbox {
    fn git_env(&self, command: &mut Command) {
        command
            .env("HOME", &self.home)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", self.home.join("gitconfig"))
            .env("GIT_CONFIG_COUNT", "1")
            .env(
                "GIT_CONFIG_KEY_0",
                format!("url.file://{}/.insteadOf", self.remotes.display()),
            )
            .env("GIT_CONFIG_VALUE_0", "https://github.com/");
    }

    fn run(&self, command: &str, cwd: &Path, secret: Option<&str>) -> SessionSandboxExecResponse {
        let mut process = Command::new("sh");
        process.arg("-c").arg(command).current_dir(cwd);
        self.git_env(&mut process);
        let secret_file = self.home.join("secret");
        if let Some(secret) = secret {
            std::fs::write(&secret_file, secret).unwrap();
            process.env("EVR_SECRET_FILE", &secret_file);
        }
        let output = process.output().unwrap();
        let _ = std::fs::remove_file(&secret_file);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        SessionSandboxExecResponse {
            exit_code: output.status.code().unwrap_or(-1),
            total_lines: stdout.lines().count(),
            stdout,
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            success: output.status.success(),
            truncated: false,
            raw_output: None,
            hint: None,
        }
    }
}

#[async_trait]
impl SandboxWorkspace for ShellSandbox {
    async fn exec_with_secret_file(
        &self,
        _context: &ToolContext,
        command: &str,
        cwd: &str,
        secret: &str,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        assert_eq!(cwd, "/workspace");
        self.secret_commands
            .lock()
            .unwrap()
            .push((command.to_string(), secret.to_string()));
        Ok(self.run(command, &self.root, Some(secret)))
    }

    async fn provider_root(&self, _context: &ToolContext) -> Result<String, ToolExecutionResult> {
        Ok(self.root.to_string_lossy().into_owned())
    }

    async fn exec_raw(
        &self,
        _context: &ToolContext,
        command: &str,
        cwd: Option<&str>,
        _timeout_ms: Option<u64>,
    ) -> Result<SessionSandboxExecResponse, ToolExecutionResult> {
        let cwd = cwd.map(PathBuf::from).unwrap_or_else(|| self.root.clone());
        Ok(self.run(command, &cwd, None))
    }

    async fn read_file_raw(
        &self,
        _context: &ToolContext,
        path: &str,
    ) -> Result<Vec<u8>, ToolExecutionResult> {
        std::fs::read(path).map_err(|err| ToolExecutionResult::tool_error(err.to_string()))
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    sandbox: Arc<ShellSandbox>,
    resolver: Arc<RecordingResolver>,
    context: ToolContext,
}

impl Fixture {
    /// A workspace and a fake GitHub holding `repositories`, each with one
    /// commit on `main` and an `AGENTS.md`.
    fn new(repositories: &[&str]) -> Self {
        Self::with_resolver(repositories, RecordingResolver::default())
    }

    fn with_resolver(repositories: &[&str], resolver: RecordingResolver) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let sandbox = Arc::new(ShellSandbox {
            root: base.join("workspace"),
            home: base.join("home"),
            remotes: base.join("remotes"),
            secret_commands: Mutex::default(),
        });
        for path in [&sandbox.root, &sandbox.home, &sandbox.remotes] {
            std::fs::create_dir_all(path).unwrap();
        }
        let resolver = Arc::new(resolver);
        let context = ToolContext::new(SessionId::from_seed(0xabcdef12))
            .with_connection_resolver(resolver.clone())
            .with_extension(Arc::new(SandboxWorkspaceExt(sandbox.clone())));
        let fixture = Self {
            _dir: dir,
            sandbox,
            resolver,
            context,
        };
        for repo in repositories {
            fixture.seed(repo);
        }
        fixture
    }

    /// Run a shell command outside the sandbox tools, as the agent's own
    /// `bash` would.
    fn sh(&self, cwd: &Path, command: &str) -> String {
        let script = format!(
            "set -e\nexport GIT_AUTHOR_NAME=t GIT_AUTHOR_EMAIL=t@example.com \
             GIT_COMMITTER_NAME=t GIT_COMMITTER_EMAIL=t@example.com\n{command}"
        );
        let response = self.sandbox.run(&script, cwd, None);
        assert!(
            response.success,
            "`{command}` failed: {}{}",
            response.stdout, response.stderr
        );
        response.stdout.trim().to_string()
    }

    fn seed(&self, repo: &str) {
        let source = self.sandbox.home.join("seed");
        let bare = self.sandbox.remotes.join(format!("{repo}.git"));
        std::fs::create_dir_all(bare.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        self.sh(
            &source,
            &format!(
                "git init -q -b main . && echo '# {repo}' > README.md && \
                 echo 'Rules for {repo}.' > AGENTS.md && git add . && git commit -q -m init && \
                 git clone -q --bare . {} && rm -rf .git README.md AGENTS.md",
                quote(&bare.to_string_lossy())
            ),
        );
    }

    fn bare(&self, repo: &str) -> PathBuf {
        self.sandbox.remotes.join(format!("{repo}.git"))
    }

    fn checkout(&self, directory: &str) -> PathBuf {
        self.sandbox.root.join(directory)
    }

    async fn call(&self, config: &Value, tool: &str, arguments: Value) -> ToolExecutionResult {
        let tools = RepositoriesCapability.tools_with_config(config);
        let tool = tools
            .iter()
            .find(|candidate| candidate.name() == tool)
            .unwrap_or_else(|| panic!("{tool} is not offered"));
        tool.execute_with_context(arguments, &self.context).await
    }

    async fn ok(&self, config: &Value, tool: &str, arguments: Value) -> Value {
        match self.call(config, tool, arguments).await {
            ToolExecutionResult::Success(value) => value,
            other => panic!("{tool} failed: {other:?}"),
        }
    }

    async fn error(&self, config: &Value, tool: &str, arguments: Value) -> String {
        match self.call(config, tool, arguments).await {
            ToolExecutionResult::ToolError(message) => message,
            other => panic!("{tool} should have failed: {other:?}"),
        }
    }

    fn requests(&self) -> Vec<GitHubAccess> {
        self.resolver.requests.lock().unwrap().clone()
    }
}

fn two_repositories() -> Value {
    json!({
        "repositories": [
            { "repo": "acme/app", "access": "write" },
            { "repo": "acme/lib" }
        ]
    })
}

fn work_branch(config: &Value, fixture: &Fixture) -> String {
    RepositoriesConfig::parse(config)
        .unwrap()
        .work_branch(fixture.context.session_id)
}

fn repository<'a>(status: &'a Value, repo: &str) -> &'a Value {
    status["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["repo"] == repo)
        .unwrap_or_else(|| panic!("{repo} missing from {status}"))
}

#[tokio::test]
async fn status_checks_out_the_listed_repositories_side_by_side() {
    let fixture = Fixture::new(&["acme/app", "acme/lib"]);
    let config = two_repositories();
    let status = fixture.ok(&config, "repo_status", json!({})).await;
    let branch = work_branch(&config, &fixture);
    assert_eq!(status["work_branch"], branch.as_str());

    let app = repository(&status, "acme/app");
    assert_eq!(app["path"], "/workspace/app");
    assert_eq!(app["access"], "write");
    assert_eq!(app["branch"], branch.as_str());
    assert_eq!(app["on_work_branch"], true);
    assert_eq!(app["base_branch"], "main");
    assert_eq!(app["commits_ahead_of_base"], 0);
    assert_eq!(app["agents_md"], "Rules for acme/app.\n");

    let lib = repository(&status, "acme/lib");
    assert_eq!(lib["path"], "/workspace/lib");
    assert_eq!(lib["access"], "read");
    assert_eq!(lib["branch"], "main");
    assert_eq!(lib["on_work_branch"], false);

    // One read-only token for both repositories of the owner, and it is in no
    // command text.
    let requests = fixture.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].repositories, ["acme/app", "acme/lib"]);
    assert_eq!(
        requests[0].permissions,
        BTreeMap::from([("contents".to_string(), "read".to_string())])
    );
    for (command, secret) in fixture.sandbox.secret_commands.lock().unwrap().iter() {
        assert_eq!(secret, TOKEN);
        assert!(!command.contains(TOKEN));
    }
    // Commits made in the sandbox have an author.
    assert_eq!(
        fixture.sh(&fixture.checkout("app"), "git config user.email"),
        DEFAULT_COMMIT_EMAIL
    );

    // A second look clones nothing, asks for no token and repeats no
    // instructions.
    let again = fixture.ok(&config, "repo_status", json!({})).await;
    assert_eq!(fixture.requests().len(), 1);
    assert!(repository(&again, "acme/app").get("agents_md").is_none());
}

#[tokio::test]
async fn push_sends_only_the_work_branch_of_a_writable_repository() {
    let fixture = Fixture::new(&["acme/app", "acme/lib"]);
    let config = two_repositories();
    fixture.ok(&config, "repo_status", json!({})).await;
    let branch = work_branch(&config, &fixture);
    let app = fixture.checkout("app");
    fixture.sh(
        &app,
        "echo change >> README.md && git commit -q -am change && echo dirty > scratch.txt",
    );

    let before = fixture.ok(&config, "repo_status", json!({})).await;
    let entry = repository(&before, "acme/app");
    assert_eq!(entry["commits_ahead_of_base"], 1);
    assert_eq!(entry["uncommitted_files"], 1);

    let pushed = fixture
        .ok(&config, "repo_push", json!({ "repo": "acme/app" }))
        .await;
    assert_eq!(pushed["branch"], branch.as_str());
    assert_eq!(pushed["base_branch"], "main");
    assert_eq!(
        pushed["compare_url"],
        format!("https://github.com/acme/app/compare/main...{branch}")
    );
    let head = fixture.sh(&app, "git rev-parse HEAD");
    assert_eq!(
        fixture.sh(
            &fixture.bare("acme/app"),
            &format!("git rev-parse refs/heads/{branch}")
        ),
        head
    );
    // `main` on the remote did not move.
    assert_ne!(
        fixture.sh(&fixture.bare("acme/app"), "git rev-parse refs/heads/main"),
        head
    );

    let push_request = fixture.requests().pop().unwrap();
    assert_eq!(push_request.repositories, ["acme/app"]);
    assert_eq!(
        push_request.permissions,
        BTreeMap::from([("contents".to_string(), "write".to_string())])
    );

    let after = fixture.ok(&config, "repo_status", json!({})).await;
    assert_eq!(repository(&after, "acme/app")["commits_not_pushed"], 0);
}

#[tokio::test]
async fn push_is_refused_without_asking_for_a_token() {
    let fixture = Fixture::new(&["acme/app", "acme/lib"]);
    let config = two_repositories();
    fixture.ok(&config, "repo_status", json!({})).await;
    let asked = fixture.requests().len();
    let branch = work_branch(&config, &fixture);

    // A read-only repository.
    let error = fixture
        .error(&config, "repo_push", json!({ "repo": "acme/lib" }))
        .await;
    assert!(error.contains("read-only"), "{error}");

    // A repository the session does not have.
    let error = fixture
        .error(&config, "repo_push", json!({ "repo": "acme/secret" }))
        .await;
    assert!(error.contains("read-only"), "{error}");

    // Another branch than the work branch.
    fixture.sh(&fixture.checkout("app"), "git checkout -q main");
    let error = fixture
        .error(&config, "repo_push", json!({ "repo": "acme/app" }))
        .await;
    assert!(error.contains(&branch), "{error}");

    // A checkout whose origin was pointed at the read-only repository is no
    // longer the writable one, and the read-only one gains no branch.
    fixture.sh(
        &fixture.checkout("app"),
        &format!(
            "git checkout -q {branch} && git remote set-url origin https://github.com/acme/lib.git"
        ),
    );
    let error = fixture
        .error(&config, "repo_push", json!({ "repo": "acme/app" }))
        .await;
    assert!(error.contains("not checked out"), "{error}");
    let branches = fixture.sh(&fixture.bare("acme/lib"), "git branch --list");
    assert!(!branches.contains(&branch), "{branches}");

    assert_eq!(fixture.requests().len(), asked);
}

#[tokio::test]
async fn a_push_the_app_may_not_make_says_which_permission_is_missing() {
    let fixture = Fixture::with_resolver(
        &["acme/app"],
        RecordingResolver {
            refuses_write: true,
            ..Default::default()
        },
    );
    let config = json!({ "repositories": [{ "repo": "acme/app", "access": "write" }] });
    fixture.ok(&config, "repo_status", json!({})).await;
    fixture.sh(
        &fixture.checkout("app"),
        "echo change >> README.md && git commit -q -am change",
    );

    let error = fixture
        .error(&config, "repo_push", json!({ "repo": "acme/app" }))
        .await;
    assert!(error.contains("Contents write"), "{error}");
    assert!(error.contains("acme/app"), "{error}");
    // What GitHub answered stays on the server.
    assert!(!error.contains("422"), "{error}");
}

#[tokio::test]
async fn unverified_credentials_never_reach_the_sandbox() {
    let fixture = Fixture::with_resolver(
        &["acme/app"],
        RecordingResolver {
            unverified: true,
            ..Default::default()
        },
    );
    let config = json!({ "repositories": [{ "repo": "acme/app", "access": "write" }] });
    let status = fixture.ok(&config, "repo_status", json!({})).await;
    assert!(status["repositories"].as_array().unwrap().is_empty());
    let failure = &status["not_checked_out"][0];
    assert_eq!(failure["repo"], "acme/app");
    assert!(
        failure["error"]
            .as_str()
            .unwrap()
            .contains("GitHub App installation"),
        "{failure}"
    );
    assert!(fixture.sandbox.secret_commands.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_agent_adds_repositories_within_the_add_policy() {
    let fixture = Fixture::new(&["acme/app", "acme/docs", "other/app"]);
    let config = json!({ "repositories": [{ "repo": "acme/app" }] });
    fixture.ok(&config, "repo_status", json!({})).await;

    let added = fixture
        .ok(&config, "repo_add", json!({ "repo": "acme/docs" }))
        .await;
    assert_eq!(added["path"], "/workspace/docs");
    assert_eq!(added["access"], "read");
    assert_eq!(added["branch"], "main");
    assert_eq!(added["already_present"], false);
    assert_eq!(added["agents_md"], "Rules for acme/docs.\n");

    // Adding it again is a no-op.
    let again = fixture
        .ok(&config, "repo_add", json!({ "repo": "acme/docs" }))
        .await;
    assert_eq!(again["already_present"], true);

    // A second repository named `app` goes next to the first.
    let clash = fixture
        .ok(&config, "repo_add", json!({ "repo": "other/app" }))
        .await;
    assert_eq!(clash["path"], "/workspace/other-app");

    // Nothing here is writable, so there is nothing to push with.
    let names = |config: &Value| -> Vec<String> {
        RepositoriesCapability
            .tools_with_config(config)
            .iter()
            .map(|tool| tool.name().to_string())
            .collect()
    };
    assert_eq!(names(&config), ["repo_status", "repo_sync", "repo_add"]);
    assert_eq!(
        names(&json!({ "add_repositories": "off" })),
        ["repo_status", "repo_sync"]
    );
    assert_eq!(
        names(&json!({ "add_repositories": "write" })),
        ["repo_status", "repo_sync", "repo_add", "repo_push"]
    );
}

#[tokio::test]
async fn sync_fetches_the_base_branch_without_merging() {
    let fixture = Fixture::new(&["acme/app"]);
    let config = json!({ "repositories": [{ "repo": "acme/app", "access": "write" }] });
    fixture.ok(&config, "repo_status", json!({})).await;

    // Someone else lands a commit on `main`.
    let other = fixture.sandbox.home.join("other");
    std::fs::create_dir_all(&other).unwrap();
    fixture.sh(
        &other,
        &format!(
            "git clone -q {} . && echo more >> README.md && git commit -q -am more && \
             git push -q origin main",
            quote(&fixture.bare("acme/app").to_string_lossy())
        ),
    );
    let app = fixture.checkout("app");
    let head = fixture.sh(&app, "git rev-parse HEAD");

    let synced = fixture
        .ok(&config, "repo_sync", json!({ "repo": "acme/app" }))
        .await;
    assert_eq!(synced["fetched"], "origin/main");
    assert_eq!(synced["commits_behind"], 1);
    assert_eq!(synced["commits_ahead"], 0);
    assert_eq!(fixture.sh(&app, "git rev-parse HEAD"), head);

    let error = fixture
        .error(
            &config,
            "repo_sync",
            json!({ "repo": "acme/app", "ref": "../x" }),
        )
        .await;
    assert!(error.contains("valid branch"), "{error}");
}

#[test]
fn credential_helper_answers_for_github_only() {
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("secret");
    std::fs::write(&secret, TOKEN).unwrap();
    let fill = |host: &str| {
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "printf 'protocol=https\\nhost={host}\\n\\n' | \
                 git -c credential.helper= -c credential.helper={} credential fill",
                quote(CREDENTIAL_HELPER)
            ))
            .env("EVR_SECRET_FILE", &secret)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", dir.path().join("gitconfig"))
            .env("HOME", dir.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert!(fill("github.com").contains(&format!("password={TOKEN}")));
    assert!(!fill("example.com").contains(TOKEN));
    assert!(!fill("github.com.example.com").contains(TOKEN));
}

#[test]
fn config_is_validated() {
    let valid = |config: Value| RepositoriesCapability.validate_config(&config);
    assert!(valid(Value::Null).is_ok());
    assert!(valid(two_repositories()).is_ok());
    assert!(
        valid(json!({ "repositories": [{ "repo": "acme/app", "ref": "release/1.x" }] })).is_ok()
    );
    assert!(valid(json!({ "repositories": [{ "repo": "not-a-repo" }] })).is_err());
    assert!(valid(json!({ "repositories": [{ "repo": "acme/app", "ref": "a..b" }] })).is_err());
    assert!(
        valid(json!({ "repositories": [{ "repo": "acme/app" }, { "repo": "Acme/App" }] })).is_err()
    );
    assert!(valid(json!({ "repositories": [{ "repo": "acme/app", "access": "admin" }] })).is_err());
    assert!(valid(json!({ "branch_prefix": "bad prefix/" })).is_err());
    assert!(valid(json!({ "depth": 0 })).is_err());
    assert!(valid(json!({ "commit_email": "a\nb" })).is_err());
    assert!(valid(json!({ "unknown": true })).is_err());
    assert!(RepositoriesCapability.config_schema().is_some());
}

#[tokio::test]
async fn the_model_is_told_what_the_tools_enforce() {
    let config = two_repositories();
    let session = SessionId::from_seed(7);
    let block = environment_block(&RepositoriesConfig::parse(&config).unwrap(), session);
    assert!(
        block.contains("`acme/app` at `/workspace/app` (write)"),
        "{block}"
    );
    assert!(
        block.contains("`acme/lib` at `/workspace/lib` (read)"),
        "{block}"
    );
    let branch = RepositoriesConfig::default().work_branch(session);
    assert!(branch.starts_with("everruns/") && branch.len() == "everruns/".len() + 8);
    assert!(block.contains(&branch), "{block}");
}

#[test]
fn remote_urls_map_back_to_repositories() {
    assert_eq!(
        repo_from_url("https://github.com/acme/app.git").as_deref(),
        Some("acme/app")
    );
    assert_eq!(
        repo_from_url("git@github.com:acme/app.git").as_deref(),
        Some("acme/app")
    );
    assert_eq!(
        repo_from_url("https://github.com/acme/app/").as_deref(),
        Some("acme/app")
    );
    assert_eq!(repo_from_url("https://example.com/acme/app.git"), None);
    assert_eq!(repo_from_url("https://github.com/acme/app/extra"), None);
}

#[test]
fn only_a_writable_configuration_can_push() {
    let can_push = RepositoriesConfig::config_can_push;
    assert!(!can_push(&Value::Null));
    assert!(!can_push(
        &json!({ "repositories": [{ "repo": "acme/docs" }] })
    ));
    assert!(can_push(
        &json!({ "repositories": [{ "repo": "acme/app", "access": "write" }] })
    ));
    assert!(can_push(&json!({ "add_repositories": "write" })));
    // A config that does not parse grants nothing.
    assert!(!can_push(&json!({ "repositories": "everything" })));
}

/// The model is offered `repo_push` and `repo_add` exactly when the config
/// lets the agent use them.
#[test]
fn the_advertised_tools_follow_the_configuration() {
    let advertised = |config: Value| -> Vec<String> {
        RepositoriesCapability
            .tool_definitions_with_config(&config)
            .iter()
            .map(|definition| definition.name().to_string())
            .collect()
    };
    assert_eq!(
        advertised(json!({})),
        ["repo_status", "repo_sync", "repo_add"]
    );
    assert_eq!(
        advertised(json!({ "repositories": [{ "repo": "acme/app", "access": "write" }] })),
        ["repo_status", "repo_sync", "repo_add", "repo_push"]
    );
    assert_eq!(
        advertised(json!({ "add_repositories": "off" })),
        ["repo_status", "repo_sync"]
    );
}
