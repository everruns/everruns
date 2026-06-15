//! GitHub source file fetcher
//!
//! Handles GitHub blob URLs, returning raw source file content with language
//! metadata via the GitHub API, optimized for LLM consumption.

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

const API_TIMEOUT: Duration = Duration::from_secs(10);

/// GitHub API host and port (DNS-pinned per request).
const GITHUB_API_HOST: &str = "api.github.com";
const GITHUB_API_PORT: u16 = 443;

/// Max file size we'll return inline (1 MB, matching GitHub contents API limit)
const MAX_INLINE_SIZE: u64 = 1_048_576;

/// GitHub source file fetcher
///
/// Matches `https://github.com/{owner}/{repo}/blob/{ref}/{path}` and returns
/// raw file content with language metadata.
pub struct GitHubCodeFetcher;

impl GitHubCodeFetcher {
    pub fn new() -> Self {
        Self
    }

    /// Extract owner, repo, ref, and path from a GitHub blob URL
    fn parse_url(url: &Url) -> Option<ParsedBlobUrl> {
        if url.host_str() != Some("github.com") {
            return None;
        }

        let segments: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();

        // Minimum: /{owner}/{repo}/blob/{ref}/{path} = 5+ segments
        if segments.len() < 5 {
            return None;
        }

        let owner = segments[0];
        let repo = segments[1];
        let kind = segments[2];
        let git_ref = segments[3];

        if owner.is_empty() || repo.is_empty() || git_ref.is_empty() {
            return None;
        }

        if kind != "blob" {
            return None;
        }

        // Exclude reserved owner paths
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

        // Path is everything after the ref
        let file_path = segments[4..].join("/");
        if file_path.is_empty() {
            return None;
        }

        Some(ParsedBlobUrl {
            owner: owner.to_string(),
            repo: repo.to_string(),
            git_ref: git_ref.to_string(),
            path: file_path,
        })
    }
}

impl Default for GitHubCodeFetcher {
    fn default() -> Self {
        Self::new()
    }
}

struct ParsedBlobUrl {
    owner: String,
    repo: String,
    git_ref: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct GitHubContents {
    name: String,
    path: String,
    size: u64,
    #[serde(rename = "type")]
    content_type: String,
    content: Option<String>,
    html_url: Option<String>,
}

#[async_trait]
impl Fetcher for GitHubCodeFetcher {
    fn name(&self) -> &'static str {
        "github_code"
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

        let parsed = Self::parse_url(&url)
            .ok_or_else(|| FetchError::FetcherError("Not a valid GitHub blob URL".to_string()))?;

        let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
        let ua_header = HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT));
        let accept_header = HeaderValue::from_static("application/vnd.github+json");

        // Fetch file contents via GitHub API
        let api_url = format!(
            "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
            parsed.owner, parsed.repo, parsed.path, parsed.git_ref
        );
        let parsed_api_url = Url::parse(&api_url).map_err(|_| FetchError::InvalidUrlScheme)?;
        // THREAT[TM-INPUT]: enforce host/port policy on the GitHub API subrequest (PR #131).
        options.validate_url(&parsed_api_url)?;

        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, ua_header);
        headers.insert(ACCEPT, accept_header);

        // THREAT[TM-SSRF-010]: single-hop request, redirects not followed.
        let response = transport_request(
            parsed_api_url,
            reqwest::Method::GET,
            headers,
            options,
            API_TIMEOUT,
            GITHUB_API_HOST,
            GITHUB_API_PORT,
        )
        .await?;

        let status_code = response.status;
        if !(200..300).contains(&status_code) {
            let error_msg = if status_code == 404 {
                format!(
                    "{}/{}:{} {} not found",
                    parsed.owner, parsed.repo, parsed.git_ref, parsed.path
                )
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

        let body = read_full_body(response, options).await?;
        let contents: GitHubContents = serde_json::from_slice(&body)
            .map_err(|e| FetchError::FetcherError(format!("Failed to parse contents: {}", e)))?;

        // Handle directories (content_type == "dir")
        if contents.content_type != "file" {
            return Ok(FetchResponse {
                url: request.url.clone(),
                status_code: 200,
                format: Some("github_file".to_string()),
                error: Some(format!("Path is a {} (not a file)", contents.content_type)),
                ..Default::default()
            });
        }

        // Handle binary/large files — return metadata only
        if contents.size > MAX_INLINE_SIZE || contents.content.is_none() {
            let content = format_metadata_only(&parsed, &contents);
            return Ok(FetchResponse {
                url: request.url.clone(),
                status_code: 200,
                content_type: Some("text/markdown".to_string()),
                format: Some("github_file".to_string()),
                content: Some(content),
                size: Some(contents.size),
                ..Default::default()
            });
        }

        // Decode base64 content
        let raw_content = contents.content.as_deref().and_then(decode_base64_content);

        let (file_content, is_binary) = match raw_content {
            Some(bytes) => match String::from_utf8(bytes) {
                Ok(text) => (Some(text), false),
                Err(_) => (None, true),
            },
            None => (None, true),
        };

        if is_binary {
            let content = format_metadata_only(&parsed, &contents);
            return Ok(FetchResponse {
                url: request.url.clone(),
                status_code: 200,
                content_type: Some("text/markdown".to_string()),
                format: Some("github_file".to_string()),
                content: Some(content),
                size: Some(contents.size),
                error: Some("Binary file — metadata only".to_string()),
                ..Default::default()
            });
        }

        let lang = detect_language(&contents.name);
        let content = format_file_response(&parsed, &contents, file_content.as_deref(), lang);

        Ok(FetchResponse {
            url: request.url.clone(),
            status_code: 200,
            content_type: Some("text/markdown".to_string()),
            format: Some("github_file".to_string()),
            content: Some(content),
            size: Some(contents.size),
            ..Default::default()
        })
    }
}

