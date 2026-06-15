//! GitHub repository fetcher
//!
//! Handles GitHub repository root URLs, returning repo metadata and README content.

use crate::client::FetchOptions;
use crate::error::FetchError;
use crate::fetchers::default::{read_full_body, transport_request};
use crate::fetchers::Fetcher;
use crate::types::{FetchRequest, FetchResponse, HttpMethod};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

/// GitHub API host and port (DNS-pinned per request).
const GITHUB_API_HOST: &str = "api.github.com";
const GITHUB_API_PORT: u16 = 443;

/// First-byte timeout for API requests
const API_TIMEOUT: Duration = Duration::from_secs(10);

/// GitHub repository fetcher
///
/// Matches GitHub repository root URLs (`https://github.com/{owner}/{repo}`)
/// and returns repository metadata along with README content.
pub struct GitHubRepoFetcher;

impl GitHubRepoFetcher {
    /// Create a new GitHub repo fetcher
    pub fn new() -> Self {
        Self
    }

    /// Extract owner and repo from a GitHub URL
    fn parse_github_url(url: &Url) -> Option<(String, String)> {
        // Must be github.com
        if url.host_str() != Some("github.com") {
            return None;
        }

        // Get path segments
        let segments: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();

        // Must have exactly 2 segments (owner/repo)
        // Ignore URLs like /owner/repo/issues, /owner/repo/blob/main/file.rs
        if segments.len() != 2 {
            return None;
        }

        let owner = segments[0];
        let repo = segments[1];

        // Basic validation
        if owner.is_empty() || repo.is_empty() {
            return None;
        }

        // Ignore special GitHub paths
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

        Some((owner.to_string(), repo.to_string()))
    }

    /// Enforce URL prefix policy for secondary outbound requests.
    fn validate_policy_url(url: &str, options: &FetchOptions) -> Result<(), FetchError> {
        if !options.allow_prefixes.is_empty()
            && !options
                .allow_prefixes
                .iter()
                .any(|prefix| url.starts_with(prefix))
        {
            return Err(FetchError::BlockedUrl);
        }

        if options
            .block_prefixes
            .iter()
            .any(|prefix| url.starts_with(prefix))
        {
            return Err(FetchError::BlockedUrl);
        }

        Ok(())
    }
}

impl Default for GitHubRepoFetcher {
    fn default() -> Self {
        Self::new()
    }
}

/// GitHub API repository response (partial)
#[derive(Debug, Deserialize)]
struct GitHubRepo {
    #[allow(dead_code)]
    name: String,
    full_name: String,
    description: Option<String>,
    html_url: String,
    homepage: Option<String>,
    stargazers_count: u64,
    forks_count: u64,
    open_issues_count: u64,
    language: Option<String>,
    license: Option<GitHubLicense>,
    default_branch: String,
    created_at: String,
    updated_at: String,
    pushed_at: String,
    topics: Option<Vec<String>>,
    archived: bool,
    fork: bool,
    owner: GitHubOwner,
}

#[derive(Debug, Deserialize)]
struct GitHubLicense {
    name: String,
    spdx_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GitHubOwner {
    login: String,
    #[serde(rename = "type")]
    owner_type: String,
}

/// GitHub API README response
#[derive(Debug, Deserialize)]
struct GitHubReadme {
    content: String,
    encoding: String,
}

#[async_trait]
impl Fetcher for GitHubRepoFetcher {
    fn name(&self) -> &'static str {
        "github_repo"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::parse_github_url(url).is_some()
    }

