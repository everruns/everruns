//! `github_checkout_pull_request`: put a pull request's code into the
//! session's managed sandbox, private repositories included.
//!
//! Decision: the token never enters the sandbox's git config, a command line,
//! or the environment of later `bash` calls. It is handed to the sandbox as a
//! short-lived file (`SandboxWorkspace::exec_with_secret_file`) that a one-shot
//! credential helper reads during this fetch only, and the remote URL carries
//! no user info. Admission requires installation-mint evidence of read-only
//! access to this one repository and at most one hour remaining. A scope
//! request alone never authorizes sandbox delivery. THREAT[TM-GITHUB-007].

use async_trait::async_trait;
use everruns_contracts::runtime::sandbox_workspace::{SandboxWorkspace, SandboxWorkspaceExt};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::ToolHints;
use serde_json::{Value, json};

use super::checkout_files::{self, FileCheckout};
use crate::github::GitHubActsAs;
use crate::github::client::GitHubClient;
use crate::github::fix_pull_requests::is_valid_branch;
use crate::github::pull_requests::{github_token, target, target_schema};
use crate::github::tools::enforce_github_network_access;
use everruns_contracts::runtime::GitHubAccess;

const WORKSPACE: &str = "/workspace";
const DEFAULT_DEPTH: u64 = 100;
const MAX_DEPTH: u64 = 5_000;
const MAX_ERROR_BYTES: usize = 2_000;
const MAX_FILE_KIB: u64 = 2_048;

/// Only the trusted resolver can attest installation scope. Session secrets
/// and personal tokens remain host-side; requested access is not evidence.
pub(super) async fn checkout_token(
    context: &ToolContext,
    acts_as: GitHubActsAs,
    repo: &str,
) -> Result<String, ToolExecutionResult> {
    enforce_github_network_access(context)?;
    let access = GitHubAccess::new(
        acts_as.into(),
        &[("contents", "read"), ("pull_requests", "read")],
    )
    .for_repository(repo);
    if let Some(resolver) = &context.connection_resolver {
        let credential = resolver
            .get_github_token(context.session_id, &access)
            .await
            .map_err(|_| {
                ToolExecutionResult::tool_error("Could not verify GitHub checkout credential")
            })?;
        if let Some(credential) = credential
            && !credential.token.trim().is_empty()
            && access
                .acts_as
                .resolution_order()
                .contains(&credential.acted_as)
            && credential
                .github_installation_scope
                .as_ref()
                .is_some_and(|scope| scope.permits_checkout(repo, chrono::Utc::now()))
        {
            return Ok(credential.token);
        }
    }
    Err(ToolExecutionResult::tool_error(
        "github_checkout_pull_request requires a verified installation token with read-only access \
         to this repository and at most one hour remaining. Connect a GitHub App installation; \
         personal tokens and GITHUB_TOKEN session secrets cannot be delivered to the sandbox. \
         Use github_get_pull_request or github_get_pull_request_diff for host-side review.",
    ))
}

/// Reads the token from the file the sandbox wrapper names, for `get` only.
/// It is passed with `-c`, so it lives on one git command and nowhere else.
const CREDENTIAL_HELPER: &str = r#"!f() { test "$1" = get || exit 0; echo username=x-access-token; printf 'password=%s\n' "$(cat "$EVR_SECRET_FILE")"; }; f"#;

struct Checkout {
    repo: String,
    number: u64,
    path: String,
    depth: u64,
    /// Files-mode only: repository-relative prefixes to copy.
    paths: Vec<String>,
    /// Files-mode only: larger files are skipped.
    max_file_bytes: u64,
}

fn parse_paths(arguments: &Value) -> Result<Vec<String>, ToolExecutionResult> {
    let Some(value) = arguments.get("paths") else {
        return Ok(Vec::new());
    };
    let invalid = || {
        ToolExecutionResult::tool_error(
            "paths must be an array of repository-relative paths without `..`",
        )
    };
    let items = value.as_array().ok_or_else(invalid)?;
    items
        .iter()
        .map(|item| {
            let path = item.as_str().ok_or_else(invalid)?.trim().trim_matches('/');
            let valid = !path.is_empty()
                && path
                    .split('/')
                    .all(|segment| !segment.is_empty() && segment != "." && segment != "..");
            valid.then(|| path.to_string()).ok_or_else(invalid)
        })
        .collect()
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// `/workspace/<relative>` from an absolute or workspace-relative path.
fn checkout_path(input: Option<&str>, repo: &str) -> Result<String, String> {
    let name = repo.rsplit('/').next().unwrap_or(repo);
    let relative = match input.map(str::trim).filter(|path| !path.is_empty()) {
        None => name,
        Some(path) => path
            .strip_prefix("/workspace/")
            .or_else(|| (!path.starts_with('/')).then_some(path))
            .ok_or("path must be inside /workspace")?,
    }
    .trim_end_matches('/');
    let valid = !relative.is_empty()
        && relative.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && !segment.starts_with('-')
                && segment
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        });
    if !valid {
        return Err("path must be a directory inside /workspace, without `..`".to_string());
    }
    Ok(format!("{WORKSPACE}/{relative}"))
}