/// Decode base64 with whitespace (GitHub API includes newlines in base64)
fn decode_base64_content(encoded: &str) -> Option<Vec<u8>> {
    let cleaned: String = encoded.chars().filter(|c| !c.is_whitespace()).collect();
    base64_decode(&cleaned)
}

/// Basic base64 decoder (same approach as github_repo.rs)
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn decode_char(c: u8) -> Option<u8> {
        if c == b'=' {
            return Some(0);
        }
        ALPHABET.iter().position(|&x| x == c).map(|p| p as u8)
    }

    let bytes: Vec<u8> = input.bytes().collect();
    if !bytes.is_empty() && !bytes.len().is_multiple_of(4) {
        return None;
    }

    let mut result = Vec::with_capacity(bytes.len() * 3 / 4);

    for chunk in bytes.chunks(4) {
        if chunk.len() != 4 {
            return None;
        }
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

/// Simple language detection from file extension
fn detect_language(filename: &str) -> Option<&'static str> {
    let ext = filename.rsplit('.').next()?;
    match ext.to_ascii_lowercase().as_str() {
        "rs" => Some("rust"),
        "py" => Some("python"),
        "js" => Some("javascript"),
        "ts" => Some("typescript"),
        "tsx" => Some("tsx"),
        "jsx" => Some("jsx"),
        "rb" => Some("ruby"),
        "go" => Some("go"),
        "java" => Some("java"),
        "kt" | "kts" => Some("kotlin"),
        "swift" => Some("swift"),
        "c" => Some("c"),
        "cpp" | "cc" | "cxx" => Some("cpp"),
        "h" | "hpp" => Some("cpp"),
        "cs" => Some("csharp"),
        "php" => Some("php"),
        "sh" | "bash" => Some("bash"),
        "zsh" => Some("zsh"),
        "fish" => Some("fish"),
        "yml" | "yaml" => Some("yaml"),
        "json" => Some("json"),
        "toml" => Some("toml"),
        "xml" => Some("xml"),
        "html" | "htm" => Some("html"),
        "css" => Some("css"),
        "scss" | "sass" => Some("scss"),
        "sql" => Some("sql"),
        "md" | "markdown" => Some("markdown"),
        "dockerfile" => Some("dockerfile"),
        "tf" => Some("terraform"),
        "ex" | "exs" => Some("elixir"),
        "erl" => Some("erlang"),
        "hs" => Some("haskell"),
        "ml" | "mli" => Some("ocaml"),
        "r" => Some("r"),
        "scala" => Some("scala"),
        "lua" => Some("lua"),
        "zig" => Some("zig"),
        "nim" => Some("nim"),
        "v" => Some("v"),
        "dart" => Some("dart"),
        "proto" => Some("protobuf"),
        "graphql" | "gql" => Some("graphql"),
        _ => None,
    }
}

fn format_metadata_only(parsed: &ParsedBlobUrl, contents: &GitHubContents) -> String {
    let lang = detect_language(&contents.name);
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", contents.path));
    out.push_str("## File Info\n\n");
    out.push_str(&format!(
        "- **Repository:** {}/{}\n",
        parsed.owner, parsed.repo
    ));
    out.push_str(&format!("- **Ref:** {}\n", parsed.git_ref));
    out.push_str(&format!("- **Size:** {} bytes\n", contents.size));
    if let Some(lang) = lang {
        out.push_str(&format!("- **Language:** {}\n", lang));
    }
    if let Some(url) = &contents.html_url {
        out.push_str(&format!("- **URL:** {}\n", url));
    }
    out
}

