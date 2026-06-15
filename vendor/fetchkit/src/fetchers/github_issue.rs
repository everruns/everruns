//! GitHub issue and pull request fetcher
//!
//! Handles GitHub issue/PR URLs, returning structured metadata and comments
//! optimized for LLM consumption via the GitHub API.

use crate::client::FetchOptions;
use crate::error::FetchError;
use crate::fetchers::default::{read_full_body, transport_request};
use crate::fetchers::Fetcher;
use crate::types::{FetchRequest, FetchResponse};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

/// First-byte timeout for API requests
const API_TIMEOUT: Duration = Duration::from_secs(10);

/// GitHub API host and port (DNS-pinned per request).
const GITHUB_API_HOST: &str = "api.github.com";
const GITHUB_API_PORT: u16 = 443;

/// Max comments to fetch (first page)
const MAX_COMMENTS: usize = 100;
const DEFAULT_MAX_BODY_SIZE: usize = 5 * 1024 * 1024; // 5MB

/// GitHub issue/PR fetcher
///
/// Matches `https://github.com/{owner}/{repo}/issues/{number}` and
/// `https://github.com/{owner}/{repo}/pull/{number}`, returning
/// structured metadata and comment threads.
pub struct GitHubIssueFetcher;

impl GitHubIssueFetcher {
    pub fn new() -> Self {
        Self
    }

    /// Extract owner, repo, and issue number from a GitHub issue/PR URL
    fn parse_url(url: &Url) -> Option<(String, String, u64)> {
        if url.host_str() != Some("github.com") {
            return None;
        }

        let segments: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();

        // Must be /{owner}/{repo}/issues/{number} or /{owner}/{repo}/pull/{number}
        if segments.len() != 4 {
            return None;
        }

        let owner = segments[0];
        let repo = segments[1];
        let kind = segments[2];
        let number_str = segments[3];

        if owner.is_empty() || repo.is_empty() {
            return None;
        }

        // Only match issues and pull
        if kind != "issues" && kind != "pull" {
            return None;
        }

        let number: u64 = number_str.parse().ok()?;

        // Exclude reserved owner paths (same as GitHubRepoFetcher)
        let reserved = [
            "settings",
            "explore",
            "trending",
            "collections",
            "events",
            "sponsors",
            "notifications",
            "marketplace",
            "pulls",
            "issues",
            "codespaces",
            "features",
            "enterprise",
            "organizations",
            "pricing",
            "about",
            "team",
            "security",
            "login",
            "join",
        ];
        if reserved.contains(&owner) {
            return None;
        }

        Some((owner.to_string(), repo.to_string(), number))
    }
}

impl Default for GitHubIssueFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// --- API response types ---

#[derive(Debug, Deserialize)]
struct GitHubIssueData {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    html_url: String,
    user: GitHubUser,
    labels: Vec<GitHubLabel>,
    assignees: Vec<GitHubUser>,
    milestone: Option<GitHubMilestone>,
    created_at: String,
    updated_at: String,
    closed_at: Option<String>,
    comments: u64,
    // PR-specific fields (present when fetched via issues API for PRs)
    pull_request: Option<GitHubPullRequestRef>,
}

#[derive(Debug, Deserialize)]
struct GitHubUser {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GitHubLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubMilestone {
    title: String,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequestRef {
    merged_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequestData {
    additions: u64,
    deletions: u64,
    changed_files: u64,
    merged: bool,
    mergeable_state: Option<String>,
    review_comments: u64,
}

#[derive(Debug, Deserialize)]
struct GitHubComment {
    user: GitHubUser,
    body: Option<String>,
    created_at: String,
}

#[async_trait]
impl Fetcher for GitHubIssueFetcher {
    fn name(&self) -> &'static str {
        "github_issue"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::parse_url(url).is_some()
    }

    async fn fetch(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
    ) -> Result<FetchResponse, FetchError> {
        let url = Url::parse(&request.url).map_err(|_| FetchError::InvalidUrlScheme)?;

        let (owner, repo, number) = Self::parse_url(&url).ok_or_else(|| {
            FetchError::FetcherError("Not a valid GitHub issue/PR URL".to_string())
        })?;

        // Same pattern as GitHubRepoFetcher: single-hop API requests, DNS pinned per call.
        let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
        let ua_header = HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT));
        let accept_header = HeaderValue::from_static("application/vnd.github+json");

