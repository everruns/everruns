//! Documentation site fetcher with llms.txt support
//!
//! Detects known documentation sites and the llms.txt standard,
//! returning clean content optimized for LLM consumption.
//!
//! Design: Matches known documentation site patterns (ReadTheDocs, docs.rs,
//! Docusaurus, etc.) and explicit llms.txt/llms-full.txt URLs. For matched
//! sites, probes for llms.txt before fetching the page. Falls through to
//! DefaultFetcher for non-docs URLs.

use crate::client::FetchOptions;
use crate::error::FetchError;
use crate::fetchers::default::{
    apply_bot_auth_if_enabled, header_value, read_body_with_timeout, read_full_body,
    send_request_following_redirects, BODY_TIMEOUT, DEFAULT_MAX_BODY_SIZE, TRUNCATION_MESSAGE,
};
use crate::fetchers::Fetcher;
use crate::types::{FetchRequest, FetchResponse};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use std::time::Duration;
use url::Url;

/// Timeout for API/probe requests
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Max size for llms.txt content (2 MB)
const MAX_LLMS_TXT_SIZE: usize = 2 * 1024 * 1024;

/// Known documentation site patterns (host suffixes or exact matches)
const DOCS_HOSTS: &[&str] = &[
    ".readthedocs.io",
    ".readthedocs.org",
    "docs.rs",
    ".gitbook.io",
    ".netlify.app", // Many docs sites use Netlify
    ".vercel.app",  // Many docs sites use Vercel
];

/// Known documentation site host prefixes
const DOCS_HOST_PREFIXES: &[&str] = &["docs.", "wiki.", "developer.", "devdocs."];

/// Documentation site fetcher with llms.txt support
///
/// Matches known documentation sites and explicit llms.txt URLs.
/// For matched sites, probes for llms-full.txt/llms.txt at the origin
/// before returning content.
pub struct DocsSiteFetcher;

impl DocsSiteFetcher {
    pub fn new() -> Self {
        Self
    }

    /// Check if a URL is a direct llms.txt request
    fn is_llms_txt_url(url: &Url) -> bool {
        let path = url.path();
        path == "/llms.txt" || path == "/llms-full.txt"
    }

    /// Check if a URL belongs to a known documentation site
    fn is_docs_site(url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return false;
        };
        let host = host.to_ascii_lowercase();

        // Check known host suffixes
        for suffix in DOCS_HOSTS {
            if host.ends_with(suffix) {
                return true;
            }
        }

        // Check known host prefixes
        for prefix in DOCS_HOST_PREFIXES {
            if host.starts_with(prefix) {
                return true;
            }
        }

        false
    }
}

impl Default for DocsSiteFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Fetcher for DocsSiteFetcher {
    fn name(&self) -> &'static str {
        "docs_site"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::is_llms_txt_url(url) || Self::is_docs_site(url)
    }

    async fn fetch(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
    ) -> Result<FetchResponse, FetchError> {
        let url = Url::parse(&request.url).map_err(|_| FetchError::InvalidUrlScheme)?;
        let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
        let ua_header = HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT));

        // If this IS a direct llms.txt URL, fetch it directly
        if Self::is_llms_txt_url(&url) {
            return fetch_llms_txt_direct(url, ua_header, options).await;
        }

        // For docs sites, probe for llms.txt at origin
        let origin = format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap_or_default(),
            url.port().map(|p| format!(":{}", p)).unwrap_or_default()
        );

        // Try llms-full.txt first, then llms.txt
        let probe_urls = [
            (format!("{}/llms-full.txt", origin), "llms-full.txt"),
            (format!("{}/llms.txt", origin), "llms.txt"),
        ];

        for (probe_url, source) in &probe_urls {
            let probe_url = Url::parse(probe_url).map_err(|_| FetchError::InvalidUrlScheme)?;
            if let Some(content) = try_fetch_llms_txt(probe_url, ua_header.clone(), options).await {
                return Ok(FetchResponse {
                    url: request.url.clone(),
                    status_code: 200,
                    content_type: Some("text/plain".to_string()),
                    format: Some("documentation".to_string()),
                    content: Some(format!("<!-- Source: {} -->\n\n{}", source, content)),
                    ..Default::default()
                });
            }
        }

        // No llms.txt — fetch the docs page directly and return raw content
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, ua_header);
        headers.insert(
            ACCEPT,
            HeaderValue::from_static("text/html, text/plain, text/markdown, */*"),
        );
        let headers = apply_bot_auth_if_enabled(headers, options, &url);
        let (response, redirect_chain) = send_request_following_redirects(
            url,
            reqwest::Method::GET,
            headers,
            options,
            PROBE_TIMEOUT,
        )
        .await?;

        let status_code = response.status;
        let final_url = response.url.to_string();
        let content_type = header_value(&response.headers, "content-type").map(|s| s.to_string());

        let body_bytes = read_full_body(response, options).await?;
        let body = String::from_utf8_lossy(&body_bytes).into_owned();

        // If HTML, convert to markdown for cleaner docs consumption
        let (content, format) = if content_type
            .as_deref()
            .is_some_and(|ct| ct.contains("text/html"))
        {
            (
                crate::convert::html_to_markdown(&body),
                "markdown".to_string(),
            )
        } else {
            (body, "documentation".to_string())
        };

        Ok(FetchResponse {
            url: final_url,
            status_code,
            content_type,
            format: Some(format),
            content: Some(content),
            redirect_chain,
            ..Default::default()
        })
    }
}

