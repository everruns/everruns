//! Wikipedia article fetcher
//!
//! Handles wikipedia.org/wiki/{title} URLs, returning clean article content
//! via the MediaWiki REST API.

use crate::client::FetchOptions;
use crate::error::FetchError;
use crate::fetchers::default::{
    read_body_with_timeout, send_request_following_redirects, BODY_TIMEOUT, DEFAULT_MAX_BODY_SIZE,
    TRUNCATION_MESSAGE,
};
use crate::fetchers::Fetcher;
use crate::types::{FetchRequest, FetchResponse};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

const API_TIMEOUT: Duration = Duration::from_secs(10);

/// Wikipedia fetcher
///
/// Matches `https://{lang}.wikipedia.org/wiki/{title}` and returns
/// article summary and content via the MediaWiki REST API.
pub struct WikipediaFetcher;

impl WikipediaFetcher {
    pub fn new() -> Self {
        Self
    }

    /// Extract language and title from a Wikipedia URL
    fn parse_url(url: &Url) -> Option<(String, String)> {
        let host = url.host_str()?;

        // Must be {lang}.wikipedia.org
        let lang = host.strip_suffix(".wikipedia.org")?;
        if lang.is_empty() || lang.contains('.') {
            return None;
        }

        let segments: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();

        // Must be /wiki/{title}
        if segments.len() < 2 || segments[0] != "wiki" {
            return None;
        }

        let title = segments[1..].join("/");
        if title.is_empty() {
            return None;
        }

        Some((lang.to_string(), title))
    }
}

impl Default for WikipediaFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct WikiSummary {
    title: String,
    extract: Option<String>,
    description: Option<String>,
    content_urls: Option<ContentUrls>,
    /// Redirect target — populated when the requested title redirects
    #[serde(default)]
    titles: Option<WikiTitles>,
}

#[derive(Debug, Deserialize)]
struct WikiTitles {
    canonical: Option<String>,
    #[allow(dead_code)]
    normalized: Option<String>,
    display: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentUrls {
    desktop: Option<DesktopUrl>,
}

#[derive(Debug, Deserialize)]
struct DesktopUrl {
    page: Option<String>,
}

#[async_trait]
impl Fetcher for WikipediaFetcher {
    fn name(&self) -> &'static str {
        "wikipedia"
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

        let (lang, title) = Self::parse_url(&url)
            .ok_or_else(|| FetchError::FetcherError("Not a valid Wikipedia URL".to_string()))?;

        let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
        let ua_header = HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT));

        // Fetch summary via REST API
        let summary_url = format!(
            "https://{}.wikipedia.org/api/rest_v1/page/summary/{}",
            lang, title
        );
        let parsed_summary = Url::parse(&summary_url).map_err(|_| FetchError::InvalidUrlScheme)?;

        let mut summary_headers = HeaderMap::new();
        summary_headers.insert(USER_AGENT, ua_header.clone());
        summary_headers.insert(ACCEPT, HeaderValue::from_static("application/json"));

        // THREAT[TM-SSRF-010]: manual redirect following re-validates each hop.
        let (summary_resp, _) = send_request_following_redirects(
            parsed_summary,
            reqwest::Method::GET,
            summary_headers,
            options,
            API_TIMEOUT,
        )
        .await?;

        let status_code = summary_resp.status;
        if !(200..300).contains(&status_code) {
            let error_msg = if status_code == 404 {
                format!("Article '{}' not found on {}.wikipedia.org", title, lang)
            } else {
                format!("Wikipedia API error: HTTP {}", status_code)
            };
            return Ok(FetchResponse {
                url: request.url.clone(),
                status_code,
                error: Some(error_msg),
                ..Default::default()
            });
        }

        let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);
        let (summary_body, _) =
            read_body_with_timeout(summary_resp, BODY_TIMEOUT, max_body_size).await?;
        let summary: WikiSummary = serde_json::from_slice(&summary_body).map_err(|e| {
            FetchError::FetcherError(format!("Failed to parse Wikipedia data: {}", e))
        })?;

        // Also fetch full HTML content and convert to markdown
        let html_url = format!(
            "https://{}.wikipedia.org/api/rest_v1/page/html/{}",
            lang, title
        );

        let full_content = match Url::parse(&html_url) {
            Ok(parsed_html) => {
                let mut html_headers = HeaderMap::new();
                html_headers.insert(USER_AGENT, ua_header);
                match send_request_following_redirects(
                    parsed_html,
                    reqwest::Method::GET,
                    html_headers,
                    options,
                    API_TIMEOUT,
                )
                .await
                {
                    Ok((resp, _)) if (200..300).contains(&resp.status) => {
                        let (html_body, truncated) =
                            read_body_with_timeout(resp, BODY_TIMEOUT, max_body_size).await?;
                        let html = String::from_utf8_lossy(&html_body);
                        let mut markdown = crate::convert::html_to_markdown(&html);
                        if truncated {
                            markdown.push_str(TRUNCATION_MESSAGE);
                        }
                        Some(markdown)
                    }
                    _ => None,
                }
            }
            Err(_) => None,
        };

        let content = format_wikipedia_response(&summary, full_content.as_deref(), &lang);

        Ok(FetchResponse {
            url: request.url.clone(),
            status_code: 200,
            content_type: Some("text/markdown".to_string()),
            format: Some("wikipedia".to_string()),
            content: Some(content),
            ..Default::default()
        })
    }
}