fn parse(arguments: &Value) -> Result<Checkout, ToolExecutionResult> {
    let (repo, number) = target(arguments)?;
    let path = checkout_path(arguments.get("path").and_then(Value::as_str), &repo)
        .map_err(ToolExecutionResult::tool_error)?;
    let depth = match arguments.get("depth") {
        None => DEFAULT_DEPTH,
        Some(depth) => depth
            .as_u64()
            .filter(|depth| (1..=MAX_DEPTH).contains(depth))
            .ok_or_else(|| {
                ToolExecutionResult::tool_error(format!(
                    "depth must be an integer from 1 to {MAX_DEPTH}"
                ))
            })?,
    };
    let max_file_bytes = match arguments.get("max_file_kib") {
        None => checkout_files::DEFAULT_MAX_FILE_BYTES,
        Some(kib) => kib
            .as_u64()
            .filter(|kib| (1..=MAX_FILE_KIB).contains(kib))
            .map(|kib| kib * 1024)
            .ok_or_else(|| {
                ToolExecutionResult::tool_error(format!(
                    "max_file_kib must be an integer from 1 to {MAX_FILE_KIB}"
                ))
            })?,
    };
    Ok(Checkout {
        repo,
        number,
        path,
        depth,
        paths: parse_paths(arguments)?,
        max_file_bytes,
    })
}

/// The shell script run in the sandbox. Holds no secret.
fn script(checkout: &Checkout, base_ref: &str) -> String {
    let Checkout {
        repo,
        number,
        path,
        depth,
        ..
    } = checkout;
    let url = quote(&format!("https://github.com/{repo}.git"));
    let pull = format!("refs/remotes/origin/pull/{number}");
    let base = format!("refs/remotes/origin/{base_ref}");
    let refspecs = format!(
        "{} {}",
        quote(&format!("+refs/pull/{number}/head:{pull}")),
        quote(&format!("+refs/heads/{base_ref}:{base}")),
    );
    let path = quote(path);
    let helper = quote(CREDENTIAL_HELPER);
    let (pull, base) = (quote(&pull), quote(&base));
    format!(
        "set -eu\n\
         export GIT_TERMINAL_PROMPT=0\n\
         mkdir -p -- {path}\n\
         cd -- {path}\n\
         [ -d .git ] || git init -q\n\
         if git remote get-url origin >/dev/null 2>&1; then git remote set-url origin {url}; \
         else git remote add origin {url}; fi\n\
         git -c credential.helper= -c credential.helper={helper} \
         fetch -q --no-tags --depth={depth} origin {refspecs}\n\
         git checkout -q --detach {pull}\n\
         echo \"head_sha=$(git rev-parse HEAD)\"\n\
         echo \"base_sha=$(git rev-parse {base})\"\n"
    )
}

