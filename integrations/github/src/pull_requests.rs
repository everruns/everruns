//! Host-facing tools for working on one pull request: read it, read its diff,
//! and keep one comment on it up to date.
//!
//! Decision: the comment tool upserts by a hidden HTML marker instead of always
//! posting, so an agent triggered on every push to a pull request edits its
//! earlier summary rather than stacking a new comment per push.

use async_trait::async_trait;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::ToolHints;
use serde_json::{Value, json};

use crate::tools::{
    enforce_github_network_access, get_github_token, github_client, is_valid_owner_repo,
    required_str,
};

const DEFAULT_DIFF_BYTES: u64 = 60_000;
const MAX_DIFF_BYTES: u64 = 200_000;
const MAX_LISTED_FILES: u32 = 100;
const MAX_COMMENT_BYTES: usize = 65_000;
const MAX_COMMENT_PAGES: u32 = 10;
const DEFAULT_MARKER: &str = "summary";

/// `owner/repo` and a positive pull request or issue number.
pub(crate) fn target(arguments: &Value) -> Result<(String, u64), ToolExecutionResult> {
    let repo = required_str(arguments, "repo")?;
    if !is_valid_owner_repo(repo) {
        return Err(ToolExecutionResult::tool_error(
            "repo must be in owner/repo format",
        ));
    }
    let number = arguments
        .get("number")
        .and_then(Value::as_u64)
        .filter(|number| *number > 0)
        .ok_or_else(|| ToolExecutionResult::tool_error("number must be a positive integer"))?;
    Ok((repo.to_string(), number))
}

pub(crate) async fn github_token(context: &ToolContext) -> Result<String, ToolExecutionResult> {
    enforce_github_network_access(context)?;
    get_github_token(context).await
}

pub(crate) fn target_schema(extra: Value) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": {
            "repo": {
                "type": "string",
                "description": "Repository in owner/repo format."
            },
            "number": {
                "type": "integer",
                "minimum": 1,
                "description": "Pull request (or issue) number."
            }
        },
        "required": ["repo", "number"],
        "additionalProperties": false
    });
    if let (Some(properties), Value::Object(extra)) = (schema["properties"].as_object_mut(), extra)
    {
        for (name, property) in extra {
            properties.insert(name, property);
        }
    }
    schema
}

/// The hidden line that identifies a managed comment.
pub(crate) fn marker_line(marker: &str) -> String {
    format!("<!-- everruns:{marker} -->")
}