    async fn fetch(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
    ) -> Result<FetchResponse, FetchError> {
        let url = Url::parse(&request.url).map_err(|_| FetchError::InvalidUrlScheme)?;

        let (owner, repo) = Self::parse_github_url(&url).ok_or_else(|| {
            FetchError::FetcherError("Not a valid GitHub repository URL".to_string())
        })?;

        // THREAT[TM-SSRF-001]: Validate DNS resolution for GitHub API host (pinned per request).
        let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
        let ua_header = HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT));
        let accept_header = HeaderValue::from_static("application/vnd.github+json");

        // Headers shared by every GitHub API subrequest.
        let api_headers = || {
            let mut headers = HeaderMap::new();
            headers.insert(USER_AGENT, ua_header.clone());
            headers.insert(ACCEPT, accept_header.clone());
            headers
        };

        // Fetch repository metadata
        // THREAT[TM-INPUT-002]/[TM-INPUT-007]: enforce allow/block prefix policy on the
        // GitHub API subrequest URL (PR #131), not just the user-facing github.com URL.
        let repo_url = format!("https://api.github.com/repos/{}/{}", owner, repo);
        Self::validate_policy_url(&repo_url, options)?;
        let parsed_repo_url = Url::parse(&repo_url).map_err(|_| FetchError::InvalidUrlScheme)?;

        let method = match request.effective_method() {
            HttpMethod::Get => reqwest::Method::GET,
            HttpMethod::Head => reqwest::Method::HEAD,
        };
        // THREAT[TM-SSRF-010]: single-hop transport request; redirects are not followed
        // for the GitHub API to avoid cross-host hops.
        let repo_response = transport_request(
            parsed_repo_url,
            method,
            api_headers(),
            options,
            API_TIMEOUT,
            GITHUB_API_HOST,
            GITHUB_API_PORT,
        )
        .await?;

        let status_code = repo_response.status;

        // Handle non-success status
        if !(200..300).contains(&status_code) {
            let error_msg = if status_code == 404 {
                format!("Repository {}/{} not found", owner, repo)
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

        if matches!(request.effective_method(), HttpMethod::Head) {
            return Ok(FetchResponse {
                url: request.url.clone(),
                status_code,
                content_type: Some("application/vnd.github+json".to_string()),
                ..Default::default()
            });
        }

        // Parse repository data
        let repo_body = read_full_body(repo_response, options).await?;
        let repo_data: GitHubRepo = serde_json::from_slice(&repo_body)
            .map_err(|e| FetchError::FetcherError(format!("Failed to parse repo data: {}", e)))?;

        // Fetch README (optional - don't fail if missing)
        let readme_url = format!("https://api.github.com/repos/{}/{}/readme", owner, repo);
        Self::validate_policy_url(&readme_url, options)?;
        let readme_content = match Url::parse(&readme_url) {
            Ok(parsed_readme_url) => {
                match transport_request(
                    parsed_readme_url,
                    reqwest::Method::GET,
                    api_headers(),
                    options,
                    API_TIMEOUT,
                    GITHUB_API_HOST,
                    GITHUB_API_PORT,
                )
                .await
                {
                    Ok(resp) if (200..300).contains(&resp.status) => {
                        match read_full_body(resp, options).await {
                            Ok(body) => match serde_json::from_slice::<GitHubReadme>(&body) {
                                Ok(readme) if readme.encoding == "base64" => {
                                    decode_base64_content(&readme.content)
                                }
                                _ => None,
                            },
                            Err(_) => None,
                        }
                    }
                    _ => None,
                }
            }
            Err(_) => None,
        };

        // Format response as markdown
        let content = format_github_repo_response(&repo_data, readme_content.as_deref());

        Ok(FetchResponse {
            url: request.url.clone(),
            status_code: 200,
            content_type: Some("text/markdown".to_string()),
            format: Some("github_repo".to_string()),
            content: Some(content),
            ..Default::default()
        })
    }
}

/// Decode base64-encoded content (GitHub API returns README as base64)
fn decode_base64_content(encoded: &str) -> Option<String> {
    // GitHub base64 has newlines, remove them
    let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();

    // Simple base64 decode
    let decoded = base64_decode(&cleaned)?;
    String::from_utf8(decoded).ok()
}

