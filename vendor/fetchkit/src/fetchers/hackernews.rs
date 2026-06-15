//! Hacker News thread fetcher
//!
//! Handles news.ycombinator.com/item?id={id} URLs, returning structured
//! thread content via the HN Firebase API.

use crate::client::FetchOptions;
use crate::error::FetchError;
use crate::fetchers::default::{read_full_body, send_request_following_redirects};
use crate::fetchers::Fetcher;
use crate::types::{FetchRequest, FetchResponse};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

const API_TIMEOUT: Duration = Duration::from_secs(10);

/// Max top-level comments to fetch
const MAX_COMMENTS: usize = 20;

/// Upper bound for accepted Unix timestamps (3000-01-01T00:00:00Z)
const MAX_UNIX_TIMESTAMP: u64 = 32_503_680_000;

/// Hacker News fetcher
///
/// Matches `news.ycombinator.com/item?id={id}`, returning structured
/// thread content via the HN Firebase API.
pub struct HackerNewsFetcher;

impl HackerNewsFetcher {
    pub fn new() -> Self {
        Self
    }

    fn parse_url(url: &Url) -> Option<u64> {
        let host = url.host_str()?;
        if host != "news.ycombinator.com" {
            return None;
        }

        let segments: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();
        if segments.first() != Some(&"item") {
            return None;
        }

        url.query_pairs()
            .find(|(k, _)| k == "id")
            .and_then(|(_, v)| v.parse().ok())
    }
}

impl Default for HackerNewsFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct HNItem {
    id: u64,
    #[serde(rename = "type")]
    item_type: Option<String>,
    title: Option<String>,
    text: Option<String>,
    url: Option<String>,
    by: Option<String>,
    score: Option<i64>,
    time: Option<u64>,
    descendants: Option<u64>,
    kids: Option<Vec<u64>>,
}

#[async_trait]
impl Fetcher for HackerNewsFetcher {
    fn name(&self) -> &'static str {
        "hackernews"
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

        let item_id = Self::parse_url(&url)
            .ok_or_else(|| FetchError::FetcherError("Not a valid HN URL".to_string()))?;

        let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
        let ua_header = HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT));

        // Fetch the item
        let item = fetch_item(options, &ua_header, item_id).await?;

        // Fetch top-level comments
        let comments = if let Some(kids) = &item.kids {
            let mut comments = Vec::new();
            for &kid_id in kids.iter().take(MAX_COMMENTS) {
                if let Ok(comment) = fetch_item(options, &ua_header, kid_id).await {
                    // Fetch one level of replies
                    let replies = if let Some(reply_ids) = &comment.kids {
                        let mut replies = Vec::new();
                        for &reply_id in reply_ids.iter().take(5) {
                            if let Ok(reply) = fetch_item(options, &ua_header, reply_id).await {
                                replies.push(reply);
                            }
                        }
                        replies
                    } else {
                        Vec::new()
                    };
                    comments.push((comment, replies));
                }
            }
            comments
        } else {
            Vec::new()
        };

        let content = format_hn_response(&item, &comments);

        Ok(FetchResponse {
            url: request.url.clone(),
            status_code: 200,
            content_type: Some("text/markdown".to_string()),
            format: Some("hackernews".to_string()),
            content: Some(content),
            ..Default::default()
        })
    }
}

async fn fetch_item(
    options: &FetchOptions,
    ua: &HeaderValue,
    id: u64,
) -> Result<HNItem, FetchError> {
    let url = format!("https://hacker-news.firebaseio.com/v0/item/{}.json", id);
    let parsed = Url::parse(&url).map_err(|_| FetchError::InvalidUrlScheme)?;

    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, ua.clone());

    // THREAT[TM-SSRF-010]: manual redirect following re-validates each hop.
    let (resp, _redirect_chain) = send_request_following_redirects(
        parsed,
        reqwest::Method::GET,
        headers,
        options,
        API_TIMEOUT,
    )
    .await?;

    if !(200..300).contains(&resp.status) {
        return Err(FetchError::FetcherError(format!(
            "HN API error: HTTP {}",
            resp.status
        )));
    }

    let body = read_full_body(resp, options).await?;
    serde_json::from_slice(&body)
        .map_err(|e| FetchError::FetcherError(format!("Failed to parse HN item: {}", e)))
}