        let api_headers = || {
            let mut headers = HeaderMap::new();
            headers.insert(USER_AGENT, ua_header.clone());
            headers.insert(ACCEPT, accept_header.clone());
            headers
        };
        let github_api_get = |url: String| {
            let headers = api_headers();
            async move {
                let parsed = Url::parse(&url).map_err(|_| FetchError::InvalidUrlScheme)?;
                transport_request(
                    parsed,
                    reqwest::Method::GET,
                    headers,
                    options,
                    API_TIMEOUT,
                    GITHUB_API_HOST,
                    GITHUB_API_PORT,
                )
                .await
            }
        };

        // 1. Fetch issue/PR metadata (issues API works for both)
        let issue_url = format!(
            "https://api.github.com/repos/{}/{}/issues/{}",
            owner, repo, number
        );
        let issue_response = github_api_get(issue_url).await?;

        let status_code = issue_response.status;
        if !(200..300).contains(&status_code) {
            let error_msg = if status_code == 404 {
                format!("{}/{}#{} not found", owner, repo, number)
            } else if status_code == 403 {
                "GitHub API rate limit exceeded".to_string()
            } else {
                format!("GitHub API error: HTTP {}", status_code)
            };
            return Ok(FetchResponse {
                url: request.url.clone(),
                status_code,
                error: Some(error_msg),
                ..Default::default()
            });
        }

        let issue_body = read_full_body(issue_response, options).await?;
        let issue: GitHubIssueData = serde_json::from_slice(&issue_body)
            .map_err(|e| FetchError::FetcherError(format!("Failed to parse issue data: {}", e)))?;

        let is_pr = issue.pull_request.is_some();

        // 2. For PRs, fetch additional PR-specific data (diff stats, merge status)
        let pr_data = if is_pr {
            let pr_url = format!(
                "https://api.github.com/repos/{}/{}/pulls/{}",
                owner, repo, number
            );
            match github_api_get(pr_url).await {
                Ok(resp) if (200..300).contains(&resp.status) => {
                    match read_full_body(resp, options).await {
                        Ok(body) => serde_json::from_slice(&body).ok(),
                        Err(_) => None,
                    }
                }
                _ => None,
            }
        } else {
            None
        };

        // 3. Fetch comments
        let comments = if issue.comments > 0 {
            let comments_url = format!(
                "https://api.github.com/repos/{}/{}/issues/{}/comments?per_page={}",
                owner, repo, number, MAX_COMMENTS
            );
            match github_api_get(comments_url).await {
                Ok(resp) if (200..300).contains(&resp.status) => {
                    match read_full_body(resp, options).await {
                        Ok(body) => serde_json::from_slice::<Vec<GitHubComment>>(&body).ok(),
                        Err(_) => None,
                    }
                }
                _ => None,
            }
        } else {
            None
        };

        let format = if is_pr {
            "github_pull_request"
        } else {
            "github_issue"
        };

        let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);
        let (content, truncated) = truncate_to_max_bytes(
            format_issue_response(&issue, pr_data.as_ref(), comments.as_deref()),
            max_body_size,
        );
        let size = u64::try_from(content.len()).unwrap_or(u64::MAX);

        Ok(FetchResponse {
            url: request.url.clone(),
            status_code: 200,
            content_type: Some("text/markdown".to_string()),
            format: Some(format.to_string()),
            content: Some(content),
            size: Some(size),
            truncated: Some(truncated),
            ..Default::default()
        })
    }
}

fn truncate_to_max_bytes(mut s: String, max_bytes: usize) -> (String, bool) {
    if s.len() <= max_bytes {
        return (s, false);
    }

    if max_bytes == 0 {
        return (String::new(), true);
    }

    let mut end = max_bytes.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    (s, true)
}

