//! `submit_github_pull_request_review`: one review with inline comments.
//!
//! Decisions:
//! - The review is created and submitted in one request, so a failed run never
//!   leaves a pending review behind that only the App could see or discard.
//! - Only `COMMENT` and `REQUEST_CHANGES`. An agent that can approve could
//!   satisfy branch protection on its own say-so, and a diff is untrusted
//!   input that can talk the model into anything.
//! - Re-push dedupe is deterministic, not left to the model: every inline
//!   comment carries a hidden `<!-- everruns:finding:<key> -->` line, and a
//!   comment whose key the App already posted on this pull request is dropped.
//!   The key defaults to a hash of path and body, so a redelivered run posts
//!   nothing new; agents pass a stable slug to also match a reworded finding.
//! - A review the App already submitted for the same head commit makes the
//!   call a no-op, so webhook redeliveries and retries do not stack reviews.
//! - Only bot-authored comments and reviews count as ours. A pull request
//!   author can paste a marker into their own comment; GitHub never lets a
//!   person post as `Bot`, so that cannot suppress a finding.
//! - Comments on lines outside the diff would make GitHub reject the whole
//!   review (422). They are moved into the review body instead.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use everruns_contracts::tool_types::ToolHints;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use serde_json::{Value, json};

use crate::client::{GitHubClient, PullRequestFile};
use crate::pull_requests::{github_token, target, target_schema, valid_marker};

const MAX_COMMENTS: usize = 50;
const MAX_COMMENT_BYTES: usize = 16_000;
const MAX_BODY_BYTES: usize = 60_000;
const MAX_PAGES: u32 = 10;
const FINDING_PREFIX: &str = "<!-- everruns:finding:";
const REVIEW_PREFIX: &str = "<!-- everruns:review:";

/// Which side of the diff a comment anchors to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Side {
    Left,
    Right,
}

impl Side {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.map(str::trim).unwrap_or("RIGHT") {
            "RIGHT" | "right" => Ok(Self::Right),
            "LEFT" | "left" => Ok(Self::Left),
            other => Err(format!("side must be RIGHT or LEFT, got {other:?}")),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
        }
    }
}

/// Lines a review comment may anchor to, per file and side: every added or
/// context line on the right, every removed or context line on the left.
#[derive(Debug, Default)]
pub(crate) struct DiffLines {
    lines: HashMap<String, HashSet<(Side, u64)>>,
}

impl DiffLines {
    pub(crate) fn from_files(files: &[PullRequestFile]) -> Self {
        let mut lines = HashMap::new();
        for file in files {
            if let Some(patch) = &file.patch {
                lines.insert(file.filename.clone(), commentable_lines(patch));
            }
        }
        Self { lines }
    }

    pub(crate) fn contains(&self, path: &str, side: Side, line: u64) -> bool {
        self.lines
            .get(path)
            .is_some_and(|lines| lines.contains(&(side, line)))
    }
}

/// Parse unified diff hunks into the (side, line) pairs GitHub accepts.
pub(crate) fn commentable_lines(patch: &str) -> HashSet<(Side, u64)> {
    let mut lines = HashSet::new();
    let (mut old, mut new) = (0u64, 0u64);
    let mut in_hunk = false;
    for line in patch.lines() {
        if let Some(header) = line.strip_prefix("@@ ") {
            match parse_hunk_header(header) {
                Some((old_start, new_start)) => {
                    (old, new) = (old_start, new_start);
                    in_hunk = true;
                }
                None => in_hunk = false,
            }
            continue;
        }
        if !in_hunk {
            continue;
        }
        match line.as_bytes().first() {
            Some(b'+') => {
                lines.insert((Side::Right, new));
                new += 1;
            }
            Some(b'-') => {
                lines.insert((Side::Left, old));
                old += 1;
            }
            Some(b'\\') => {} // "\ No newline at end of file"
            _ => {
                lines.insert((Side::Right, new));
                lines.insert((Side::Left, old));
                new += 1;
                old += 1;
            }
        }
    }
    lines
}

/// `-a,b +c,d @@ ...` → `(a, c)`.
fn parse_hunk_header(header: &str) -> Option<(u64, u64)> {
    let mut parts = header.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let start = |range: &str| range.split(',').next()?.parse::<u64>().ok();
    Some((start(old)?, start(new)?))
}