fn format_hn_response(item: &HNItem, comments: &[(HNItem, Vec<HNItem>)]) -> String {
    let mut out = String::new();

    let item_type = item.item_type.as_deref().unwrap_or("story");

    // Title
    let title = item.title.as_deref().unwrap_or("Hacker News Item");
    out.push_str(&format!("# {}\n\n", title));

    // Metadata
    out.push_str("## Info\n\n");
    out.push_str(&format!("- **Type:** {}\n", item_type));

    if let Some(by) = &item.by {
        out.push_str(&format!("- **By:** {}\n", by));
    }
    if let Some(score) = item.score {
        out.push_str(&format!("- **Score:** {}\n", score));
    }
    if let Some(time) = item.time.and_then(format_unix_timestamp_bounded) {
        out.push_str(&format!("- **Time:** {}\n", time));
    }
    if let Some(descendants) = item.descendants {
        out.push_str(&format!("- **Comments:** {}\n", descendants));
    }
    if let Some(url) = &item.url {
        out.push_str(&format!("- **Link:** {}\n", url));
    }
    out.push_str(&format!(
        "- **HN URL:** https://news.ycombinator.com/item?id={}\n",
        item.id
    ));

    // Story text (for Ask HN, Show HN, etc.)
    if let Some(text) = &item.text {
        let cleaned = strip_html_tags(text);
        out.push_str(&format!("\n{}\n", cleaned));
    }

    // Comments
    if !comments.is_empty() {
        let total = item.descendants.unwrap_or(0);
        let shown: usize = comments.len() + comments.iter().map(|(_, r)| r.len()).sum::<usize>();
        if shown < total as usize {
            out.push_str(&format!("\n---\n\n## Comments ({} of {})\n", shown, total));
        } else {
            out.push_str(&format!("\n---\n\n## Comments ({})\n", shown));
        }

        for (comment, replies) in comments {
            format_comment(&mut out, comment, 0);
            for reply in replies {
                format_comment(&mut out, reply, 1);
            }
        }
    }

    out
}

/// Format a Unix timestamp as an ISO 8601 UTC date-time string
fn format_unix_timestamp_bounded(ts: u64) -> Option<String> {
    if ts > MAX_UNIX_TIMESTAMP {
        return None;
    }

    Some(format_unix_timestamp(ts))
}

/// Format a Unix timestamp as an ISO 8601 UTC date-time string
fn format_unix_timestamp(ts: u64) -> String {
    let secs = ts % 60;
    let mins = (ts / 60) % 60;
    let hours = (ts / 3600) % 24;

    // Days since epoch
    let mut days = (ts / 86400) as i64;

    // Calculate year/month/day from days since epoch
    let mut year = 1970i64;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let leap = is_leap_year(year);
    let days_in_months: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];

    let mut month = 0;
    for (i, &dim) in days_in_months.iter().enumerate() {
        if days < dim {
            month = i + 1;
            break;
        }
        days -= dim;
    }
    let day = days + 1;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hours, mins, secs
    )
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn format_comment(out: &mut String, comment: &HNItem, depth: usize) {
    let indent = "> ".repeat(depth);
    let by = comment.by.as_deref().unwrap_or("anonymous");

    out.push_str(&format!("\n{}**{}**\n\n", indent, by));

    if let Some(text) = &comment.text {
        let cleaned = strip_html_tags(text);
        for line in cleaned.lines() {
            out.push_str(&format!("{}{}\n", indent, line));
        }
        out.push('\n');
    }
}