/// Fetch a direct llms.txt URL
async fn fetch_llms_txt_direct(
    url: Url,
    ua_header: HeaderValue,
    options: &FetchOptions,
) -> Result<FetchResponse, FetchError> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, ua_header);
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/plain, text/markdown, */*"),
    );
    let headers = apply_bot_auth_if_enabled(headers, options, &url);
    let (response, redirect_chain) = send_request_following_redirects(
        url,
        reqwest::Method::GET,
        headers,
        options,
        PROBE_TIMEOUT,
    )
    .await?;

    let status_code = response.status;
    let final_url = response.url.to_string();
    let content_type = header_value(&response.headers, "content-type").map(|s| s.to_string());

    if !(200..300).contains(&status_code) {
        return Ok(FetchResponse {
            url: final_url,
            status_code,
            redirect_chain,
            error: Some(format!("HTTP {}", status_code)),
            ..Default::default()
        });
    }

    let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);
    let (body, truncated) = read_body_with_timeout(response, BODY_TIMEOUT, max_body_size).await?;
    let size = body.len() as u64;
    let mut content = String::from_utf8_lossy(&body).to_string();

    if truncated {
        content.push_str(TRUNCATION_MESSAGE);
    }

    Ok(FetchResponse {
        url: final_url,
        status_code: 200,
        content_type,
        format: Some("documentation".to_string()),
        content: Some(content),
        size: Some(size),
        truncated: if truncated { Some(true) } else { None },
        redirect_chain,
        ..Default::default()
    })
}

/// Try to fetch an llms.txt URL. Returns Some(content) on success.
async fn try_fetch_llms_txt(
    url: Url,
    ua_header: HeaderValue,
    options: &FetchOptions,
) -> Option<String> {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, ua_header);
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("text/plain, text/markdown, */*"),
    );
    let headers = apply_bot_auth_if_enabled(headers, options, &url);
    let (response, _) = send_request_following_redirects(
        url,
        reqwest::Method::GET,
        headers,
        options,
        PROBE_TIMEOUT,
    )
    .await
    .ok()?;

    if !(200..300).contains(&response.status) {
        return None;
    }

    // Reject HTML error pages masquerading as 200 OK
    let content_type = header_value(&response.headers, "content-type").unwrap_or("");

    if content_type.contains("text/html") {
        return None;
    }

    // Cap at one byte over the limit so the size check below still rejects oversize bodies.
    let (body, truncated) = read_body_with_timeout(response, BODY_TIMEOUT, MAX_LLMS_TXT_SIZE + 1)
        .await
        .ok()?;

    if truncated || body.len() > MAX_LLMS_TXT_SIZE {
        return None;
    }

    let text = String::from_utf8(body.to_vec()).ok()?;

    if text.trim().is_empty() {
        return None;
    }

    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_llms_txt_url() {
        let url = Url::parse("https://example.com/llms.txt").unwrap();
        assert!(DocsSiteFetcher::is_llms_txt_url(&url));

        let url = Url::parse("https://example.com/llms-full.txt").unwrap();
        assert!(DocsSiteFetcher::is_llms_txt_url(&url));

        let url = Url::parse("https://example.com/other.txt").unwrap();
        assert!(!DocsSiteFetcher::is_llms_txt_url(&url));
    }

    #[test]
    fn test_is_docs_site() {
        // ReadTheDocs
        let url = Url::parse("https://my-project.readthedocs.io/en/latest/").unwrap();
        assert!(DocsSiteFetcher::is_docs_site(&url));

        // docs.rs
        let url = Url::parse("https://docs.rs/tokio/latest/tokio/").unwrap();
        assert!(DocsSiteFetcher::is_docs_site(&url));

        // docs. prefix
        let url = Url::parse("https://docs.python.org/3/library/").unwrap();
        assert!(DocsSiteFetcher::is_docs_site(&url));

        // developer. prefix
        let url = Url::parse("https://developer.mozilla.org/en-US/docs/Web").unwrap();
        assert!(DocsSiteFetcher::is_docs_site(&url));

        // GitBook
        let url = Url::parse("https://my-project.gitbook.io/docs/").unwrap();
        assert!(DocsSiteFetcher::is_docs_site(&url));

        // Non-docs site
        let url = Url::parse("https://github.com/owner/repo").unwrap();
        assert!(!DocsSiteFetcher::is_docs_site(&url));

        let url = Url::parse("https://example.com/page").unwrap();
        assert!(!DocsSiteFetcher::is_docs_site(&url));
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = DocsSiteFetcher::new();

        // llms.txt URLs match
        let url = Url::parse("https://example.com/llms.txt").unwrap();
        assert!(fetcher.matches(&url));

        // Docs sites match
        let url = Url::parse("https://docs.rs/tokio/latest/tokio/").unwrap();
        assert!(fetcher.matches(&url));

        // Non-docs sites don't match
        let url = Url::parse("https://github.com/owner/repo").unwrap();
        assert!(!fetcher.matches(&url));
    }
}