/// FNV-1a: a stable, dependency-free fingerprint. Not a security boundary;
/// only bot-authored comments are ever matched against it.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn default_key(path: &str, body: &str) -> String {
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("{:016x}", fnv1a(&format!("{path}\n{normalized}")))
}

fn finding_line(key: &str) -> String {
    format!("{FINDING_PREFIX}{key} -->")
}

/// The finding key a managed comment starts with, if any.
fn finding_key(body: &str) -> Option<&str> {
    body.strip_prefix(FINDING_PREFIX)?
        .split_once(" -->")
        .map(|(key, _)| key)
}

#[derive(Debug)]
pub(crate) struct DraftComment {
    pub(crate) path: String,
    pub(crate) line: u64,
    pub(crate) start_line: Option<u64>,
    pub(crate) side: Side,
    pub(crate) body: String,
    pub(crate) key: String,
}

pub(crate) fn parse_comments(arguments: &Value) -> Result<Vec<DraftComment>, String> {
    let Some(raw) = arguments.get("comments") else {
        return Ok(Vec::new());
    };
    let raw = raw.as_array().ok_or("comments must be an array")?;
    if raw.len() > MAX_COMMENTS {
        return Err(format!("At most {MAX_COMMENTS} inline comments per review"));
    }
    raw.iter()
        .enumerate()
        .map(|(index, comment)| {
            let field = |name: &str| comment.get(name).and_then(Value::as_str).map(str::trim);
            let path = field("path")
                .filter(|path| !path.is_empty())
                .ok_or_else(|| format!("comments[{index}].path is required"))?;
            let body = field("body")
                .filter(|body| !body.is_empty())
                .ok_or_else(|| format!("comments[{index}].body is required"))?;
            if body.len() > MAX_COMMENT_BYTES {
                return Err(format!(
                    "comments[{index}].body is over {MAX_COMMENT_BYTES} bytes"
                ));
            }
            let line = comment
                .get("line")
                .and_then(Value::as_u64)
                .filter(|line| *line > 0)
                .ok_or_else(|| format!("comments[{index}].line must be a positive integer"))?;
            let start_line = comment
                .get("start_line")
                .and_then(Value::as_u64)
                .filter(|start| *start > 0 && *start < line);
            let side =
                Side::parse(field("side")).map_err(|e| format!("comments[{index}].{e}"))?;
            let key = match field("key").filter(|key| !key.is_empty()) {
                Some(key) if valid_marker(key) => key.to_string(),
                Some(_) => {
                    return Err(format!(
                        "comments[{index}].key may only contain letters, digits, '-', '_' and '.' (max 64)"
                    ));
                }
                None => default_key(path, body),
            };
            Ok(DraftComment {
                path: path.to_string(),
                line,
                start_line,
                side,
                body: body.to_string(),
                key,
            })
        })
        .collect()
}

/// What a review submission will contain once duplicates and out-of-diff
/// comments are sorted out.
#[derive(Debug, Default)]
pub(crate) struct ReviewPlan {
    pub(crate) inline: Vec<Value>,
    pub(crate) outside_diff: Vec<String>,
    pub(crate) duplicates: Vec<String>,
}

pub(crate) fn plan_review(
    drafts: Vec<DraftComment>,
    diff: &DiffLines,
    posted_keys: &HashSet<String>,
) -> ReviewPlan {
    let mut plan = ReviewPlan::default();
    let mut seen = HashSet::new();
    for draft in drafts {
        if posted_keys.contains(&draft.key) || !seen.insert(draft.key.clone()) {
            plan.duplicates.push(draft.key);
            continue;
        }
        let anchored = diff.contains(&draft.path, draft.side, draft.line)
            && draft
                .start_line
                .is_none_or(|start| diff.contains(&draft.path, draft.side, start));
        if !anchored {
            plan.outside_diff.push(format!(
                "{}\n**`{}` line {}**: {}",
                finding_line(&draft.key),
                draft.path,
                draft.line,
                draft.body
            ));
            continue;
        }
        let mut comment = json!({
            "path": draft.path,
            "line": draft.line,
            "side": draft.side.as_str(),
            "body": format!("{}\n{}", finding_line(&draft.key), draft.body),
        });
        if let Some(start) = draft.start_line {
            comment["start_line"] = json!(start);
            comment["start_side"] = json!(draft.side.as_str());
        }
        plan.inline.push(comment);
    }
    plan
}

