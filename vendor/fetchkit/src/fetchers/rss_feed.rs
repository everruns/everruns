//! RSS/Atom feed fetcher
//!
//! Detects RSS and Atom feeds (by URL pattern or content-type) and returns
//! structured feed entries optimized for LLM consumption.

use crate::client::FetchOptions;
use crate::error::FetchError;
use crate::fetchers::default::{
    apply_bot_auth_if_enabled, header_value, read_body_with_timeout,
    send_request_following_redirects, BODY_TIMEOUT, DEFAULT_MAX_BODY_SIZE,
};
use crate::fetchers::Fetcher;
use crate::types::{FetchRequest, FetchResponse};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use std::time::Duration;
use url::Url;

const API_TIMEOUT: Duration = Duration::from_secs(10);

/// Max entries to include
const MAX_ENTRIES: usize = 20;

/// RSS/Atom feed fetcher
///
/// Matches common feed URL patterns and parses RSS 2.0 / Atom 1.0 feeds
/// into structured markdown entries.
pub struct RSSFeedFetcher;

impl RSSFeedFetcher {
    pub fn new() -> Self {
        Self
    }

    /// Check if a URL looks like a feed URL by path pattern.
    ///
    /// Content-type detection (application/rss+xml, application/atom+xml)
    /// happens at fetch time since we can't know the content-type from the URL alone.
    fn is_feed_url(url: &Url) -> bool {
        let path = url.path().to_lowercase();

        // Common feed URL patterns
        path.ends_with("/feed")
            || path.ends_with("/feed/")
            || path.ends_with("/rss")
            || path.ends_with("/rss/")
            || path.ends_with("/atom")
            || path.ends_with("/atom/")
            || path.ends_with("/rss.xml")
            || path.ends_with("/atom.xml")
            || path.ends_with("/feed.xml")
            || path.ends_with("/index.xml")
            || path.ends_with("/feed.rss")
            || path.ends_with("/feed.atom")
            || path.ends_with(".rss")
            || path == "/rss"
            || path == "/feed"
    }

    /// Check if a content-type indicates a feed format
    fn is_feed_content_type(content_type: &str) -> bool {
        let ct = content_type.to_lowercase();
        ct.contains("application/rss+xml")
            || ct.contains("application/atom+xml")
            || ct.contains("text/xml")
            || ct.contains("application/xml")
    }
}

impl Default for RSSFeedFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Fetcher for RSSFeedFetcher {
    fn name(&self) -> &'static str {
        "rss_feed"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::is_feed_url(url)
    }

    async fn fetch(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
    ) -> Result<FetchResponse, FetchError> {
        let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
        let mut headers = HeaderMap::new();
        let ua_header = HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT));
        headers.insert(USER_AGENT, ua_header);
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "application/rss+xml, application/atom+xml, application/xml, text/xml, */*",
            ),
        );

        let parsed_url = Url::parse(&request.url).map_err(|_| FetchError::InvalidUrlScheme)?;
        let headers = apply_bot_auth_if_enabled(headers, options, &parsed_url);
        let (response, redirect_chain) = send_request_following_redirects(
            parsed_url,
            reqwest::Method::GET,
            headers,
            options,
            API_TIMEOUT,
        )
        .await?;

        let status_code = response.status;
        let final_url = response.url.to_string();
        if !(200..300).contains(&status_code) {
            return Ok(FetchResponse {
                url: final_url,
                status_code,
                redirect_chain,
                error: Some(format!("HTTP {}", status_code)),
                ..Default::default()
            });
        }

        // Check content-type for feed detection (covers non-URL-pattern feeds)
        let content_type = header_value(&response.headers, reqwest::header::CONTENT_TYPE.as_str())
            .unwrap_or("")
            .to_string();

        let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);
        let (body, _truncated) =
            read_body_with_timeout(response, BODY_TIMEOUT, max_body_size).await?;
        let body = String::from_utf8_lossy(&body).into_owned();

        // Detect feed type: by XML structure first, then content-type
        let is_feed_by_ct = Self::is_feed_content_type(&content_type);
        let content = if body.contains("<rss") || body.contains("<channel>") {
            parse_rss(&body)
        } else if body.contains("<feed") && body.contains("xmlns=\"http://www.w3.org/2005/Atom\"") {
            parse_atom(&body)
        } else if body.contains("<feed") {
            // Atom without explicit namespace
            parse_atom(&body)
        } else if is_feed_by_ct {
            // Content-type indicates a feed but structure wasn't recognized — return as raw XML
            return Ok(FetchResponse {
                url: final_url,
                status_code: 200,
                content: Some(body),
                format: Some("raw".to_string()),
                redirect_chain,
                ..Default::default()
            });
        } else {
            // Not a recognized feed format
            return Ok(FetchResponse {
                url: final_url,
                status_code: 200,
                content: Some(body),
                format: Some("raw".to_string()),
                redirect_chain,
                ..Default::default()
            });
        };

        Ok(FetchResponse {
            url: final_url,
            status_code: 200,
            content_type: Some("text/markdown".to_string()),
            format: Some("rss_feed".to_string()),
            content: Some(content),
            redirect_chain,
            ..Default::default()
        })
    }
}