fn output_value<'a>(stdout: &'a str, key: &str) -> Option<&'a str> {
    stdout
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .map(str::trim)
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn tail(text: &str) -> &str {
    let text = text.trim();
    let mut start = text.len().saturating_sub(MAX_ERROR_BYTES);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

async fn checkout(
    client: &GitHubClient,
    sandbox: &dyn SandboxWorkspace,
    context: &ToolContext,
    token: &str,
    checkout: Checkout,
) -> Result<Value, ToolExecutionResult> {
    let pull = client
        .pull_request(&checkout.repo, checkout.number)
        .await
        .map_err(ToolExecutionResult::tool_error)?;
    if !is_valid_branch(&pull.base.name) {
        return Err(ToolExecutionResult::tool_error(format!(
            "Unsupported base branch name: {:?}",
            pull.base.name
        )));
    }
    let response = sandbox
        .exec_with_secret_file(
            context,
            &script(&checkout, &pull.base.name),
            sandbox.workspace_root(),
            token,
        )
        .await?;
    if response.exit_code != 0 {
        return Err(ToolExecutionResult::tool_error(format!(
            "git checkout failed (exit {}): {}",
            response.exit_code,
            tail(if response.stderr.trim().is_empty() {
                &response.stdout
            } else {
                &response.stderr
            })
        )));
    }
    let head_sha = output_value(&response.stdout, "head_sha").unwrap_or(&pull.head.sha);
    let base_sha = output_value(&response.stdout, "base_sha").unwrap_or(&pull.base.sha);
    Ok(json!({
        "repo": checkout.repo,
        "number": checkout.number,
        "path": checkout.path,
        "head_sha": head_sha,
        "head_ref": pull.head.name,
        "base_ref": pull.base.name,
        "base_sha": base_sha,
        "depth": checkout.depth,
        "hint": format!(
            "Checked out at a detached HEAD. Compare with `git diff {base_sha}...HEAD`. \
             The sandbox keeps no GitHub credential, so fetch again with this tool."
        ),
    }))
}

impl CheckoutGitHubPullRequestTool {
    async fn checkout_into_files(
        &self,
        request: Checkout,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(store) = context.file_store.clone() else {
            return ToolExecutionResult::tool_error(
                "github_checkout_pull_request needs somewhere to put the code: a managed sandbox \
                 (session_sandbox) or session files (Bashkit or session_file_system).",
            );
        };
        let token = match github_token(
            context,
            self.acts_as,
            &request.repo,
            &[("contents", "read"), ("pull_requests", "read")],
        )
        .await
        {
            Ok(token) => token,
            Err(e) => return e,
        };
        let client = GitHubClient::with_base_url(token, self.api_base.clone());
        let files = FileCheckout {
            repo: &request.repo,
            number: request.number,
            path: &request.path,
            paths: &request.paths,
            max_file_bytes: request.max_file_bytes,
        };
        match checkout_files::checkout_into_files(&client, &store, context, files).await {
            Ok(result) => ToolExecutionResult::success(result),
            Err(e) => e,
        }
    }
}

pub struct CheckoutGitHubPullRequestTool {
    pub(crate) acts_as: GitHubActsAs,
    pub(crate) api_base: String,
}

impl Default for CheckoutGitHubPullRequestTool {
    fn default() -> Self {
        Self {
            acts_as: GitHubActsAs::default(),
            api_base: crate::github::GITHUB_API_BASE.to_string(),
        }
    }
}

impl CheckoutGitHubPullRequestTool {
    pub(crate) fn acting_as(acts_as: GitHubActsAs) -> Self {
        Self {
            acts_as,
            ..Self::default()
        }
    }
}

#[async_trait]
impl Tool for CheckoutGitHubPullRequestTool {
    fn narrate(
        &self,
        call: &everruns_contracts::tool_types::ToolCall,
        phase: everruns_contracts::runtime::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_contracts::runtime::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(
            everruns_contracts::runtime::tool_narration::narrate_labeled_action(
                &call.arguments,
                phase,
                locale,
                (
                    "Checking out GitHub pull request",
                    "Checked out GitHub pull request",
                    "Could not check out GitHub pull request",
                ),
                (
                    "Завантажую запит на злиття GitHub",
                    "Завантажив запит на злиття GitHub",
                    "Не вдалося завантажити запит на злиття GitHub",
                ),
                &["repo"],
            ),
        )
    }

    fn name(&self) -> &str {
        "github_checkout_pull_request"
    }

    fn description(&self) -> &str {
        "Check out a GitHub pull request (private repositories too) at /workspace/<repo name> or \
         `path`. In a managed sandbox: a git checkout of its head at a detached HEAD plus its base \
         branch, so `bash` can build, test and diff it. Without one (Bashkit): a snapshot of the \
         head's text files in the session files, readable with `bash` and the file tools, with no \
         git history; narrow it with `paths`. No lasting GitHub credential is left behind; call \
         this again to refresh."
    }

    fn parameters_schema(&self) -> Value {
        target_schema(json!({
            "path": {
                "type": "string",
                "description": "Directory inside /workspace to check out into. Defaults to /workspace/<repo name>."
            },
            "depth": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_DEPTH,
                "description": "Commits of history to fetch for the head and the base (default 100). Managed sandbox only."
            },
            "paths": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Snapshot only: repository-relative files or directories to copy or refresh, e.g. [\"AGENTS.md\", \"crates/core\"]; files outside them are left as they are. Default: the whole repository."
            },
            "max_file_kib": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_FILE_KIB,
                "description": "Snapshot only: skip files larger than this many KiB (default 1024)."
            }
        }))
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("github_checkout_pull_request requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let request = match parse(&arguments) {
            Ok(request) => request,
            Err(e) => return e,
        };
        // Before any token is minted: without a sandbox the token stays
        // host-side and the head is copied into the session files instead.
        let Some(sandbox) = context.extension::<SandboxWorkspaceExt>() else {
            return self.checkout_into_files(request, context).await;
        };
        let token = match checkout_token(context, self.acts_as, &request.repo).await {
            Ok(token) => token,
            Err(e) => return e,
        };
        let client = GitHubClient::with_base_url(token.clone(), self.api_base.clone());
        match checkout(&client, sandbox.0.as_ref(), context, &token, request).await {
            Ok(result) => ToolExecutionResult::success(result),
            Err(e) => e,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
#[path = "checkout_tests.rs"]
mod tests;