/// Finding keys the App already posted on this pull request, inline or in a
/// review body. Only bot-authored text counts.
fn posted_keys(
    comments: &[crate::client::ReviewComment],
    reviews: &[crate::client::Review],
) -> HashSet<String> {
    let mut keys = HashSet::new();
    for comment in comments {
        if comment.user.as_ref().is_some_and(|user| user.is_bot())
            && let Some(key) = comment.body.as_deref().and_then(finding_key)
        {
            keys.insert(key.to_string());
        }
    }
    for review in reviews {
        if !review.user.as_ref().is_some_and(|user| user.is_bot()) {
            continue;
        }
        for line in review.body.as_deref().unwrap_or_default().lines() {
            if let Some(key) = finding_key(line.trim_start()) {
                keys.insert(key.to_string());
            }
        }
    }
    keys
}

fn review_marker(commit: &str) -> String {
    format!("{REVIEW_PREFIX}{commit} -->")
}

pub struct SubmitGitHubReviewTool;

#[async_trait]
impl Tool for SubmitGitHubReviewTool {
    fn name(&self) -> &str {
        "submit_github_pull_request_review"
    }

    fn description(&self) -> &str {
        "Submit a review on a GitHub pull request with a summary and inline comments on changed lines. \
         Comments this agent already posted on the pull request (same key) are skipped, comments outside \
         the diff go into the summary, and a second review of the same head commit is skipped. Cannot approve."
    }

