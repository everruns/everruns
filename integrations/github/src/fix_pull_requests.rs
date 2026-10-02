//! `create_github_pull_request`: open a pull request from a pushed branch.
//!
//! Decisions:
//! - Offered only when the `github` capability's `allow_pull_requests`
//!   setting is on (off by default). The tool is absent otherwise, so no
//!   prompt, diff or scanned file can make an agent open one.
//! - Same-repository branches only: the head is a branch of `repo`, never a
//!   fork, so the pull request carries exactly what the agent pushed there.
//! - Draft by default, so a person decides when it is ready.
//! - Idempotent on the head branch: an open pull request from the same
//!   branch is returned instead of failing or opening another.
//! - The branch push itself happens in the sandbox with git and needs the
//!   App's `Contents: write` permission, which agent Apps do not request by
//!   default (`crates/server/src/github_apps.rs`). Without it the push fails
//!   and no pull request can be opened.

use async_trait::async_trait;
use everruns_core::tool_context::ToolContext;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_provider::tool_types::ToolHints;
use serde_json::{Value, json};

use crate::client::GitHubClient;
use crate::pull_requests::github_token;
use crate::tools::{is_valid_owner_repo, required_str};

const MAX_BODY_BYTES: usize = 60_000;

/// A branch name git and GitHub both accept, conservatively.
pub(crate) fn is_valid_branch(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with(['/', '-', '.'])
        && !name.ends_with(['/', '.'])
        && !name.ends_with(".lock")
        && !name.contains("..")
        && !name.contains("//")
        && !name.contains("@{")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
}

#[derive(Debug)]
pub(crate) struct PullDraft {
    pub(crate) repo: String,
    pub(crate) head: String,
    pub(crate) base: String,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) draft: bool,
}

pub(crate) fn parse_pull(arguments: &Value) -> Result<PullDraft, String> {
    let field =
        |name: &str| required_str(arguments, name).map_err(|_| format!("{name} is required"));
    let repo = field("repo")?;
    if !is_valid_owner_repo(repo) {
        return Err("repo must be in owner/repo format".into());
    }
    let head = field("head")?;
    let base = field("base")?;
    for (name, branch) in [("head", head), ("base", base)] {
        if !is_valid_branch(branch) {
            return Err(format!("{name} is not a valid branch name"));
        }
    }
    if head == base {
        return Err("head and base must differ".into());
    }
    let body = arguments
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if body.len() > MAX_BODY_BYTES {
        return Err(format!("body is over {MAX_BODY_BYTES} bytes"));
    }
    Ok(PullDraft {
        repo: repo.to_string(),
        head: head.to_string(),
        base: base.to_string(),
        title: field("title")?.to_string(),
        body: body.to_string(),
        draft: arguments
            .get("draft")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

pub(crate) async fn open(client: &GitHubClient, pull: PullDraft) -> Result<Value, String> {
    let owner = pull.repo.split('/').next().unwrap_or_default();
    let existing = client
        .open_pull_requests_from(&pull.repo, &format!("{owner}:{}", pull.head))
        .await?;
    if let Some(existing) = existing.first() {
        return Ok(json!({
            "action": "existing",
            "number": existing.number,
            "url": existing.html_url,
        }));
    }
    let created = client
        .create_pull_request(
            &pull.repo,
            &json!({
                "title": pull.title,
                "head": pull.head,
                "base": pull.base,
                "body": pull.body,
                "draft": pull.draft,
            }),
        )
        .await?;
    Ok(json!({
        "action": "created",
        "number": created.number,
        "url": created.html_url,
        "draft": created.draft,
    }))
}

pub struct CreateGitHubPullRequestTool;

#[async_trait]
impl Tool for CreateGitHubPullRequestTool {
    fn name(&self) -> &str {
        "create_github_pull_request"
    }

    fn description(&self) -> &str {
        "Open a pull request (draft by default) from a branch you already pushed to the same repository. \
         Returns the existing pull request when one is already open from that branch."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "repo": { "type": "string", "description": "Repository in owner/repo format." },
                "head": { "type": "string", "description": "Branch in the same repository that holds the change." },
                "base": { "type": "string", "description": "Branch to merge into, usually the default branch." },
                "title": { "type": "string", "description": "Pull request title." },
                "body": { "type": "string", "description": "Pull request description in GitHub Markdown." },
                "draft": { "type": "boolean", "description": "Open as a draft (default true)." }
            },
            "required": ["repo", "head", "base", "title"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("create_github_pull_request requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let pull = match parse_pull(&arguments) {
            Ok(pull) => pull,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        let client = match github_token(context).await {
            Ok(token) => crate::tools::github_client(token),
            Err(e) => return e,
        };
        match open(&client, pull).await {
            Ok(result) => ToolExecutionResult::success(result),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn arguments() -> Value {
        json!({
            "repo": "acme/app",
            "head": "everruns/fix-sqli-users",
            "base": "main",
            "title": "Fix SQL injection in users query"
        })
    }

    #[test]
    fn branch_names_are_checked() {
        for good in ["main", "everruns/fix-1", "release-1.2"] {
            assert!(is_valid_branch(good), "{good}");
        }
        for bad in [
            "", "-x", "a..b", "a b", "a//b", "x.lock", "a/", "a@{1}", "a:b", "a~1",
        ] {
            assert!(!is_valid_branch(bad), "{bad}");
        }
        let mut same = arguments();
        same["head"] = json!("main");
        assert!(parse_pull(&same).unwrap_err().contains("differ"));
        let mut fork = arguments();
        fork["head"] = json!("mallory:main");
        assert!(parse_pull(&fork).is_err());
        assert!(parse_pull(&arguments()).unwrap().draft);
    }

    #[tokio::test]
    async fn opens_a_draft_once_per_branch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .and(query_param("head", "acme:everruns/fix-sqli-users"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/app/pulls"))
            .and(body_partial_json(json!({"draft": true, "base": "main"})))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({
                "number": 12, "html_url": "https://github.com/acme/app/pull/12", "draft": true
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url("token".into(), server.uri());
        let result = open(&client, parse_pull(&arguments()).unwrap())
            .await
            .unwrap();
        assert_eq!(result["action"], "created");
        assert_eq!(result["number"], 12);
    }

    #[tokio::test]
    async fn returns_the_open_pull_request_for_the_branch() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"number": 8, "html_url": "https://github.com/acme/app/pull/8", "draft": true}
            ])))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201))
            .expect(0)
            .mount(&server)
            .await;
        let client = GitHubClient::with_base_url("token".into(), server.uri());
        let result = open(&client, parse_pull(&arguments()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            (result["action"].as_str(), result["number"].as_u64()),
            (Some("existing"), Some(8))
        );
    }
}