fn format_wikipedia_response(
    summary: &WikiSummary,
    full_content: Option<&str>,
    lang: &str,
) -> String {
    let mut out = String::new();

    out.push_str(&format!("# {}\n\n", summary.title));

    if let Some(desc) = &summary.description {
        out.push_str(&format!("*{}*\n\n", desc));
    }

    out.push_str(&format!("- **Language:** {}\n", lang));

    if let Some(urls) = &summary.content_urls {
        if let Some(desktop) = &urls.desktop {
            if let Some(page) = &desktop.page {
                out.push_str(&format!("- **URL:** {}\n", page));
            }
        }
    }

    // Show redirect info if the canonical title differs from the display title
    if let Some(titles) = &summary.titles {
        if let (Some(canonical), Some(display)) = (&titles.canonical, &titles.display) {
            if canonical != display {
                out.push_str(&format!("- **Redirected from:** {}\n", display));
            }
        }
    }

    // Use full content if available, otherwise use summary extract
    if let Some(content) = full_content {
        out.push_str(&format!("\n---\n\n{}", content));
    } else if let Some(extract) = &summary.extract {
        out.push_str(&format!("\n## Summary\n\n{}\n", extract));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_wikipedia_url() {
        let url = Url::parse("https://en.wikipedia.org/wiki/Rust_(programming_language)").unwrap();
        assert_eq!(
            WikipediaFetcher::parse_url(&url),
            Some(("en".to_string(), "Rust_(programming_language)".to_string()))
        );
    }

    #[test]
    fn test_parse_other_language() {
        let url = Url::parse("https://de.wikipedia.org/wiki/Berlin").unwrap();
        assert_eq!(
            WikipediaFetcher::parse_url(&url),
            Some(("de".to_string(), "Berlin".to_string()))
        );
    }

    #[test]
    fn test_parse_subpage_url() {
        let url = Url::parse("https://en.wikipedia.org/wiki/Rust/History").unwrap();
        assert_eq!(
            WikipediaFetcher::parse_url(&url),
            Some(("en".to_string(), "Rust/History".to_string()))
        );
    }

    #[test]
    fn test_parse_mobile_url() {
        // Mobile URLs use m.wikipedia.org, not {lang}.wikipedia.org
        let url = Url::parse("https://m.wikipedia.org/wiki/Rust").unwrap();
        assert_eq!(
            WikipediaFetcher::parse_url(&url),
            Some(("m".to_string(), "Rust".to_string()))
        );
    }

    #[test]
    fn test_rejects_non_wiki_path() {
        let url = Url::parse("https://en.wikipedia.org/w/index.php?title=Rust").unwrap();
        assert_eq!(WikipediaFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_non_wikipedia() {
        let url = Url::parse("https://example.org/wiki/Test").unwrap();
        assert_eq!(WikipediaFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_bare_wiki_path() {
        let url = Url::parse("https://en.wikipedia.org/wiki").unwrap();
        assert_eq!(WikipediaFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_subdomain_wikipedia() {
        // sub.sub.wikipedia.org shouldn't match (contains dot)
        let url = Url::parse("https://upload.wikimedia.wikipedia.org/wiki/Test").unwrap();
        assert_eq!(WikipediaFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = WikipediaFetcher::new();

        let url = Url::parse("https://en.wikipedia.org/wiki/Rust").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://fr.wikipedia.org/wiki/Paris").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://example.com/wiki/Rust").unwrap();
        assert!(!fetcher.matches(&url));
    }

    #[test]
    fn test_format_wikipedia_response_summary_only() {
        let summary = WikiSummary {
            title: "Rust (programming language)".to_string(),
            extract: Some("Rust is a systems programming language.".to_string()),
            description: Some("Programming language".to_string()),
            content_urls: None,
            titles: None,
        };

        let output = format_wikipedia_response(&summary, None, "en");

        assert!(output.contains("# Rust (programming language)"));
        assert!(output.contains("*Programming language*"));
        assert!(output.contains("**Language:** en"));
        assert!(output.contains("Rust is a systems programming language."));
    }

    #[test]
    fn test_format_wikipedia_response_with_full_content() {
        let summary = WikiSummary {
            title: "Rust".to_string(),
            extract: Some("Short extract.".to_string()),
            description: None,
            content_urls: Some(ContentUrls {
                desktop: Some(DesktopUrl {
                    page: Some("https://en.wikipedia.org/wiki/Rust".to_string()),
                }),
            }),
            titles: None,
        };

        let output = format_wikipedia_response(&summary, Some("# Full article content"), "en");

        assert!(output.contains("# Rust"));
        assert!(output.contains("**URL:** https://en.wikipedia.org/wiki/Rust"));
        // Full content should be used instead of extract
        assert!(output.contains("Full article content"));
        assert!(!output.contains("Short extract."));
    }

    #[test]
    fn test_format_wikipedia_response_with_redirect() {
        let summary = WikiSummary {
            title: "Rust (programming language)".to_string(),
            extract: Some("Rust is...".to_string()),
            description: None,
            content_urls: None,
            titles: Some(WikiTitles {
                canonical: Some("Rust (programming language)".to_string()),
                normalized: Some("Rust (programming language)".to_string()),
                display: Some("Rust programming language".to_string()),
            }),
        };

        let output = format_wikipedia_response(&summary, None, "en");

        assert!(output.contains("**Redirected from:** Rust programming language"));
    }

    #[test]
    fn test_format_wikipedia_response_no_redirect_when_same() {
        let summary = WikiSummary {
            title: "Rust".to_string(),
            extract: Some("Rust is...".to_string()),
            description: None,
            content_urls: None,
            titles: Some(WikiTitles {
                canonical: Some("Rust".to_string()),
                normalized: Some("Rust".to_string()),
                display: Some("Rust".to_string()),
            }),
        };

        let output = format_wikipedia_response(&summary, None, "en");

        assert!(!output.contains("Redirected from"));
    }
}
