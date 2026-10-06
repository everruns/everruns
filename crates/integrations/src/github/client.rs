//! Minimal GitHub REST client for the GitHub tools.

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSearchResponse {
    #[serde(default)]
    pub total_count: u64,
    #[serde(default)]
    pub incomplete_results: bool,
    #[serde(default)]
    pub items: Vec<CodeSearchItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSearchItem {
    pub name: String,
    pub path: String,
    pub sha: String,
    pub html_url: String,
    pub repository: SearchRepository,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSearchResponse {
    #[serde(default)]
    pub total_count: u64,
    #[serde(default)]
    pub incomplete_results: bool,
    #[serde(default)]
    pub items: Vec<IssueSearchItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSearchItem {
    pub number: u64,
    pub title: String,
    pub html_url: String,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub user: Option<SearchUser>,
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRepository {
    pub full_name: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchUser {
    pub login: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubFile {
    pub name: String,
    pub path: String,
    pub sha: String,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub download_url: Option<String>,
    #[serde(default)]
    pub encoding: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
}

impl GitHubFile {
    pub fn decoded_content(&self) -> Result<String, String> {
        let content = self
            .content
            .as_ref()
            .ok_or_else(|| "GitHub response did not include file content".to_string())?;
        let encoding = self.encoding.as_deref().unwrap_or("base64");
        if encoding != "base64" {
            return Err(format!("Unsupported GitHub file encoding: {encoding}"));
        }

        let normalized = content.lines().collect::<String>();
        let bytes = BASE64
            .decode(normalized)
            .map_err(|e| format!("Failed to decode GitHub file content: {e}"))?;
        String::from_utf8(bytes).map_err(|e| format!("GitHub file is not valid UTF-8: {e}"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub state: String,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub merged: bool,
    pub html_url: String,
    #[serde(default)]
    pub user: Option<SearchUser>,
    pub base: PullRequestRef,
    pub head: PullRequestRef,
    #[serde(default)]
    pub commits: u64,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    #[serde(default)]
    pub changed_files: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestRef {
    #[serde(rename = "ref")]
    pub name: String,
    pub sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestFile {
    pub filename: String,
    pub status: String,
    #[serde(default)]
    pub additions: u64,
    #[serde(default)]
    pub deletions: u64,
    /// Unified diff hunks for the file. Absent for binary files and for
    /// patches GitHub considers too large to inline.
    #[serde(default)]
    pub patch: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueComment {
    pub id: u64,
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
}

/// Author of a comment, review or issue. `kind` is GitHub's `type`
/// (`User`, `Bot`, `Organization`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Author {
    pub login: String,
    #[serde(rename = "type", default)]
    pub kind: String,
}

impl Author {
    /// Whether a GitHub App (or another bot account) wrote this. People cannot
    /// post as `Bot`, so a marker in a human's comment never counts as ours.
    pub fn is_bot(&self) -> bool {
        self.kind == "Bot"
    }
}

/// An inline review comment on a pull request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    pub id: u64,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub user: Option<Author>,
}

/// A submitted pull request review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Review {
    pub id: u64,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub commit_id: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub user: Option<Author>,
}

/// An issue (or pull request, which the issues API also lists).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Issue {
    pub number: u64,
    pub html_url: String,
    pub state: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub user: Option<Author>,
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub full_name: String,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub default_branch: Option<String>,
}

impl Repository {
    /// Public repositories expose every issue to the world.
    pub fn is_public(&self) -> bool {
        match self.visibility.as_deref() {
            Some(visibility) => visibility == "public",
            None => !self.private,
        }
    }
}

/// A created or existing pull request, as the create endpoint returns it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequestSummary {
    pub number: u64,
    pub html_url: String,
    #[serde(default)]
    pub draft: bool,
}

/// A diff cut to a byte budget on a UTF-8 boundary.
#[derive(Debug, Clone, PartialEq)]
pub struct CappedText {
    pub text: String,
    pub total_bytes: usize,
    pub truncated: bool,
}

pub struct GitHubClient {
    http: reqwest::Client,
    token: String,
    api_base: String,
}

impl GitHubClient {
    pub fn new(token: String) -> Self {
        Self::with_base_url(token, crate::github::GITHUB_API_BASE.to_string())
    }

    pub fn with_base_url(token: String, api_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            token,
            api_base,
        }
    }

    pub async fn search_code(
        &self,
        query: &str,
        per_page: u32,
    ) -> Result<CodeSearchResponse, String> {
        let url = format!(
            "{}/search/code?q={}&per_page={}",
            self.api_base,
            encode_query(query),
            per_page
        );
        self.get_json(&url).await
    }

    pub async fn read_file(
        &self,
        repo: &str,
        path: &str,
        reference: Option<&str>,
    ) -> Result<GitHubFile, String> {
        let mut url = format!(
            "{}/repos/{}/contents/{}",
            self.api_base,
            repo,
            encode_path(path)
        );
        if let Some(reference) = reference.filter(|r| !r.trim().is_empty()) {
            url.push_str("?ref=");
            url.push_str(&encode_query(reference));
        }
        self.get_json(&url).await
    }

    pub async fn search_issues(
        &self,
        query: &str,
        per_page: u32,
    ) -> Result<IssueSearchResponse, String> {
        let url = format!(
            "{}/search/issues?q={}&per_page={}",
            self.api_base,
            encode_query(query),
            per_page
        );
        self.get_json(&url).await
    }

    pub async fn pull_request(&self, repo: &str, number: u64) -> Result<PullRequest, String> {
        self.get_json(&format!("{}/repos/{repo}/pulls/{number}", self.api_base))
            .await
    }

    pub async fn pull_request_files(
        &self,
        repo: &str,
        number: u64,
        per_page: u32,
    ) -> Result<Vec<PullRequestFile>, String> {
        self.get_json(&format!(
            "{}/repos/{repo}/pulls/{number}/files?per_page={per_page}",
            self.api_base
        ))
        .await
    }

    /// The unified diff, cut to `max_bytes`. GitHub refuses diffs over its own
    /// limit (about 20,000 lines or 1 MB); that error is returned as is.
    pub async fn pull_request_diff(
        &self,
        repo: &str,
        number: u64,
        max_bytes: usize,
    ) -> Result<CappedText, String> {
        let url = format!("{}/repos/{repo}/pulls/{number}", self.api_base);
        let body = self
            .send(self.http.get(&url), "application/vnd.github.diff")
            .await?;
        Ok(cap_text(body, max_bytes))
    }

    /// Issue comments on an issue or pull request, oldest first, up to
    /// `max_pages` pages of 100.
    pub async fn issue_comments(
        &self,
        repo: &str,
        number: u64,
        max_pages: u32,
    ) -> Result<Vec<IssueComment>, String> {
        let mut comments = Vec::new();
        for page in 1..=max_pages {
            let batch: Vec<IssueComment> = self
                .get_json(&format!(
                    "{}/repos/{repo}/issues/{number}/comments?per_page=100&page={page}",
                    self.api_base
                ))
                .await?;
            let last = batch.len() < 100;
            comments.extend(batch);
            if last {
                break;
            }
        }
        Ok(comments)
    }

    pub async fn create_issue_comment(
        &self,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<IssueComment, String> {
        let url = format!("{}/repos/{repo}/issues/{number}/comments", self.api_base);
        let text = self
            .send(
                self.http
                    .post(&url)
                    .json(&serde_json::json!({ "body": body })),
                "application/vnd.github+json",
            )
            .await?;
        parse_json(&text)
    }

    pub async fn update_issue_comment(
        &self,
        repo: &str,
        comment_id: u64,
        body: &str,
    ) -> Result<IssueComment, String> {
        let url = format!(
            "{}/repos/{repo}/issues/comments/{comment_id}",
            self.api_base
        );
        let text = self
            .send(
                self.http
                    .patch(&url)
                    .json(&serde_json::json!({ "body": body })),
                "application/vnd.github+json",
            )
            .await?;
        parse_json(&text)
    }

    /// Edit the first comment whose body starts with `marker`, or post a new
    /// one. Returns the comment and whether it was created.
    pub async fn upsert_marked_comment(
        &self,
        repo: &str,
        number: u64,
        marker: &str,
        body: &str,
        max_pages: u32,
    ) -> Result<(IssueComment, bool), String> {
        let existing = self
            .issue_comments(repo, number, max_pages)
            .await?
            .into_iter()
            .find(|comment| {
                comment
                    .body
                    .as_deref()
                    .is_some_and(|text| text.starts_with(marker))
            });
        match existing {
            Some(comment) => Ok((
                self.update_issue_comment(repo, comment.id, body).await?,
                false,
            )),
            None => Ok((self.create_issue_comment(repo, number, body).await?, true)),
        }
    }

    /// Every changed file with its patch, up to `max_pages` pages of 100.
    pub async fn pull_request_files_with_patches(
        &self,
        repo: &str,
        number: u64,
        max_pages: u32,
    ) -> Result<Vec<PullRequestFile>, String> {
        self.get_pages(
            &format!("{}/repos/{repo}/pulls/{number}/files", self.api_base),
            max_pages,
        )
        .await
    }

    /// Inline review comments on a pull request, oldest first.
    pub async fn review_comments(
        &self,
        repo: &str,
        number: u64,
        max_pages: u32,
    ) -> Result<Vec<ReviewComment>, String> {
        self.get_pages(
            &format!("{}/repos/{repo}/pulls/{number}/comments", self.api_base),
            max_pages,
        )
        .await
    }

    /// Submitted reviews on a pull request, oldest first.
    pub async fn reviews(
        &self,
        repo: &str,
        number: u64,
        max_pages: u32,
    ) -> Result<Vec<Review>, String> {
        self.get_pages(
            &format!("{}/repos/{repo}/pulls/{number}/reviews", self.api_base),
            max_pages,
        )
        .await
    }

    /// Submit a review in one call (no pending state is left behind).
    pub async fn create_review(
        &self,
        repo: &str,
        number: u64,
        review: &serde_json::Value,
    ) -> Result<Review, String> {
        let url = format!("{}/repos/{repo}/pulls/{number}/reviews", self.api_base);
        let text = self
            .send(
                self.http.post(&url).json(review),
                "application/vnd.github+json",
            )
            .await?;
        parse_json(&text)
    }

    pub async fn repository(&self, repo: &str) -> Result<Repository, String> {
        self.get_json(&format!("{}/repos/{repo}", self.api_base))
            .await
    }

    /// Issues of every state, newest first, up to `max_pages` pages of 100.
    /// The issues API also returns pull requests; callers filter them out.
    pub async fn issues(&self, repo: &str, max_pages: u32) -> Result<Vec<Issue>, String> {
        self.get_pages(
            &format!(
                "{}/repos/{repo}/issues?state=all&sort=created&direction=desc",
                self.api_base
            ),
            max_pages,
        )
        .await
    }

    pub async fn create_issue(
        &self,
        repo: &str,
        issue: &serde_json::Value,
    ) -> Result<Issue, String> {
        let url = format!("{}/repos/{repo}/issues", self.api_base);
        let text = self
            .send(
                self.http.post(&url).json(issue),
                "application/vnd.github+json",
            )
            .await?;
        parse_json(&text)
    }

    pub async fn update_issue(
        &self,
        repo: &str,
        number: u64,
        issue: &serde_json::Value,
    ) -> Result<Issue, String> {
        let url = format!("{}/repos/{repo}/issues/{number}", self.api_base);
        let text = self
            .send(
                self.http.patch(&url).json(issue),
                "application/vnd.github+json",
            )
            .await?;
        parse_json(&text)
    }

    /// Open pull requests whose head is `owner:branch`.
    pub async fn open_pull_requests_from(
        &self,
        repo: &str,
        head: &str,
    ) -> Result<Vec<PullRequestSummary>, String> {
        self.get_json(&format!(
            "{}/repos/{repo}/pulls?state=open&head={}",
            self.api_base,
            encode_query(head)
        ))
        .await
    }

    pub async fn create_pull_request(
        &self,
        repo: &str,
        pull: &serde_json::Value,
    ) -> Result<PullRequestSummary, String> {
        let url = format!("{}/repos/{repo}/pulls", self.api_base);
        let text = self
            .send(
                self.http.post(&url).json(pull),
                "application/vnd.github+json",
            )
            .await?;
        parse_json(&text)
    }

    /// GET a list endpoint page by page (100 per page) until a short page or
    /// `max_pages`.
    async fn get_pages<T>(&self, url: &str, max_pages: u32) -> Result<Vec<T>, String>
    where
        T: for<'de> Deserialize<'de>,
    {
        let separator = if url.contains('?') { '&' } else { '?' };
        let mut items = Vec::new();
        for page in 1..=max_pages.max(1) {
            let batch: Vec<T> = self
                .get_json(&format!("{url}{separator}per_page=100&page={page}"))
                .await?;
            let last = batch.len() < 100;
            items.extend(batch);
            if last {
                break;
            }
        }
        Ok(items)
    }

    async fn get_json<T>(&self, url: &str) -> Result<T, String>
    where
        T: for<'de> Deserialize<'de>,
    {
        let body = self
            .send(self.http.get(url), "application/vnd.github+json")
            .await?;
        parse_json(&body)
    }

    async fn send(&self, request: reqwest::RequestBuilder, accept: &str) -> Result<String, String> {
        let response = request
            .bearer_auth(&self.token)
            .header("Accept", accept)
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "everruns-github")
            .send()
            .await
            .map_err(|e| format!("Failed to connect to GitHub API: {e}"))?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| format!("Failed to read GitHub API response: {e}"))?;

        if !status.is_success() {
            return Err(format!("GitHub API error ({status}): {body}"));
        }
        Ok(body)
    }
}

fn parse_json<T>(body: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_str(body).map_err(|e| format!("Invalid JSON from GitHub API: {e}"))
}

pub(crate) fn cap_text(text: String, max_bytes: usize) -> CappedText {
    let total_bytes = text.len();
    if total_bytes <= max_bytes {
        return CappedText {
            text,
            total_bytes,
            truncated: false,
        };
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    CappedText {
        text: text[..end].to_string(),
        total_bytes,
        truncated: true,
    }
}

pub(crate) fn encode_query(input: &str) -> String {
    percent_encode(input, false)
}

fn encode_path(input: &str) -> String {
    input
        .split('/')
        .map(|segment| percent_encode(segment, false))
        .collect::<Vec<_>>()
        .join("/")
}

fn percent_encode(input: &str, keep_slash: bool) -> String {
    let mut result = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b'/' if keep_slash => result.push('/'),
            _ => {
                result.push('%');
                result.push_str(&format!("{byte:02X}"));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn encodes_query_qualifiers() {
        assert_eq!(
            encode_query("auth repo:fastify/fastify path:lib"),
            "auth%20repo%3Afastify%2Ffastify%20path%3Alib"
        );
    }

    #[test]
    fn decodes_github_file_content() {
        let file = GitHubFile {
            name: "mod.rs".into(),
            path: "src/mod.rs".into(),
            sha: "abc".into(),
            html_url: None,
            download_url: None,
            encoding: Some("base64".into()),
            content: Some("Zm4gbWFpbigpIHt9\n".into()),
        };
        assert_eq!(file.decoded_content().unwrap(), "fn main() {}");
    }

    #[test]
    fn caps_text_on_a_char_boundary() {
        let capped = cap_text("aé".repeat(3), 4);
        assert_eq!(capped.text, "aéa");
        assert_eq!(capped.total_bytes, 9);
        assert!(capped.truncated);
        assert!(!cap_text("short".into(), 10).truncated);
    }

    #[tokio::test]
    async fn pull_request_diff_asks_for_the_diff_media_type_and_caps_it() {
        use wiremock::matchers::header;
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/pulls/7"))
            .and(header("Accept", "application/vnd.github.diff"))
            .respond_with(ResponseTemplate::new(200).set_body_string("diff --git a/x b/x\n+1\n"))
            .mount(&mock_server)
            .await;

        let client = GitHubClient::with_base_url("token".into(), mock_server.uri());
        let diff = client.pull_request_diff("acme/app", 7, 10).await.unwrap();
        assert_eq!(diff.text, "diff --git");
        assert!(diff.truncated);
    }

    async fn comment_server(existing: serde_json::Value) -> MockServer {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/acme/app/issues/7/comments"))
            .respond_with(ResponseTemplate::new(200).set_body_json(existing))
            .mount(&mock_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/repos/acme/app/issues/7/comments"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": 99, "html_url": "https://github.com/acme/app/pull/7#issuecomment-99"
            })))
            .mount(&mock_server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/repos/acme/app/issues/comments/5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 5, "html_url": "https://github.com/acme/app/pull/7#issuecomment-5"
            })))
            .mount(&mock_server)
            .await;
        mock_server
    }

    #[tokio::test]
    async fn upsert_edits_the_marked_comment() {
        let server = comment_server(serde_json::json!([
            {"id": 4, "html_url": "u4", "body": "LGTM <!-- everruns:summary -->"},
            {"id": 5, "html_url": "u5", "body": "<!-- everruns:summary -->\nold"}
        ]))
        .await;
        let client = GitHubClient::with_base_url("token".into(), server.uri());
        let (comment, created) = client
            .upsert_marked_comment("acme/app", 7, "<!-- everruns:summary -->", "new", 1)
            .await
            .unwrap();
        assert_eq!((comment.id, created), (5, false));
    }

    #[tokio::test]
    async fn upsert_posts_when_no_marked_comment_exists() {
        // A comment that only quotes the marker mid-body is not ours.
        let server = comment_server(serde_json::json!([
            {"id": 4, "html_url": "u4", "body": "see <!-- everruns:summary -->"}
        ]))
        .await;
        let client = GitHubClient::with_base_url("token".into(), server.uri());
        let (comment, created) = client
            .upsert_marked_comment("acme/app", 7, "<!-- everruns:summary -->", "new", 1)
            .await
            .unwrap();
        assert_eq!((comment.id, created), (99, true));
    }

    #[tokio::test]
    async fn search_code_success() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search/code"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "total_count": 1,
                "incomplete_results": false,
                "items": [{
                    "name": "auth.rs",
                    "path": "src/auth.rs",
                    "sha": "abc",
                    "html_url": "https://github.com/acme/app/blob/main/src/auth.rs",
                    "repository": {
                        "full_name": "acme/app",
                        "html_url": "https://github.com/acme/app"
                    }
                }]
            })))
            .mount(&mock_server)
            .await;

        let client = GitHubClient::with_base_url("token".into(), mock_server.uri());
        let result = client.search_code("auth repo:acme/app", 10).await.unwrap();
        assert_eq!(result.total_count, 1);
        assert_eq!(result.items[0].path, "src/auth.rs");
    }
}