/// Parse RSS 2.0 feed into markdown
fn parse_rss(xml: &str) -> String {
    let mut out = String::new();

    // Feed title
    let feed_title = extract_first_tag(xml, "title").unwrap_or("RSS Feed".to_string());
    out.push_str(&format!("# {}\n\n", decode_entities(&feed_title)));

    // Feed description
    if let Some(desc) = extract_first_tag(xml, "description") {
        out.push_str(&format!("{}\n\n", decode_entities(&desc)));
    }

    // Feed link
    if let Some(link) = extract_first_tag(xml, "link") {
        out.push_str(&format!("- **Link:** {}\n", link));
    }

    // Items
    let items = extract_blocks(xml, "item");
    if !items.is_empty() {
        out.push_str(&format!(
            "\n## Entries ({})\n",
            items.len().min(MAX_ENTRIES)
        ));

        for item_xml in items.iter().take(MAX_ENTRIES) {
            let title = extract_first_tag(item_xml, "title")
                .map(|t| decode_entities(&t))
                .unwrap_or_else(|| "(untitled)".to_string());
            let link = extract_first_tag(item_xml, "link").unwrap_or_default();
            let pub_date = extract_first_tag(item_xml, "pubDate");
            let description =
                extract_first_tag(item_xml, "description").map(|d| decode_entities(&d));

            out.push_str(&format!("\n### {}\n\n", title));
            if !link.is_empty() {
                out.push_str(&format!("- **Link:** {}\n", link));
            }
            if let Some(date) = pub_date {
                out.push_str(&format!("- **Published:** {}\n", date));
            }
            if let Some(desc) = description {
                let converted = convert_entry_content(&desc);
                if !converted.is_empty() {
                    let truncated = if converted.len() > 500 {
                        format!("{}...", &converted[..500])
                    } else {
                        converted
                    };
                    out.push_str(&format!("\n{}\n", truncated));
                }
            }
        }
    }

    out
}

/// Parse Atom 1.0 feed into markdown
fn parse_atom(xml: &str) -> String {
    let mut out = String::new();

    // Feed title
    let feed_title = extract_first_tag(xml, "title").unwrap_or("Atom Feed".to_string());
    out.push_str(&format!("# {}\n\n", decode_entities(&feed_title)));

    // Feed subtitle
    if let Some(subtitle) = extract_first_tag(xml, "subtitle") {
        out.push_str(&format!("{}\n\n", decode_entities(&subtitle)));
    }

    // Entries
    let entries = extract_blocks(xml, "entry");
    if !entries.is_empty() {
        out.push_str(&format!(
            "\n## Entries ({})\n",
            entries.len().min(MAX_ENTRIES)
        ));

        for entry_xml in entries.iter().take(MAX_ENTRIES) {
            let title = extract_first_tag(entry_xml, "title")
                .map(|t| decode_entities(&t))
                .unwrap_or_else(|| "(untitled)".to_string());

            // Atom links are in <link href="..."/> attributes
            let link = extract_link_href(entry_xml).unwrap_or_default();
            let updated = extract_first_tag(entry_xml, "updated");
            let published = extract_first_tag(entry_xml, "published");
            let summary = extract_first_tag(entry_xml, "summary").map(|s| decode_entities(&s));
            let author = extract_first_tag(entry_xml, "name");

            out.push_str(&format!("\n### {}\n\n", title));
            if !link.is_empty() {
                out.push_str(&format!("- **Link:** {}\n", link));
            }
            if let Some(author) = author {
                out.push_str(&format!("- **Author:** {}\n", author));
            }
            if let Some(date) = published.or(updated) {
                out.push_str(&format!("- **Published:** {}\n", date));
            }
            if let Some(summary) = summary {
                let converted = convert_entry_content(&summary);
                if !converted.is_empty() {
                    let truncated = if converted.len() > 500 {
                        format!("{}...", &converted[..500])
                    } else {
                        converted
                    };
                    out.push_str(&format!("\n{}\n", truncated));
                }
            }
        }
    }

    out
}

/// Extract first occurrence of a tag's text content
fn extract_first_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);

    let start = xml.find(&open)?;
    let content_start = xml[start..].find('>')? + start + 1;

    // Handle CDATA
    let content_end = xml[content_start..].find(&close)? + content_start;
    let content = &xml[content_start..content_end];

    // Strip CDATA wrapper if present
    let content = content
        .strip_prefix("<![CDATA[")
        .and_then(|c| c.strip_suffix("]]>"))
        .unwrap_or(content);

    Some(content.trim().to_string())
}

/// Extract XML blocks delimited by a tag
fn extract_blocks(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{}", tag);
    let close = format!("</{}>", tag);
    let mut results = Vec::new();
    let mut search_from = 0;

    while let Some(start) = xml[search_from..].find(&open) {
        let abs_start = search_from + start;
        if let Some(end) = xml[abs_start..].find(&close) {
            let block = &xml[abs_start..abs_start + end + close.len()];
            results.push(block.to_string());
            search_from = abs_start + end + close.len();
        } else {
            break;
        }
    }

    results
}