/// Basic base64 decoder (avoiding extra dependency)
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn decode_char(c: u8) -> Option<u8> {
        if c == b'=' {
            return Some(0);
        }
        ALPHABET.iter().position(|&x| x == c).map(|p| p as u8)
    }

    let bytes: Vec<u8> = input.bytes().collect();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }

    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);

    for chunk in bytes.chunks(4) {
        let a = decode_char(chunk[0])?;
        let b = decode_char(chunk[1])?;
        let c = decode_char(chunk[2])?;
        let d = decode_char(chunk[3])?;

        result.push((a << 2) | (b >> 4));
        if chunk[2] != b'=' {
            result.push((b << 4) | (c >> 2));
        }
        if chunk[3] != b'=' {
            result.push((c << 6) | d);
        }
    }

    Some(result)
}

/// Format GitHub repo data as markdown
fn format_github_repo_response(repo: &GitHubRepo, readme: Option<&str>) -> String {
    let mut output = String::new();

    // Header
    output.push_str(&format!("# {}\n\n", repo.full_name));

    // Description
    if let Some(ref desc) = repo.description {
        output.push_str(&format!("{}\n\n", desc));
    }

    // Metadata section
    output.push_str("## Repository Info\n\n");

    // Stats
    output.push_str(&format!(
        "- **Stars:** {}\n- **Forks:** {}\n- **Open Issues:** {}\n",
        repo.stargazers_count, repo.forks_count, repo.open_issues_count
    ));

    // Language
    if let Some(ref lang) = repo.language {
        output.push_str(&format!("- **Language:** {}\n", lang));
    }

    // License
    if let Some(ref license) = repo.license {
        let license_str = license
            .spdx_id
            .as_ref()
            .unwrap_or(&license.name)
            .to_string();
        output.push_str(&format!("- **License:** {}\n", license_str));
    }

    // Topics
    if let Some(ref topics) = repo.topics {
        if !topics.is_empty() {
            output.push_str(&format!("- **Topics:** {}\n", topics.join(", ")));
        }
    }

    // Links
    output.push_str(&format!("- **URL:** {}\n", repo.html_url));
    if let Some(ref homepage) = repo.homepage {
        if !homepage.is_empty() {
            output.push_str(&format!("- **Homepage:** {}\n", homepage));
        }
    }

    // Branch info
    output.push_str(&format!("- **Default Branch:** {}\n", repo.default_branch));

    // Owner info
    output.push_str(&format!(
        "- **Owner:** {} ({})\n",
        repo.owner.login, repo.owner.owner_type
    ));

    // Status flags
    if repo.archived {
        output.push_str("- **Status:** Archived\n");
    }
    if repo.fork {
        output.push_str("- **Fork:** Yes\n");
    }

    // Dates
    output.push_str(&format!("- **Created:** {}\n", repo.created_at));
    output.push_str(&format!("- **Last Updated:** {}\n", repo.updated_at));
    output.push_str(&format!("- **Last Push:** {}\n", repo.pushed_at));

    // README content
    if let Some(readme_content) = readme {
        output.push_str("\n---\n\n## README\n\n");
        output.push_str(readme_content);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_github_url_valid() {
        let url = Url::parse("https://github.com/owner/repo").unwrap();
        assert_eq!(
            GitHubRepoFetcher::parse_github_url(&url),
            Some(("owner".to_string(), "repo".to_string()))
        );
    }

    #[test]
    fn test_parse_github_url_with_trailing_slash() {
        // URL parser normalizes away trailing slash for path
        let url = Url::parse("https://github.com/owner/repo/").unwrap();
        // This actually has 3 segments: ["owner", "repo", ""]
        assert_eq!(GitHubRepoFetcher::parse_github_url(&url), None);
    }

    #[test]
    fn test_parse_github_url_too_many_segments() {
        let url = Url::parse("https://github.com/owner/repo/issues").unwrap();
        assert_eq!(GitHubRepoFetcher::parse_github_url(&url), None);

        let url = Url::parse("https://github.com/owner/repo/blob/main/README.md").unwrap();
        assert_eq!(GitHubRepoFetcher::parse_github_url(&url), None);
    }

    #[test]
    fn test_parse_github_url_too_few_segments() {
        let url = Url::parse("https://github.com/owner").unwrap();
        assert_eq!(GitHubRepoFetcher::parse_github_url(&url), None);

        let url = Url::parse("https://github.com/").unwrap();
        assert_eq!(GitHubRepoFetcher::parse_github_url(&url), None);
    }

    #[test]
    fn test_parse_github_url_reserved_paths() {
        let url = Url::parse("https://github.com/settings/profile").unwrap();
        assert_eq!(GitHubRepoFetcher::parse_github_url(&url), None);

        let url = Url::parse("https://github.com/explore/topics").unwrap();
        assert_eq!(GitHubRepoFetcher::parse_github_url(&url), None);
    }

    #[test]
    fn test_parse_github_url_wrong_host() {
        let url = Url::parse("https://gitlab.com/owner/repo").unwrap();
        assert_eq!(GitHubRepoFetcher::parse_github_url(&url), None);
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = GitHubRepoFetcher::new();

        let url = Url::parse("https://github.com/rust-lang/rust").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://github.com/rust-lang/rust/issues").unwrap();
        assert!(!fetcher.matches(&url));

        let url = Url::parse("https://example.com/foo/bar").unwrap();
        assert!(!fetcher.matches(&url));
    }

    #[test]
    fn test_validate_policy_url() {
        let options = FetchOptions {
            allow_prefixes: vec!["https://github.com".to_string()],
            ..Default::default()
        };
        assert!(GitHubRepoFetcher::validate_policy_url(
            "https://api.github.com/repos/o/r",
            &options
        )
        .is_err());

        let options = FetchOptions {
            allow_prefixes: vec!["https://api.github.com/repos/".to_string()],
            block_prefixes: vec!["https://api.github.com/repos/o/r".to_string()],
            ..Default::default()
        };
        assert!(GitHubRepoFetcher::validate_policy_url(
            "https://api.github.com/repos/o/r",
            &options
        )
        .is_err());
    }

    #[test]
    fn test_base64_decode() {
        // "Hello, World!" in base64
        assert_eq!(
            base64_decode("SGVsbG8sIFdvcmxkIQ=="),
            Some(b"Hello, World!".to_vec())
        );

        // Empty string
        assert_eq!(base64_decode(""), Some(vec![]));

        // Invalid length
        assert_eq!(base64_decode("abc"), None);
    }

    #[test]
    fn test_format_github_repo_response() {
        let repo = GitHubRepo {
            name: "test-repo".to_string(),
            full_name: "owner/test-repo".to_string(),
            description: Some("A test repository".to_string()),
            html_url: "https://github.com/owner/test-repo".to_string(),
            homepage: None,
            stargazers_count: 100,
            forks_count: 10,
            open_issues_count: 5,
            language: Some("Rust".to_string()),
            license: Some(GitHubLicense {
                name: "MIT License".to_string(),
                spdx_id: Some("MIT".to_string()),
            }),
            default_branch: "main".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-06-01T00:00:00Z".to_string(),
            pushed_at: "2024-06-01T00:00:00Z".to_string(),
            topics: Some(vec!["rust".to_string(), "cli".to_string()]),
            archived: false,
            fork: false,
            owner: GitHubOwner {
                login: "owner".to_string(),
                owner_type: "User".to_string(),
            },
        };

        let output = format_github_repo_response(&repo, Some("# Test\n\nThis is a test README."));

        assert!(output.contains("# owner/test-repo"));
        assert!(output.contains("A test repository"));
        assert!(output.contains("**Stars:** 100"));
        assert!(output.contains("**Language:** Rust"));
        assert!(output.contains("**License:** MIT"));
        assert!(output.contains("## README"));
        assert!(output.contains("This is a test README."));
    }
}