/// Simple HTML tag stripper for HN comment text
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;

    for (idx, c) in html.char_indices() {
        match c {
            '<' => {
                in_tag = true;
                // Check for <p> tags -> newlines
                let rest: String = html[idx + c.len_utf8()..].chars().take(3).collect();
                if rest.starts_with("p>") || rest.starts_with("br") {
                    result.push('\n');
                }
            }
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }

    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&#39;", "'")
        .replace("&#x2F;", "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hn_url() {
        let url = Url::parse("https://news.ycombinator.com/item?id=12345").unwrap();
        assert_eq!(HackerNewsFetcher::parse_url(&url), Some(12345));
    }

    #[test]
    fn test_rejects_non_hn() {
        let url = Url::parse("https://example.com/item?id=123").unwrap();
        assert_eq!(HackerNewsFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_non_item_path() {
        let url = Url::parse("https://news.ycombinator.com/newest").unwrap();
        assert_eq!(HackerNewsFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_rejects_no_id() {
        let url = Url::parse("https://news.ycombinator.com/item").unwrap();
        assert_eq!(HackerNewsFetcher::parse_url(&url), None);
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = HackerNewsFetcher::new();

        let url = Url::parse("https://news.ycombinator.com/item?id=123").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://example.com/item?id=123").unwrap();
        assert!(!fetcher.matches(&url));
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(strip_html_tags("Hello <b>world</b>"), "Hello world");
        assert_eq!(strip_html_tags("a &amp; b"), "a & b");
        assert_eq!(strip_html_tags("ab<é>xy<"), "abxy");
    }

    #[test]
    fn test_format_hn_response() {
        let item = HNItem {
            id: 42,
            item_type: Some("story".to_string()),
            title: Some("Show HN: My Project".to_string()),
            text: None,
            url: Some("https://example.com".to_string()),
            by: Some("pg".to_string()),
            score: Some(100),
            time: Some(1704067200), // 2024-01-01T00:00:00Z
            descendants: Some(5),
            kids: None,
        };

        let output = format_hn_response(&item, &[]);

        assert!(output.contains("# Show HN: My Project"));
        assert!(output.contains("**By:** pg"));
        assert!(output.contains("**Score:** 100"));
        assert!(output.contains("**Time:** 2024-01-01T00:00:00Z"));
        assert!(output.contains("https://example.com"));
    }

    #[test]
    fn test_format_hn_response_with_comments() {
        let item = HNItem {
            id: 42,
            item_type: Some("story".to_string()),
            title: Some("Test Story".to_string()),
            text: None,
            url: None,
            by: Some("user1".to_string()),
            score: Some(50),
            time: None,
            descendants: Some(2),
            kids: None,
        };

        let comment = HNItem {
            id: 43,
            item_type: Some("comment".to_string()),
            title: None,
            text: Some("Great post!".to_string()),
            url: None,
            by: Some("user2".to_string()),
            score: None,
            time: None,
            descendants: None,
            kids: None,
        };

        let output = format_hn_response(&item, &[(comment, vec![])]);
        assert!(output.contains("## Comments"));
        assert!(output.contains("**user2**"));
        assert!(output.contains("Great post!"));
    }

    #[test]
    fn test_format_hn_response_ask_hn() {
        let item = HNItem {
            id: 100,
            item_type: Some("story".to_string()),
            title: Some("Ask HN: Best Rust crates?".to_string()),
            text: Some("<p>Looking for recommendations.</p>".to_string()),
            url: None,
            by: Some("asker".to_string()),
            score: Some(25),
            time: None,
            descendants: Some(0),
            kids: None,
        };

        let output = format_hn_response(&item, &[]);
        assert!(output.contains("Ask HN: Best Rust crates?"));
        assert!(output.contains("Looking for recommendations."));
    }

    #[test]
    fn test_format_unix_timestamp() {
        assert_eq!(format_unix_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_timestamp(1704067200), "2024-01-01T00:00:00Z");
    }

    #[test]
    fn test_format_unix_timestamp_bounded() {
        assert_eq!(
            format_unix_timestamp_bounded(1704067200),
            Some("2024-01-01T00:00:00Z".to_string())
        );
        assert_eq!(format_unix_timestamp_bounded(u64::MAX), None);
    }
}
