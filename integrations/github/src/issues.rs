//! `upsert_github_issue`: one issue per finding, keyed by a fingerprint.
//!
//! Decisions:
//! - A scheduled scan finds the same problem every run. The issue body starts
//!   with `<!-- everruns:finding:<fingerprint> -->`; a bot-authored issue with
//!   that line is the finding's issue. An open one is updated in place, a
//!   closed one is left closed (someone fixed it or decided not to), and only
//!   an unknown fingerprint opens a new issue.
//! - Only bot-authored issues match, so a person cannot pre-file an issue with
//!   the marker to swallow a finding.
//! - With `private_only` (the capability's `private_issues_only` setting) the
//!   tool refuses public repositories: a vulnerability filed as a public issue
//!   is a disclosure. It is enforced here, not in the prompt, because the
//!   scanned code is untrusted and could talk the model out of the rule.
//! - The lookup reads the newest 1,000 issues. A finding older than that
//!   could be filed twice on a very busy repository; the cost is a duplicate
//!   issue, not a lost finding.

use async_trait::async_trait;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::ToolHints;
use serde_json::{Value, json};

use crate::client::{GitHubClient, Issue};
use crate::pull_requests::{github_token, valid_marker};
use crate::tools::{is_valid_owner_repo, required_str};

const MAX_ISSUE_PAGES: u32 = 10;
const MAX_TITLE_CHARS: usize = 256;
const MAX_BODY_BYTES: usize = 60_000;
const MAX_LABELS: usize = 10;

fn fingerprint_line(fingerprint: &str) -> String {
    format!("<!-- everruns:finding:{fingerprint} -->")
}

pub struct UpsertGitHubIssueTool {
    private_only: bool,
}

impl UpsertGitHubIssueTool {
    pub fn new(private_only: bool) -> Self {
        Self { private_only }
    }
}

#[derive(Debug)]
pub(crate) struct IssueDraft {
    pub(crate) repo: String,
    pub(crate) fingerprint: String,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) labels: Vec<String>,
}

pub(crate) fn parse_issue(arguments: &Value) -> Result<IssueDraft, String> {
    let field =
        |name: &str| required_str(arguments, name).map_err(|_| format!("{name} is required"));
    let repo = field("repo")?;
    if !is_valid_owner_repo(repo) {
        return Err("repo must be in owner/repo format".into());
    }
    let fingerprint = field("fingerprint")?;
    if !valid_marker(fingerprint) {
        return Err(
            "fingerprint may only contain letters, digits, '-', '_' and '.' (max 64)".into(),
        );
    }
    let title = field("title")?;
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("title is over {MAX_TITLE_CHARS} characters"));
    }
    let body = format!("{}\n{}", fingerprint_line(fingerprint), field("body")?);
    if body.len() > MAX_BODY_BYTES {
        return Err(format!("body is over {MAX_BODY_BYTES} bytes"));
    }
    let labels: Vec<String> = arguments
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|label| !label.is_empty() && label.len() <= 50)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    if labels.len() > MAX_LABELS {
        return Err(format!("At most {MAX_LABELS} labels"));
    }
    Ok(IssueDraft {
        repo: repo.to_string(),
        fingerprint: fingerprint.to_string(),
        title: title.to_string(),
        body,
        labels,
    })
}

fn find_finding<'a>(issues: &'a [Issue], fingerprint: &str) -> Option<&'a Issue> {
    let marker = fingerprint_line(fingerprint);
    issues.iter().find(|issue| {
        issue.pull_request.is_none()
            && issue.user.as_ref().is_some_and(|user| user.is_bot())
            && issue
                .body
                .as_deref()
                .is_some_and(|body| body.starts_with(&marker))
    })
}

pub(crate) async fn upsert(
    client: &GitHubClient,
    draft: IssueDraft,
    private_only: bool,
) -> Result<Value, String> {
    if private_only && client.repository(&draft.repo).await?.is_public() {
        return Err(format!(
            "{} is public; this agent only files findings on private repositories. \
             Report the finding privately instead (for example GitHub private vulnerability reporting).",
            draft.repo
        ));
    }
    let issues = client.issues(&draft.repo, MAX_ISSUE_PAGES).await?;
    match find_finding(&issues, &draft.fingerprint) {
        Some(issue) if issue.state == "closed" => Ok(json!({
            "action": "unchanged",
            "reason": "closed",
            "number": issue.number,
            "url": issue.html_url,
        })),
        Some(issue) => {
            let updated = client
                .update_issue(
                    &draft.repo,
                    issue.number,
                    &json!({ "title": draft.title, "body": draft.body }),
                )
                .await?;
            Ok(json!({
                "action": "updated",
                "number": updated.number,
                "url": updated.html_url,
            }))
        }
        None => {
            let mut request = json!({ "title": draft.title, "body": draft.body });
            if !draft.labels.is_empty() {
                request["labels"] = json!(draft.labels);
            }
            let created = client.create_issue(&draft.repo, &request).await?;
            Ok(json!({
                "action": "created",
                "number": created.number,
                "url": created.html_url,
            }))
        }
    }
}