fn format_issue_response(
    issue: &GitHubIssueData,
    pr_data: Option<&GitHubPullRequestData>,
    comments: Option<&[GitHubComment]>,
) -> String {
    let mut out = String::new();

    let kind = if pr_data.is_some() { "PR" } else { "Issue" };

    // Title
    out.push_str(&format!(
        "# {} #{}: {}\n\n",
        kind, issue.number, issue.title
    ));

    // Metadata
    out.push_str("## Metadata\n\n");
    out.push_str(&format!("- **State:** {}\n", issue.state));
    out.push_str(&format!("- **Author:** {}\n", issue.user.login));
    out.push_str(&format!("- **Created:** {}\n", issue.created_at));
    out.push_str(&format!("- **Updated:** {}\n", issue.updated_at));

    if let Some(closed) = &issue.closed_at {
        out.push_str(&format!("- **Closed:** {}\n", closed));
    }

    if !issue.labels.is_empty() {
        let labels: Vec<&str> = issue.labels.iter().map(|l| l.name.as_str()).collect();
        out.push_str(&format!("- **Labels:** {}\n", labels.join(", ")));
    }

    if !issue.assignees.is_empty() {
        let assignees: Vec<&str> = issue.assignees.iter().map(|a| a.login.as_str()).collect();
        out.push_str(&format!("- **Assignees:** {}\n", assignees.join(", ")));
    }

    if let Some(milestone) = &issue.milestone {
        out.push_str(&format!("- **Milestone:** {}\n", milestone.title));
    }

    out.push_str(&format!("- **URL:** {}\n", issue.html_url));

    // PR-specific metadata
    if let Some(pr) = pr_data {
        out.push_str(&format!(
            "- **Changes:** +{} -{} across {} files\n",
            pr.additions, pr.deletions, pr.changed_files
        ));

        if pr.merged {
            if let Some(merged_at) = &issue
                .pull_request
                .as_ref()
                .and_then(|p| p.merged_at.clone())
            {
                out.push_str(&format!("- **Merged:** {}\n", merged_at));
            } else {
                out.push_str("- **Merged:** yes\n");
            }
        }

        if let Some(state) = &pr.mergeable_state {
            out.push_str(&format!("- **Mergeable:** {}\n", state));
        }

        if pr.review_comments > 0 {
            out.push_str(&format!("- **Review comments:** {}\n", pr.review_comments));
        }
    }

    // Body
    if let Some(body) = &issue.body {
        if !body.is_empty() {
            out.push_str(&format!("\n## Description\n\n{}\n", body));
        }
    }

    // Comments
    if let Some(comments) = comments {
        if !comments.is_empty() {
            let total = issue.comments as usize;
            let shown = comments.len();
            if shown < total {
                out.push_str(&format!("\n## Comments ({} of {})\n\n", shown, total));
            } else {
                out.push_str(&format!("\n## Comments ({})\n\n", shown));
            }

            for comment in comments {
                out.push_str(&format!(
                    "### {} — {}\n\n",
                    comment.user.login, comment.created_at
                ));
                if let Some(body) = &comment.body {
                    out.push_str(body);
                    out.push_str("\n\n");
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_issue_url() {
        let url = Url::parse("https://github.com/owner/repo/issues/42").unwrap();
        assert_eq!(
            GitHubIssueFetcher::parse_url(&url),
            Some(("owner".to_string(), "repo".to_string(), 42))
        );
    }

    #[test]
    fn test_parse_pull_url() {
        let url = Url::parse("https://github.com/owner/repo/pull/123").unwrap();
        assert_eq!(
            GitHubIssueFetcher::parse_url(&url),
            Some(("owner".to_string(), "repo".to_string(), 123))
        );
    }

    #[test]
    fn test_rejects_non_github() {
        let url = Url::parse("https://gitlab.com/owner/repo/issues/1").unwrap();
        assert_eq!(GitHubIssueFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_non_issue_paths() {
        let url = Url::parse("https://github.com/owner/repo/blob/main/file.rs").unwrap();
        assert_eq!(GitHubIssueFetcher::parse_url(&url), None);

        let url = Url::parse("https://github.com/owner/repo/actions/123").unwrap();
        assert_eq!(GitHubIssueFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_non_numeric_number() {
        let url = Url::parse("https://github.com/owner/repo/issues/abc").unwrap();
        assert_eq!(GitHubIssueFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_reserved_owner() {
        let url = Url::parse("https://github.com/settings/repo/issues/1").unwrap();
        assert_eq!(GitHubIssueFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_wrong_segment_count() {
        let url = Url::parse("https://github.com/owner/repo/issues").unwrap();
        assert_eq!(GitHubIssueFetcher::parse_url(&url), None);

        let url = Url::parse("https://github.com/owner/repo/issues/42/comments").unwrap();
        assert_eq!(GitHubIssueFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = GitHubIssueFetcher::new();

        let url = Url::parse("https://github.com/rust-lang/rust/issues/1").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://github.com/rust-lang/rust/pull/99").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://github.com/rust-lang/rust").unwrap();
        assert!(!fetcher.matches(&url));
    }

    #[test]
    fn test_format_issue_response() {
        let issue = GitHubIssueData {
            number: 42,
            title: "Bug in parser".to_string(),
            body: Some("The parser fails on empty input.".to_string()),
            state: "open".to_string(),
            html_url: "https://github.com/owner/repo/issues/42".to_string(),
            user: GitHubUser {
                login: "alice".to_string(),
            },
            labels: vec![GitHubLabel {
                name: "bug".to_string(),
            }],
            assignees: vec![GitHubUser {
                login: "bob".to_string(),
            }],
            milestone: Some(GitHubMilestone {
                title: "v1.0".to_string(),
            }),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            closed_at: None,
            comments: 1,
            pull_request: None,
        };

        let comments = vec![GitHubComment {
            user: GitHubUser {
                login: "charlie".to_string(),
            },
            body: Some("I can reproduce this.".to_string()),
            created_at: "2024-01-01T12:00:00Z".to_string(),
        }];

        let output = format_issue_response(&issue, None, Some(&comments));

        assert!(output.contains("# Issue #42: Bug in parser"));
        assert!(output.contains("**State:** open"));
        assert!(output.contains("**Author:** alice"));
        assert!(output.contains("**Labels:** bug"));
        assert!(output.contains("**Assignees:** bob"));
        assert!(output.contains("**Milestone:** v1.0"));
        assert!(output.contains("The parser fails on empty input."));
        assert!(output.contains("charlie"));
        assert!(output.contains("I can reproduce this."));
    }

    #[test]
    fn test_format_pr_response() {
        let issue = GitHubIssueData {
            number: 10,
            title: "Add feature X".to_string(),
            body: Some("Implements feature X.".to_string()),
            state: "closed".to_string(),
            html_url: "https://github.com/owner/repo/pull/10".to_string(),
            user: GitHubUser {
                login: "dev".to_string(),
            },
            labels: vec![],
            assignees: vec![],
            milestone: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
            closed_at: Some("2024-01-02T00:00:00Z".to_string()),
            comments: 0,
            pull_request: Some(GitHubPullRequestRef {
                merged_at: Some("2024-01-02T00:00:00Z".to_string()),
            }),
        };

        let pr_data = GitHubPullRequestData {
            additions: 50,
            deletions: 10,
            changed_files: 3,
            merged: true,
            mergeable_state: None,
            review_comments: 2,
        };

        let output = format_issue_response(&issue, Some(&pr_data), None);

        assert!(output.contains("# PR #10: Add feature X"));
        assert!(output.contains("+50 -10 across 3 files"));
        assert!(output.contains("**Merged:**"));
        assert!(output.contains("**Review comments:** 2"));
    }

    #[test]
    fn test_truncate_to_max_bytes() {
        let (s, truncated) = truncate_to_max_bytes("hello world".to_string(), 5);
        assert_eq!(s, "hello");
        assert!(truncated);

        let (s, truncated) = truncate_to_max_bytes("éclair".to_string(), 1);
        assert_eq!(s, "");
        assert!(truncated);
    }
}