/// Extract href from Atom <link> element
fn extract_link_href(xml: &str) -> Option<String> {
    let link_start = xml.find("<link")?;
    let tag_end = xml[link_start..].find('>')? + link_start;
    let tag = &xml[link_start..=tag_end];

    let href_start = tag.find("href=\"")? + 6;
    let href_end = tag[href_start..].find('"')? + href_start;
    Some(tag[href_start..href_end].to_string())
}

/// Decode common XML/HTML entities
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Convert entry content: use html_to_markdown for HTML, plain text for non-HTML
fn convert_entry_content(content: &str) -> String {
    if content.contains('<') && content.contains('>') {
        // Contains HTML tags — convert via html_to_markdown
        crate::convert::html_to_markdown(content)
    } else {
        content.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_feed_url() {
        let url = Url::parse("https://example.com/feed").unwrap();
        assert!(RSSFeedFetcher::is_feed_url(&url));

        let url = Url::parse("https://example.com/rss.xml").unwrap();
        assert!(RSSFeedFetcher::is_feed_url(&url));

        let url = Url::parse("https://example.com/atom.xml").unwrap();
        assert!(RSSFeedFetcher::is_feed_url(&url));

        let url = Url::parse("https://example.com/blog/feed").unwrap();
        assert!(RSSFeedFetcher::is_feed_url(&url));

        let url = Url::parse("https://example.com/index.xml").unwrap();
        assert!(RSSFeedFetcher::is_feed_url(&url));

        let url = Url::parse("https://example.com/page").unwrap();
        assert!(!RSSFeedFetcher::is_feed_url(&url));
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = RSSFeedFetcher::new();

        let url = Url::parse("https://blog.example.com/feed").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://example.com/page").unwrap();
        assert!(!fetcher.matches(&url));
    }

    #[test]
    fn test_parse_rss() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0">
<channel>
<title>My Blog</title>
<description>A test blog</description>
<link>https://example.com</link>
<item>
<title>First Post</title>
<link>https://example.com/first</link>
<pubDate>Mon, 01 Jan 2024 00:00:00 GMT</pubDate>
<description>This is the first post.</description>
</item>
<item>
<title>Second Post</title>
<link>https://example.com/second</link>
<description><![CDATA[<p>HTML content</p>]]></description>
</item>
</channel>
</rss>"#;

        let output = parse_rss(xml);
        assert!(output.contains("# My Blog"));
        assert!(output.contains("A test blog"));
        assert!(output.contains("### First Post"));
        assert!(output.contains("https://example.com/first"));
        assert!(output.contains("This is the first post."));
        assert!(output.contains("### Second Post"));
        assert!(output.contains("HTML content"));
    }

    #[test]
    fn test_parse_atom() {
        let xml = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>My Blog</title>
<subtitle>A test blog</subtitle>
<entry>
<title>First Entry</title>
<link href="https://example.com/first"/>
<published>2024-01-01T00:00:00Z</published>
<author><name>Alice</name></author>
<summary>Entry summary here.</summary>
</entry>
</feed>"#;

        let output = parse_atom(xml);
        assert!(output.contains("# My Blog"));
        assert!(output.contains("### First Entry"));
        assert!(output.contains("https://example.com/first"));
        assert!(output.contains("Alice"));
        assert!(output.contains("Entry summary here."));
    }

    #[test]
    fn test_decode_entities() {
        assert_eq!(decode_entities("a &amp; b"), "a & b");
        assert_eq!(decode_entities("&lt;tag&gt;"), "<tag>");
    }

    #[test]
    fn test_is_feed_content_type() {
        assert!(RSSFeedFetcher::is_feed_content_type("application/rss+xml"));
        assert!(RSSFeedFetcher::is_feed_content_type(
            "application/atom+xml; charset=utf-8"
        ));
        assert!(RSSFeedFetcher::is_feed_content_type("text/xml"));
        assert!(RSSFeedFetcher::is_feed_content_type("application/xml"));
        assert!(!RSSFeedFetcher::is_feed_content_type("text/html"));
        assert!(!RSSFeedFetcher::is_feed_content_type("application/json"));
    }

    #[test]
    fn test_convert_entry_content_html() {
        let html = "<p>Hello <b>world</b></p>";
        let result = convert_entry_content(html);
        assert!(result.contains("Hello"));
        assert!(result.contains("world"));
    }

    #[test]
    fn test_convert_entry_content_plain() {
        let plain = "Just plain text.";
        let result = convert_entry_content(plain);
        assert_eq!(result, "Just plain text.");
    }

    #[test]
    fn test_parse_rss_with_cdata() {
        let xml = r#"<?xml version="1.0"?>
<rss version="2.0">
<channel>
<title>Test Feed</title>
<item>
<title>CDATA Post</title>
<link>https://example.com/cdata</link>
<description><![CDATA[<p>Rich <strong>HTML</strong> content</p>]]></description>
</item>
</channel>
</rss>"#;

        let output = parse_rss(xml);
        assert!(output.contains("# Test Feed"));
        assert!(output.contains("### CDATA Post"));
        assert!(output.contains("HTML"));
    }
}