fn format_file_response(
    parsed: &ParsedBlobUrl,
    contents: &GitHubContents,
    file_content: Option<&str>,
    lang: Option<&str>,
) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {}\n\n", contents.path));
    out.push_str("## File Info\n\n");
    out.push_str(&format!(
        "- **Repository:** {}/{}\n",
        parsed.owner, parsed.repo
    ));
    out.push_str(&format!("- **Ref:** {}\n", parsed.git_ref));
    out.push_str(&format!("- **Size:** {} bytes\n", contents.size));
    if let Some(lang) = lang {
        out.push_str(&format!("- **Language:** {}\n", lang));
    }
    if let Some(url) = &contents.html_url {
        out.push_str(&format!("- **URL:** {}\n", url));
    }

    if let Some(content) = file_content {
        let lang_hint = lang.unwrap_or("");
        out.push_str(&format!(
            "\n## Content\n\n```{}\n{}\n```\n",
            lang_hint, content
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_blob_url() {
        let url = Url::parse("https://github.com/owner/repo/blob/main/src/lib.rs").unwrap();
        let parsed = GitHubCodeFetcher::parse_url(&url).unwrap();
        assert_eq!(parsed.owner, "owner");
        assert_eq!(parsed.repo, "repo");
        assert_eq!(parsed.git_ref, "main");
        assert_eq!(parsed.path, "src/lib.rs");
    }

    #[test]
    fn test_parse_blob_url_nested_path() {
        let url = Url::parse("https://github.com/owner/repo/blob/v1.0.0/crates/core/src/main.rs")
            .unwrap();
        let parsed = GitHubCodeFetcher::parse_url(&url).unwrap();
        assert_eq!(parsed.git_ref, "v1.0.0");
        assert_eq!(parsed.path, "crates/core/src/main.rs");
    }

    #[test]
    fn test_rejects_non_blob() {
        let url = Url::parse("https://github.com/owner/repo/tree/main/src").unwrap();
        assert!(GitHubCodeFetcher::parse_url(&url).is_none());
    }

    #[test]
    fn test_rejects_too_few_segments() {
        let url = Url::parse("https://github.com/owner/repo/blob/main").unwrap();
        assert!(GitHubCodeFetcher::parse_url(&url).is_none());
    }

    #[test]
    fn test_rejects_non_github() {
        let url = Url::parse("https://gitlab.com/owner/repo/blob/main/file.rs").unwrap();
        assert!(GitHubCodeFetcher::parse_url(&url).is_none());
    }

    #[test]
    fn test_rejects_reserved_owner() {
        let url = Url::parse("https://github.com/settings/repo/blob/main/file.rs").unwrap();
        assert!(GitHubCodeFetcher::parse_url(&url).is_none());
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = GitHubCodeFetcher::new();

        let url = Url::parse("https://github.com/rust-lang/rust/blob/master/Cargo.toml").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://github.com/rust-lang/rust").unwrap();
        assert!(!fetcher.matches(&url));

        let url = Url::parse("https://github.com/rust-lang/rust/issues/1").unwrap();
        assert!(!fetcher.matches(&url));
    }

    #[test]
    fn test_detect_language() {
        assert_eq!(detect_language("main.rs"), Some("rust"));
        assert_eq!(detect_language("app.py"), Some("python"));
        assert_eq!(detect_language("index.tsx"), Some("tsx"));
        assert_eq!(detect_language("Cargo.toml"), Some("toml"));
        assert_eq!(detect_language("unknown.xyz"), None);
        assert_eq!(detect_language("Dockerfile"), Some("dockerfile"));
    }

    #[test]
    fn test_format_file_response() {
        let parsed = ParsedBlobUrl {
            owner: "owner".to_string(),
            repo: "repo".to_string(),
            git_ref: "main".to_string(),
            path: "src/lib.rs".to_string(),
        };
        let contents = GitHubContents {
            name: "lib.rs".to_string(),
            path: "src/lib.rs".to_string(),
            size: 42,
            content_type: "file".to_string(),
            content: None,
            html_url: Some("https://github.com/owner/repo/blob/main/src/lib.rs".to_string()),
        };

        let output = format_file_response(&parsed, &contents, Some("fn main() {}"), Some("rust"));

        assert!(output.contains("# src/lib.rs"));
        assert!(output.contains("**Repository:** owner/repo"));
        assert!(output.contains("**Language:** rust"));
        assert!(output.contains("```rust\nfn main() {}\n```"));
    }

    #[test]
    fn test_base64_decode() {
        // "Hello" in base64
        assert_eq!(base64_decode("SGVsbG8="), Some(b"Hello".to_vec()));
        assert_eq!(base64_decode(""), Some(vec![]));
        assert_eq!(base64_decode("abc"), None);
    }
}