pub(crate) fn valid_marker(marker: &str) -> bool {
    !marker.is_empty()
        && marker.len() <= 64
        && marker
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub struct GetGitHubPullRequestTool;

#[async_trait]
impl Tool for GetGitHubPullRequestTool {
    fn name(&self) -> &str {
        "get_github_pull_request"
    }

    fn description(&self) -> &str {
        "Get a GitHub pull request: title, description, author, state, branches, size, and the changed files."
    }

    fn parameters_schema(&self) -> Value {
        target_schema(json!({}))
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("get_github_pull_request requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let (repo, number) = match target(&arguments) {
            Ok(target) => target,
            Err(e) => return e,
        };
        let client = match github_token(context).await {
            Ok(token) => github_client(token),
            Err(e) => return e,
        };
        let pull = match client.pull_request(&repo, number).await {
            Ok(pull) => pull,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        let files = match client
            .pull_request_files(&repo, number, MAX_LISTED_FILES)
            .await
        {
            Ok(files) => files,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        ToolExecutionResult::success(json!({
            "repo": repo,
            "number": pull.number,
            "title": pull.title,
            "body": pull.body,
            "author": pull.user.map(|user| user.login),
            "state": pull.state,
            "draft": pull.draft,
            "merged": pull.merged,
            "url": pull.html_url,
            "base": { "ref": pull.base.name, "sha": pull.base.sha },
            "head": { "ref": pull.head.name, "sha": pull.head.sha },
            "commits": pull.commits,
            "additions": pull.additions,
            "deletions": pull.deletions,
            "changed_files": pull.changed_files,
            "files": files.iter().map(|file| json!({
                "path": file.filename,
                "status": file.status,
                "additions": file.additions,
                "deletions": file.deletions,
            })).collect::<Vec<_>>(),
            "files_truncated": pull.changed_files > files.len() as u64,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct GetGitHubPullRequestDiffTool;

#[async_trait]
impl Tool for GetGitHubPullRequestDiffTool {
    fn name(&self) -> &str {
        "get_github_pull_request_diff"
    }

    fn description(&self) -> &str {
        "Get the unified diff of a GitHub pull request, cut to a byte budget. Check `truncated` before treating it as complete."
    }

    fn parameters_schema(&self) -> Value {
        target_schema(json!({
            "max_bytes": {
                "type": "integer",
                "minimum": 1000,
                "maximum": MAX_DIFF_BYTES,
                "description": "Maximum diff bytes to return (default 60000)."
            }
        }))
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("get_github_pull_request_diff requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let (repo, number) = match target(&arguments) {
            Ok(target) => target,
            Err(e) => return e,
        };
        let max_bytes = arguments
            .get("max_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_DIFF_BYTES)
            .clamp(1000, MAX_DIFF_BYTES) as usize;
        let client = match github_token(context).await {
            Ok(token) => github_client(token),
            Err(e) => return e,
        };
        match client.pull_request_diff(&repo, number, max_bytes).await {
            Ok(diff) => ToolExecutionResult::success(json!({
                "repo": repo,
                "number": number,
                "diff": diff.text,
                "total_bytes": diff.total_bytes,
                "truncated": diff.truncated,
            })),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub struct UpsertGitHubCommentTool;

#[async_trait]
impl Tool for UpsertGitHubCommentTool {
    fn name(&self) -> &str {
        "upsert_github_comment"
    }

    fn description(&self) -> &str {
        "Post a Markdown comment on a GitHub pull request or issue, or edit the comment this tool posted there earlier under the same marker, so repeated runs keep one comment up to date."
    }

    fn parameters_schema(&self) -> Value {
        target_schema(json!({
            "body": {
                "type": "string",
                "description": "Comment body in GitHub Markdown."
            },
            "marker": {
                "type": "string",
                "description": "Identifies the managed comment (letters, digits, - _ .; default \"summary\"). Use a different marker for each kind of comment."
            }
        }))
        .as_object()
        .cloned()
        .map(|mut schema| {
            schema.insert("required".into(), json!(["repo", "number", "body"]));
            Value::Object(schema)
        })
        .unwrap_or_default()
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("upsert_github_comment requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let (repo, number) = match target(&arguments) {
            Ok(target) => target,
            Err(e) => return e,
        };
        let body = match required_str(&arguments, "body") {
            Ok(body) => body,
            Err(e) => return e,
        };
        let marker = arguments
            .get("marker")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|marker| !marker.is_empty())
            .unwrap_or(DEFAULT_MARKER);
        if !valid_marker(marker) {
            return ToolExecutionResult::tool_error(
                "marker may only contain letters, digits, '-', '_' and '.' (max 64)",
            );
        }
        let marker = marker_line(marker);
        let body = format!("{marker}\n{body}");
        if body.len() > MAX_COMMENT_BYTES {
            return ToolExecutionResult::tool_error(format!(
                "Comment is {} bytes; GitHub accepts at most {MAX_COMMENT_BYTES}",
                body.len()
            ));
        }
        let client = match github_token(context).await {
            Ok(token) => github_client(token),
            Err(e) => return e,
        };
        let result = client
            .upsert_marked_comment(&repo, number, &marker, &body, MAX_COMMENT_PAGES)
            .await;
        match result {
            Ok((comment, created)) => ToolExecutionResult::success(json!({
                "action": if created { "created" } else { "updated" },
                "comment_id": comment.id,
                "url": comment.html_url,
            })),
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
    use everruns_contracts::typed_id::SessionId;

    async fn run(tool: &dyn Tool, arguments: Value) -> ToolExecutionResult {
        tool.execute_with_context(arguments, &ToolContext::new(SessionId::new()))
            .await
    }

    #[tokio::test]
    async fn rejects_bad_targets_before_touching_github() {
        for arguments in [
            json!({"repo": "acme", "number": 1}),
            json!({"repo": "acme/../x", "number": 1}),
            json!({"repo": "acme/app", "number": 0}),
            json!({"repo": "acme/app"}),
        ] {
            let result = run(&GetGitHubPullRequestTool, arguments.clone()).await;
            assert!(
                matches!(result, ToolExecutionResult::ToolError { .. }),
                "{arguments}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn comment_rejects_bad_markers_and_empty_bodies() {
        for arguments in [
            json!({"repo": "acme/app", "number": 1, "body": "hi", "marker": "a b"}),
            json!({"repo": "acme/app", "number": 1, "body": "hi", "marker": "--><script>"}),
            json!({"repo": "acme/app", "number": 1, "body": "  "}),
        ] {
            let result = run(&UpsertGitHubCommentTool, arguments.clone()).await;
            assert!(
                matches!(result, ToolExecutionResult::ToolError { .. }),
                "{arguments}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn review_rejects_approval_and_empty_reviews_before_touching_github() {
        for arguments in [
            json!({"repo": "acme/app", "number": 1, "body": "ok", "event": "APPROVE"}),
            json!({"repo": "acme/app", "number": 1}),
            json!({"repo": "acme/app", "number": 1, "comments": [{"path": "a", "line": 1}]}),
        ] {
            let result = run(&crate::reviews::SubmitGitHubReviewTool, arguments.clone()).await;
            assert!(
                matches!(result, ToolExecutionResult::ToolError { .. }),
                "{arguments}: {result:?}"
            );
        }
    }

    #[tokio::test]
    async fn valid_calls_without_a_connection_ask_for_github() {
        for (tool, arguments) in [
            (
                &GetGitHubPullRequestTool as &dyn Tool,
                json!({"repo": "acme/app", "number": 1}),
            ),
            (
                &GetGitHubPullRequestDiffTool,
                json!({"repo": "acme/app", "number": 1, "max_bytes": 5000}),
            ),
            (
                &UpsertGitHubCommentTool,
                json!({"repo": "acme/app", "number": 1, "body": "Summary"}),
            ),
            (
                &crate::reviews::SubmitGitHubReviewTool,
                json!({"repo": "acme/app", "number": 1, "body": "Summary"}),
            ),
            (
                &crate::issues::UpsertGitHubIssueTool::new(true),
                json!({"repo": "acme/app", "fingerprint": "f", "title": "t", "body": "b"}),
            ),
            (
                &crate::fix_pull_requests::CreateGitHubPullRequestTool,
                json!({"repo": "acme/app", "head": "fix", "base": "main", "title": "t"}),
            ),
        ] {
            match run(tool, arguments).await {
                ToolExecutionResult::ConnectionRequired { provider, .. } => {
                    assert_eq!(provider, "github")
                }
                other => panic!(
                    "{}: expected connection required, got {other:?}",
                    tool.name()
                ),
            }
        }
    }

    #[test]
    fn schemas_require_the_target() {
        assert_eq!(
            UpsertGitHubCommentTool.parameters_schema()["required"],
            json!(["repo", "number", "body"])
        );
        assert_eq!(
            GetGitHubPullRequestDiffTool.parameters_schema()["required"],
            json!(["repo", "number"])
        );
        assert_eq!(marker_line("summary"), "<!-- everruns:summary -->");
    }
}