    fn parameters_schema(&self) -> Value {
        let mut schema = target_schema(json!({
            "body": {
                "type": "string",
                "description": "Review summary in GitHub Markdown. Keep it short; findings go in comments."
            },
            "event": {
                "type": "string",
                "enum": ["COMMENT", "REQUEST_CHANGES"],
                "description": "COMMENT (default) or REQUEST_CHANGES for blocking problems."
            },
            "commit_id": {
                "type": "string",
                "description": "Head commit SHA you reviewed (from get_github_pull_request). Defaults to the current head."
            },
            "comments": {
                "type": "array",
                "maxItems": MAX_COMMENTS,
                "description": "Inline comments on lines of the diff.",
                "items": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "File path as it appears in the diff." },
                        "line": { "type": "integer", "minimum": 1, "description": "Line number in the new file (RIGHT) or the old file (LEFT)." },
                        "start_line": { "type": "integer", "minimum": 1, "description": "First line of a multi-line comment." },
                        "side": { "type": "string", "enum": ["RIGHT", "LEFT"], "description": "RIGHT (default) for added or unchanged lines, LEFT for removed lines." },
                        "body": { "type": "string", "description": "The finding in GitHub Markdown." },
                        "key": { "type": "string", "description": "Stable slug naming this finding (letters, digits, - _ .). Reuse it for the same problem on later pushes so it is not posted twice." }
                    },
                    "required": ["path", "line", "body"],
                    "additionalProperties": false
                }
            }
        }));
        schema["required"] = json!(["repo", "number"]);
        schema
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "submit_github_pull_request_review requires session context.",
        )
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
        let body = arguments
            .get("body")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        let event = match arguments.get("event").and_then(Value::as_str) {
            None | Some("COMMENT") => "COMMENT",
            Some("REQUEST_CHANGES") => "REQUEST_CHANGES",
            Some(other) => {
                return ToolExecutionResult::tool_error(format!(
                    "event must be COMMENT or REQUEST_CHANGES, got {other:?}; this tool cannot approve"
                ));
            }
        };
        let drafts = match parse_comments(&arguments) {
            Ok(drafts) => drafts,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        if body.is_empty() && drafts.is_empty() {
            return ToolExecutionResult::tool_error("Provide a body, comments, or both");
        }
        let client = match github_token(context).await {
            Ok(token) => crate::tools::github_client(token),
            Err(e) => return e,
        };
        match submit(&client, &repo, number, &arguments, body, event, drafts).await {
            Ok(result) => ToolExecutionResult::success(result),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

pub(crate) async fn submit(
    client: &GitHubClient,
    repo: &str,
    number: u64,
    arguments: &Value,
    body: String,
    event: &str,
    drafts: Vec<DraftComment>,
) -> Result<Value, String> {
    let commit = match arguments
        .get("commit_id")
        .and_then(Value::as_str)
        .map(str::trim)
    {
        Some(commit) if !commit.is_empty() => {
            if !commit.bytes().all(|b| b.is_ascii_hexdigit()) || commit.len() > 64 {
                return Err("commit_id must be a hex commit SHA".into());
            }
            commit.to_string()
        }
        _ => client.pull_request(repo, number).await?.head.sha,
    };
    let reviews = client.reviews(repo, number, MAX_PAGES).await?;
    let marker = review_marker(&commit);
    if let Some(existing) = reviews.iter().find(|review| {
        review.user.as_ref().is_some_and(|user| user.is_bot())
            && review
                .body
                .as_deref()
                .is_some_and(|text| text.starts_with(&marker))
    }) {
        return Ok(json!({
            "action": "skipped",
            "reason": "already_reviewed",
            "commit_id": commit,
            "review_id": existing.id,
            "url": existing.html_url,
        }));
    }
    let comments = client.review_comments(repo, number, MAX_PAGES).await?;
    let files = client
        .pull_request_files_with_patches(repo, number, MAX_PAGES)
        .await?;
    let plan = plan_review(
        drafts,
        &DiffLines::from_files(&files),
        &posted_keys(&comments, &reviews),
    );
    if plan.inline.is_empty() && plan.outside_diff.is_empty() && body.is_empty() {
        return Ok(json!({
            "action": "skipped",
            "reason": "no_new_comments",
            "commit_id": commit,
            "duplicates": plan.duplicates,
        }));
    }

    let mut text = format!("{marker}\n{body}");
    if !plan.outside_diff.is_empty() {
        text.push_str("\n\n**Comments outside the diff**\n\n");
        text.push_str(&plan.outside_diff.join("\n\n"));
    }
    if text.len() > MAX_BODY_BYTES {
        return Err(format!(
            "Review body is {} bytes; keep it under {MAX_BODY_BYTES}",
            text.len()
        ));
    }
    let review = client
        .create_review(
            repo,
            number,
            &json!({
                "commit_id": commit,
                "body": text,
                "event": event,
                "comments": plan.inline,
            }),
        )
        .await?;
    Ok(json!({
        "action": "submitted",
        "review_id": review.id,
        "url": review.html_url,
        "commit_id": commit,
        "event": event,
        "inline_comments": plan.inline.len(),
        "moved_to_body": plan.outside_diff.len(),
        "duplicates": plan.duplicates,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Author, Review, ReviewComment};

    const PATCH: &str = "@@ -10,4 +10,5 @@ fn main() {\n context\n-old\n+new\n+added\n context2\n\\ No newline at end of file\n@@ -40,2 +41,2 @@\n-gone\n+here\n tail";

    fn file(path: &str, patch: Option<&str>) -> PullRequestFile {
        PullRequestFile {
            filename: path.into(),
            status: "modified".into(),
            additions: 0,
            deletions: 0,
            patch: patch.map(String::from),
        }
    }

    fn bot() -> Option<Author> {
        Some(Author {
            login: "reviewer[bot]".into(),
            kind: "Bot".into(),
        })
    }

    fn human() -> Option<Author> {
        Some(Author {
            login: "mallory".into(),
            kind: "User".into(),
        })
    }

    #[test]
    fn hunks_map_to_commentable_lines() {
        let lines = commentable_lines(PATCH);
        // Right side: context 10, new 11, added 12, context2 13, here 41, tail 42.
        for line in [10, 11, 12, 13, 41, 42] {
            assert!(lines.contains(&(Side::Right, line)), "right {line}");
        }
        // Left side: context 10, old 11, context2 12, gone 40, tail 41.
        for line in [10, 11, 12, 40, 41] {
            assert!(lines.contains(&(Side::Left, line)), "left {line}");
        }
        assert!(!lines.contains(&(Side::Right, 14)));
        assert!(!lines.contains(&(Side::Right, 1)));
        assert!(!lines.contains(&(Side::Left, 13)));
    }

    #[test]
    fn parses_hunk_headers_with_and_without_counts() {
        assert_eq!(parse_hunk_header("-1 +1 @@"), Some((1, 1)));
        assert_eq!(parse_hunk_header("-3,7 +4,9 @@ ctx"), Some((3, 4)));
        assert_eq!(parse_hunk_header("garbage"), None);
    }

    #[test]
    fn comments_are_validated() {
        for (arguments, needle) in [
            (json!({"comments": [{"line": 1, "body": "x"}]}), "path"),
            (
                json!({"comments": [{"path": "a", "line": 0, "body": "x"}]}),
                "line",
            ),
            (
                json!({"comments": [{"path": "a", "line": 1, "body": " "}]}),
                "body",
            ),
            (
                json!({"comments": [{"path": "a", "line": 1, "body": "x", "side": "UP"}]}),
                "side",
            ),
            (
                json!({"comments": [{"path": "a", "line": 1, "body": "x", "key": "a -->b"}]}),
                "key",
            ),
            (json!({"comments": "nope"}), "array"),
        ] {
            let err = parse_comments(&arguments).unwrap_err();
            assert!(err.contains(needle), "{arguments}: {err}");
        }
        let too_many: Vec<Value> = (0..=MAX_COMMENTS)
            .map(|i| json!({"path": "a", "line": i + 1, "body": "x"}))
            .collect();
        assert!(parse_comments(&json!({ "comments": too_many })).is_err());
    }

    #[test]
    fn default_key_ignores_whitespace_but_not_path() {
        assert_eq!(
            default_key("a.rs", "Bad  thing\n"),
            default_key("a.rs", "Bad thing")
        );
        assert_ne!(
            default_key("a.rs", "Bad thing"),
            default_key("b.rs", "Bad thing")
        );
    }

    #[test]
    fn plan_skips_posted_keys_and_moves_out_of_diff_comments() {
        let diff =
            DiffLines::from_files(&[file("src/main.rs", Some(PATCH)), file("logo.png", None)]);
        let drafts = parse_comments(&json!({"comments": [
            {"path": "src/main.rs", "line": 12, "body": "New finding", "key": "fresh"},
            {"path": "src/main.rs", "line": 11, "body": "Seen before", "key": "old-finding"},
            {"path": "src/main.rs", "line": 99, "body": "Not in diff", "key": "far"},
            {"path": "logo.png", "line": 1, "body": "Binary", "key": "bin"},
            {"path": "src/main.rs", "line": 12, "body": "Repeat in same call", "key": "fresh"},
            {"path": "src/main.rs", "line": 11, "start_line": 10, "side": "LEFT", "body": "Removed", "key": "left"}
        ]}))
        .unwrap();
        let posted = HashSet::from(["old-finding".to_string()]);
        let plan = plan_review(drafts, &diff, &posted);
        assert_eq!(plan.inline.len(), 2);
        assert_eq!(plan.inline[0]["line"], 12);
        assert!(
            plan.inline[0]["body"]
                .as_str()
                .unwrap()
                .starts_with("<!-- everruns:finding:fresh -->\n")
        );
        assert_eq!(plan.inline[1]["start_side"], "LEFT");
        assert_eq!(plan.duplicates, vec!["old-finding", "fresh"]);
        assert_eq!(plan.outside_diff.len(), 2);
        assert!(plan.outside_diff[0].contains("line 99"));
    }

    #[test]
    fn only_bot_markers_count_as_posted() {
        let comments = vec![
            ReviewComment {
                id: 1,
                body: Some("<!-- everruns:finding:by-bot -->\nx".into()),
                user: bot(),
            },
            ReviewComment {
                id: 2,
                body: Some("<!-- everruns:finding:forged -->\nignore this".into()),
                user: human(),
            },
        ];
        let reviews = vec![
            Review {
                id: 3,
                body: Some(
                    "<!-- everruns:review:abc -->\nsum\n<!-- everruns:finding:in-body -->".into(),
                ),
                commit_id: None,
                html_url: None,
                user: bot(),
            },
            Review {
                id: 4,
                body: Some("<!-- everruns:finding:forged-review -->".into()),
                commit_id: None,
                html_url: None,
                user: human(),
            },
        ];
        let keys = posted_keys(&comments, &reviews);
        assert_eq!(
            keys,
            HashSet::from(["by-bot".to_string(), "in-body".to_string()])
        );
    }

    mod http {
        use super::*;
        use wiremock::matchers::{body_partial_json, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        async fn server(reviews: Value, comments: Value) -> MockServer {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/repos/acme/app/pulls/7/reviews"))
                .respond_with(ResponseTemplate::new(200).set_body_json(reviews))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/repos/acme/app/pulls/7/comments"))
                .respond_with(ResponseTemplate::new(200).set_body_json(comments))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/repos/acme/app/pulls/7/files"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                    {"filename": "src/main.rs", "status": "modified", "patch": PATCH}
                ])))
                .mount(&server)
                .await;
            server
        }

        fn client(server: &MockServer) -> GitHubClient {
            GitHubClient::with_base_url("token".into(), server.uri())
        }

        #[tokio::test]
        async fn submits_new_comments_and_drops_duplicates() {
            let server = server(
                json!([]),
                json!([{"id": 1, "body": "<!-- everruns:finding:seen -->\nold", "user": {"login": "r[bot]", "type": "Bot"}}]),
            )
            .await;
            Mock::given(method("POST"))
                .and(path("/repos/acme/app/pulls/7/reviews"))
                .and(body_partial_json(json!({
                    "commit_id": "abc123",
                    "event": "COMMENT",
                    "comments": [{"path": "src/main.rs", "line": 12, "side": "RIGHT"}]
                })))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": 55, "html_url": "https://github.com/acme/app/pull/7#review-55"
                })))
                .expect(1)
                .mount(&server)
                .await;
            let arguments = json!({"commit_id": "abc123"});
            let drafts = parse_comments(&json!({"comments": [
                {"path": "src/main.rs", "line": 12, "body": "new", "key": "fresh"},
                {"path": "src/main.rs", "line": 11, "body": "again", "key": "seen"}
            ]}))
            .unwrap();
            let result = submit(
                &client(&server),
                "acme/app",
                7,
                &arguments,
                "Summary".into(),
                "COMMENT",
                drafts,
            )
            .await
            .unwrap();
            assert_eq!(result["action"], "submitted");
            assert_eq!(result["inline_comments"], 1);
            assert_eq!(result["duplicates"], json!(["seen"]));
        }

        #[tokio::test]
        async fn same_commit_is_reviewed_once() {
            let server = server(
                json!([{"id": 9, "body": "<!-- everruns:review:abc123 -->\nDone", "user": {"login": "r[bot]", "type": "Bot"}}]),
                json!([]),
            )
            .await;
            Mock::given(method("POST"))
                .and(path("/repos/acme/app/pulls/7/reviews"))
                .respond_with(ResponseTemplate::new(200))
                .expect(0)
                .mount(&server)
                .await;
            let result = submit(
                &client(&server),
                "acme/app",
                7,
                &json!({"commit_id": "abc123"}),
                "Summary".into(),
                "COMMENT",
                Vec::new(),
            )
            .await
            .unwrap();
            assert_eq!(result["reason"], "already_reviewed");
            assert_eq!(result["review_id"], 9);
        }

        #[tokio::test]
        async fn a_human_review_with_the_marker_does_not_block() {
            let server = server(
                json!([{"id": 9, "body": "<!-- everruns:review:abc123 -->", "user": {"login": "m", "type": "User"}}]),
                json!([]),
            )
            .await;
            Mock::given(method("POST"))
                .and(path("/repos/acme/app/pulls/7/reviews"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 56})))
                .expect(1)
                .mount(&server)
                .await;
            let result = submit(
                &client(&server),
                "acme/app",
                7,
                &json!({"commit_id": "abc123"}),
                "Summary".into(),
                "COMMENT",
                Vec::new(),
            )
            .await
            .unwrap();
            assert_eq!(result["action"], "submitted");
        }

        #[tokio::test]
        async fn nothing_new_posts_nothing() {
            let server = server(
                json!([]),
                json!([{"id": 1, "body": "<!-- everruns:finding:seen -->\nold", "user": {"login": "r[bot]", "type": "Bot"}}]),
            )
            .await;
            Mock::given(method("POST"))
                .and(path("/repos/acme/app/pulls/7/reviews"))
                .respond_with(ResponseTemplate::new(200))
                .expect(0)
                .mount(&server)
                .await;
            let drafts = parse_comments(
                &json!({"comments": [{"path": "src/main.rs", "line": 11, "body": "again", "key": "seen"}]}),
            )
            .unwrap();
            let result = submit(
                &client(&server),
                "acme/app",
                7,
                &json!({"commit_id": "abc123"}),
                String::new(),
                "COMMENT",
                drafts,
            )
            .await
            .unwrap();
            assert_eq!(result["reason"], "no_new_comments");
        }
    }
}