#[async_trait]
impl Tool for UpsertGitHubIssueTool {
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
                    "Saving GitHub issue",
                    "Saved GitHub issue",
                    "Could not save GitHub issue",
                ),
                (
                    "Зберігаю задачу GitHub",
                    "Зберіг задачу GitHub",
                    "Не вдалося зберегти задачу GitHub",
                ),
                &["title", "repo"],
            ),
        )
    }

    fn name(&self) -> &str {
        "upsert_github_issue"
    }

    fn description(&self) -> &str {
        "File a finding as a GitHub issue keyed by a stable fingerprint. An open issue this agent filed \
         for the same fingerprint is updated instead of duplicated; a closed one is left closed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "repo": { "type": "string", "description": "Repository in owner/repo format." },
                "fingerprint": {
                    "type": "string",
                    "description": "Stable identifier for the finding (letters, digits, - _ .; max 64), e.g. `sqli-src-db-users-rs-find`. Derive it from the rule and location, not the wording, so the next scan produces the same one."
                },
                "title": { "type": "string", "description": "Issue title." },
                "body": { "type": "string", "description": "Issue body in GitHub Markdown." },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "maxItems": MAX_LABELS,
                    "description": "Labels for a new issue (ignored when updating)."
                }
            },
            "required": ["repo", "fingerprint", "title", "body"],
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
        ToolExecutionResult::tool_error("upsert_github_issue requires session context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let draft = match parse_issue(&arguments) {
            Ok(draft) => draft,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        let client = match github_token(context).await {
            Ok(token) => crate::tools::github_client(token),
            Err(e) => return e,
        };
        match upsert(&client, draft, self.private_only).await {
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
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn draft() -> IssueDraft {
        parse_issue(&json!({
            "repo": "acme/app",
            "fingerprint": "sqli-users",
            "title": "SQL injection in users query",
            "body": "Details",
            "labels": ["security"]
        }))
        .unwrap()
    }

    #[test]
    fn validates_arguments() {
        for (arguments, needle) in [
            (
                json!({"fingerprint": "a", "title": "t", "body": "b"}),
                "repo",
            ),
            (
                json!({"repo": "acme", "fingerprint": "a", "title": "t", "body": "b"}),
                "owner/repo",
            ),
            (
                json!({"repo": "acme/app", "fingerprint": "a b", "title": "t", "body": "b"}),
                "fingerprint",
            ),
            (
                json!({"repo": "acme/app", "fingerprint": "a", "body": "b"}),
                "title",
            ),
            (
                json!({"repo": "acme/app", "fingerprint": "a", "title": "t", "body": " "}),
                "body",
            ),
        ] {
            let err = parse_issue(&arguments).unwrap_err();
            assert!(err.contains(needle), "{arguments}: {err}");
        }
        assert!(
            draft()
                .body
                .starts_with("<!-- everruns:finding:sqli-users -->\n")
        );
    }

    async fn server(issues: Value, public: bool) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "full_name": "acme/app",
                "private": !public,
                "visibility": if public { "public" } else { "private" }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/issues"))
            .respond_with(ResponseTemplate::new(200).set_body_json(issues))
            .mount(&server)
            .await;
        server
    }

    fn client(server: &MockServer) -> GitHubClient {
        GitHubClient::with_base_url("token".into(), server.uri())
    }

    fn issue(number: u64, state: &str, kind: &str, pull: bool) -> Value {
        let mut issue = json!({
            "number": number,
            "html_url": format!("https://github.com/acme/app/issues/{number}"),
            "state": state,
            "body": "<!-- everruns:finding:sqli-users -->\nold",
            "user": {"login": "x", "type": kind}
        });
        if pull {
            issue["pull_request"] = json!({});
        }
        issue
    }

    #[tokio::test]
    async fn creates_when_no_bot_issue_has_the_fingerprint() {
        // A human's issue and a pull request with the marker are not ours.
        let server = server(
            json!([
                issue(1, "open", "User", false),
                issue(2, "open", "Bot", true)
            ]),
            false,
        )
        .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/app/issues"))
            .and(body_partial_json(json!({"labels": ["security"]})))
            .respond_with(ResponseTemplate::new(201).set_body_json(issue(3, "open", "Bot", false)))
            .expect(1)
            .mount(&server)
            .await;
        let result = upsert(&client(&server), draft(), true).await.unwrap();
        assert_eq!(result["action"], "created");
        assert_eq!(result["number"], 3);
    }

    #[tokio::test]
    async fn updates_the_open_finding() {
        let server = server(json!([issue(4, "open", "Bot", false)]), false).await;
        Mock::given(method("PATCH"))
            .and(path("/repos/acme/app/issues/4"))
            .respond_with(ResponseTemplate::new(200).set_body_json(issue(4, "open", "Bot", false)))
            .expect(1)
            .mount(&server)
            .await;
        let result = upsert(&client(&server), draft(), false).await.unwrap();
        assert_eq!(result["action"], "updated");
    }

    #[tokio::test]
    async fn leaves_a_closed_finding_closed() {
        let server = server(json!([issue(5, "closed", "Bot", false)]), false).await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201))
            .expect(0)
            .mount(&server)
            .await;
        let result = upsert(&client(&server), draft(), false).await.unwrap();
        assert_eq!(
            (result["action"].as_str(), result["number"].as_u64()),
            (Some("unchanged"), Some(5))
        );
    }

    #[tokio::test]
    async fn private_only_refuses_public_repositories() {
        let server = server(json!([]), true).await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(201))
            .expect(0)
            .mount(&server)
            .await;
        let err = upsert(&client(&server), draft(), true).await.unwrap_err();
        assert!(err.contains("public"), "{err}");
    }

    #[tokio::test]
    async fn public_repositories_are_fine_without_the_setting() {
        let server = server(json!([]), true).await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/app/issues"))
            .respond_with(ResponseTemplate::new(201).set_body_json(issue(6, "open", "Bot", false)))
            .expect(1)
            .mount(&server)
            .await;
        let result = upsert(&client(&server), draft(), false).await.unwrap();
        assert_eq!(result["action"], "created");
    }
}
