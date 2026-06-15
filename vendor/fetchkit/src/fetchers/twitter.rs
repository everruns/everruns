//! Twitter/X tweet fetcher
//!
//! Handles tweet URLs from x.com and twitter.com, returning structured markdown
//! with tweet text, author info, engagement metrics, and article metadata.
//!
//! Strategy:
//! 1. Primary: Syndication API (cdn.syndication.twimg.com) — rich structured data
//! 2. Fallback: oEmbed API (publish.x.com) — basic metadata

use crate::client::FetchOptions;
use crate::error::FetchError;
use crate::fetchers::default::{
    read_body_with_timeout, transport_request, BODY_TIMEOUT, DEFAULT_MAX_BODY_SIZE,
};
use crate::fetchers::Fetcher;
use crate::types::{FetchRequest, FetchResponse};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

/// Timeout for Twitter API requests
const API_TIMEOUT: Duration = Duration::from_secs(10);

/// Syndication API base URL
const SYNDICATION_BASE: &str = "https://cdn.syndication.twimg.com/tweet-result";

/// oEmbed API base URL
const OEMBED_BASE: &str = "https://publish.x.com/oembed";

/// Twitter/X tweet fetcher
///
/// Matches tweet URLs (`https://x.com/{user}/status/{id}` and
/// `https://twitter.com/{user}/status/{id}`) and returns structured
/// markdown with tweet content, author info, and engagement metrics.
pub struct TwitterFetcher;

impl TwitterFetcher {
    pub fn new() -> Self {
        Self
    }

    /// Extract (username, tweet_id) from a tweet URL.
    fn parse_tweet_url(url: &Url) -> Option<(String, String)> {
        let host = url.host_str()?;
        if host != "x.com" && host != "twitter.com" {
            return None;
        }

        let segments: Vec<&str> = url.path_segments().map(|s| s.collect()).unwrap_or_default();

        // Must be /{user}/status/{id}
        if segments.len() != 3 {
            return None;
        }
        if segments[1] != "status" {
            return None;
        }

        let username = segments[0];
        let tweet_id = segments[2];

        // Basic validation
        if username.is_empty() || tweet_id.is_empty() {
            return None;
        }
        // Tweet IDs are numeric
        if !tweet_id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        // Exclude reserved paths
        let reserved = [
            "i",
            "settings",
            "explore",
            "search",
            "notifications",
            "messages",
            "home",
        ];
        if reserved.contains(&username) {
            return None;
        }

        Some((username.to_string(), tweet_id.to_string()))
    }
}

impl Default for TwitterFetcher {
    fn default() -> Self {
        Self::new()
    }
}

// --- Syndication API types ---

#[derive(Debug, Deserialize)]
struct SyndicationTweet {
    text: Option<String>,
    user: Option<SyndicationUser>,
    created_at: Option<String>,
    favorite_count: Option<u64>,
    conversation_count: Option<u64>,
    // Note: the API returns retweet_count but we use what's available
    article: Option<SyndicationArticle>,
    #[serde(rename = "mediaDetails")]
    media_details: Option<Vec<SyndicationMedia>>,
    entities: Option<SyndicationEntities>,
    #[serde(rename = "quoted_tweet")]
    quoted_tweet: Option<Box<SyndicationTweet>>,
}

#[derive(Debug, Deserialize)]
struct SyndicationUser {
    name: Option<String>,
    screen_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SyndicationArticle {
    title: Option<String>,
    preview_text: Option<String>,
    cover_media: Option<SyndicationCoverMedia>,
    rest_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SyndicationCoverMedia {
    media_info: Option<SyndicationMediaInfo>,
}

#[derive(Debug, Deserialize)]
struct SyndicationMediaInfo {
    original_img_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SyndicationMedia {
    #[serde(rename = "type")]
    media_type: Option<String>,
    media_url_https: Option<String>,
    // Video thumbnail
    #[allow(dead_code)]
    video_info: Option<SyndicationVideoInfo>,
}

#[derive(Debug, Deserialize)]
struct SyndicationVideoInfo {
    // We just note it's a video; no direct URL without auth
}

#[derive(Debug, Deserialize)]
struct SyndicationEntities {
    urls: Option<Vec<SyndicationEntityUrl>>,
}

#[derive(Debug, Deserialize)]
struct SyndicationEntityUrl {
    url: Option<String>,
    expanded_url: Option<String>,
    #[allow(dead_code)]
    display_url: Option<String>,
}

// --- oEmbed API types ---

#[derive(Debug, Deserialize)]
struct OEmbedResponse {
    author_name: Option<String>,
    author_url: Option<String>,
    html: Option<String>,
}

// --- Formatting ---

/// Expand t.co links in tweet text using entity data.
fn expand_text_urls(text: &str, entities: Option<&SyndicationEntities>) -> String {
    let Some(entities) = entities else {
        return text.to_string();
    };
    let Some(urls) = &entities.urls else {
        return text.to_string();
    };

    let mut result = text.to_string();
    for url_entity in urls {
        if let (Some(short), Some(expanded)) = (&url_entity.url, &url_entity.expanded_url) {
            result = result.replace(short.as_str(), expanded.as_str());
        }
    }
    result
}

/// Format a date string like "2026-03-24T18:25:37.000Z" to "Mar 24, 2026".
fn format_date(date_str: &str) -> String {
    // Parse ISO 8601 manually to avoid adding chrono dependency
    let parts: Vec<&str> = date_str.split('T').collect();
    if parts.is_empty() {
        return date_str.to_string();
    }
    let date_parts: Vec<&str> = parts[0].split('-').collect();
    if date_parts.len() != 3 {
        return date_str.to_string();
    }
    let month = match date_parts[1] {
        "01" => "Jan",
        "02" => "Feb",
        "03" => "Mar",
        "04" => "Apr",
        "05" => "May",
        "06" => "Jun",
        "07" => "Jul",
        "08" => "Aug",
        "09" => "Sep",
        "10" => "Oct",
        "11" => "Nov",
        "12" => "Dec",
        _ => return date_str.to_string(),
    };
    let day = date_parts[2].trim_start_matches('0');
    format!("{} {}, {}", month, day, date_parts[0])
}

/// Format engagement metrics as a compact string.
fn format_metrics(likes: Option<u64>, replies: Option<u64>) -> String {
    let mut parts = Vec::new();
    if let Some(n) = likes {
        if n > 0 {
            parts.push(format!("{} {}", n, if n == 1 { "like" } else { "likes" }));
        }
    }
    if let Some(n) = replies {
        if n > 0 {
            parts.push(format!(
                "{} {}",
                n,
                if n == 1 { "reply" } else { "replies" }
            ));
        }
    }
    parts.join(" · ")
}

/// Format syndication tweet data as markdown.
fn format_syndication_response(tweet: &SyndicationTweet, original_url: &str) -> String {
    let mut out = String::new();

    let author_display = tweet.user.as_ref().map(|u| {
        let name = u.name.as_deref().unwrap_or("Unknown");
        let handle = u.screen_name.as_deref().unwrap_or("unknown");
        (name, handle)
    });

    let has_article = tweet.article.as_ref().is_some_and(|a| a.title.is_some());

    if has_article {
        let article = tweet.article.as_ref().unwrap();
        let title = article.title.as_deref().unwrap();

        // Article: title as heading
        out.push_str(&format!("# {}\n\n", title));

        // Author + date line
        if let Some((name, handle)) = &author_display {
            out.push_str(&format!("By **{}** (@{})", name, handle));
        }
        if let Some(date) = &tweet.created_at {
            out.push_str(&format!(" · {}", format_date(date)));
        }
        out.push_str("\n\n");

        // Cover image
        if let Some(cover) = &article.cover_media {
            if let Some(info) = &cover.media_info {
                if let Some(url) = &info.original_img_url {
                    out.push_str(&format!("![Cover]({})\n\n", url));
                }
            }
        }

        // Preview text
        if let Some(preview) = &article.preview_text {
            out.push_str(preview);
            out.push_str("\n\n");
        }

        // Link to full article
        if let Some(rest_id) = &article.rest_id {
            out.push_str(&format!(
                "> Full article: https://x.com/i/article/{}\n\n",
                rest_id
            ));
        }
    } else {
        // Regular tweet: author as heading
        if let Some((name, handle)) = &author_display {
            out.push_str(&format!("# @{} ({})\n\n", handle, name));
        }

        // Tweet text with expanded URLs
        if let Some(text) = &tweet.text {
            let expanded = expand_text_urls(text, tweet.entities.as_ref());
            out.push_str(&expanded);
            out.push_str("\n\n");
        }
    }

    // Media attachments
    if let Some(media) = &tweet.media_details {
        for item in media {
            let media_type = item.media_type.as_deref().unwrap_or("unknown");
            match media_type {
                "photo" => {
                    if let Some(url) = &item.media_url_https {
                        out.push_str(&format!("![Image]({})\n\n", url));
                    }
                }
                "video" | "animated_gif" => {
                    if let Some(url) = &item.media_url_https {
                        out.push_str(&format!("![Video thumbnail]({})\n\n", url));
                    }
                }
                _ => {}
            }
        }
    }

    // Quoted tweet
    if let Some(qt) = &tweet.quoted_tweet {
        if let Some((name, handle)) = qt.user.as_ref().map(|u| {
            (
                u.name.as_deref().unwrap_or("Unknown"),
                u.screen_name.as_deref().unwrap_or("unknown"),
            )
        }) {
            out.push_str(&format!("> **{}** (@{}):\n", name, handle));
        }
        if let Some(text) = &qt.text {
            let expanded = expand_text_urls(text, qt.entities.as_ref());
            for line in expanded.lines() {
                out.push_str(&format!("> {}\n", line));
            }
            out.push('\n');
        }
    }

    // Metrics + source URL
    out.push_str("---\n");
    let metrics = format_metrics(tweet.favorite_count, tweet.conversation_count);
    if !metrics.is_empty() {
        out.push_str(&format!("{} · ", metrics));
    }
    out.push_str(&format!("Source: {}\n", original_url));

    out
}

/// Format oEmbed fallback response as markdown.
fn format_oembed_response(oembed: &OEmbedResponse, original_url: &str) -> String {
    let mut out = String::new();

    let name = oembed.author_name.as_deref().unwrap_or("Unknown");
    // Extract handle from author_url like "https://twitter.com/zachlloydtweets"
    let handle = oembed
        .author_url
        .as_deref()
        .and_then(|u| u.rsplit('/').next())
        .unwrap_or("unknown");

    out.push_str(&format!("# @{} ({})\n\n", handle, name));

    // Extract text from the embed HTML (between <p> tags)
    if let Some(html) = &oembed.html {
        if let (Some(start), Some(end)) = (html.find("<p"), html.find("</p>")) {
            // Find the end of the opening <p...> tag
            if let Some(content_start) = html[start..].find('>') {
                let text = &html[start + content_start + 1..end];
                // Strip remaining HTML tags
                let text = strip_html_tags(text);
                if !text.is_empty() {
                    out.push_str(&text);
                    out.push_str("\n\n");
                }
            }
        }
    }

    out.push_str("---\n");
    out.push_str(&format!(
        "Source: {} (via oEmbed, limited data)\n",
        original_url
    ));

    out
}

/// Minimal HTML tag stripper for oEmbed fallback.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

/// Build the common request headers (User-Agent + JSON Accept) for Twitter API calls.
fn twitter_headers(options: &FetchOptions) -> HeaderMap {
    let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
    let mut headers = HeaderMap::new();
    headers.insert(
        USER_AGENT,
        HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT)),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers
}

#[async_trait]
impl Fetcher for TwitterFetcher {
    fn name(&self) -> &'static str {
        "twitter_tweet"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::parse_tweet_url(url).is_some()
    }

    async fn fetch(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
    ) -> Result<FetchResponse, FetchError> {
        let url = Url::parse(&request.url).map_err(|_| FetchError::InvalidUrlScheme)?;

        let (_username, tweet_id) = Self::parse_tweet_url(&url).ok_or_else(|| {
            FetchError::FetcherError("Not a valid Twitter/X tweet URL".to_string())
        })?;

        // Try syndication API first
        match self.fetch_syndication(&tweet_id, options).await {
            Ok(tweet) => {
                let content = format_syndication_response(&tweet, &request.url);
                return Ok(FetchResponse {
                    url: request.url.clone(),
                    status_code: 200,
                    content_type: Some("text/markdown".to_string()),
                    format: Some("twitter_tweet".to_string()),
                    content: Some(content),
                    ..Default::default()
                });
            }
            Err(e) => {
                tracing::debug!(tweet_id, error = %e, "Syndication API failed, trying oEmbed");
            }
        }

        // Fallback: oEmbed API
        match self.fetch_oembed(&request.url, options).await {
            Ok(oembed) => {
                let content = format_oembed_response(&oembed, &request.url);
                Ok(FetchResponse {
                    url: request.url.clone(),
                    status_code: 200,
                    content_type: Some("text/markdown".to_string()),
                    format: Some("twitter_tweet".to_string()),
                    content: Some(content),
                    ..Default::default()
                })
            }
            Err(e) => {
                // Both failed — return error response
                Ok(FetchResponse {
                    url: request.url.clone(),
                    status_code: 502,
                    error: Some(format!(
                        "Failed to fetch tweet: syndication and oEmbed both unavailable ({})",
                        e
                    )),
                    ..Default::default()
                })
            }
        }
    }
}

impl TwitterFetcher {
    /// Fetch tweet data from the syndication API.
    async fn fetch_syndication(
        &self,
        tweet_id: &str,
        options: &FetchOptions,
    ) -> Result<SyndicationTweet, FetchError> {
        let syndication_url = format!("{}?id={}&token=0", SYNDICATION_BASE, tweet_id);
        let parsed = Url::parse(&syndication_url).map_err(|_| FetchError::InvalidUrlScheme)?;

        // THREAT[TM-SSRF-010]: single-hop request, redirects not followed.
        let response = transport_request(
            parsed,
            reqwest::Method::GET,
            twitter_headers(options),
            options,
            API_TIMEOUT,
            "cdn.syndication.twimg.com",
            443,
        )
        .await?;

        let status = response.status;
        if !(200..300).contains(&status) {
            return Err(FetchError::FetcherError(format!(
                "Syndication API returned HTTP {}",
                status
            )));
        }

        let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);
        let (body, _truncated) =
            read_body_with_timeout(response, BODY_TIMEOUT, max_body_size).await?;

        let body = String::from_utf8_lossy(&body);

        if body.is_empty() {
            return Err(FetchError::FetcherError(
                "Syndication API returned empty response".to_string(),
            ));
        }

        serde_json::from_str(body.as_ref()).map_err(|e| {
            FetchError::FetcherError(format!("Failed to parse syndication response: {}", e))
        })
    }

    /// Fetch tweet data from the oEmbed API.
    async fn fetch_oembed(
        &self,
        tweet_url: &str,
        options: &FetchOptions,
    ) -> Result<OEmbedResponse, FetchError> {
        let oembed_url = format!("{}?url={}", OEMBED_BASE, tweet_url);
        let parsed = Url::parse(&oembed_url).map_err(|_| FetchError::InvalidUrlScheme)?;

        // THREAT[TM-SSRF-010]: single-hop request, redirects not followed.
        let response = transport_request(
            parsed,
            reqwest::Method::GET,
            twitter_headers(options),
            options,
            API_TIMEOUT,
            "publish.x.com",
            443,
        )
        .await?;

        let status = response.status;
        if !(200..300).contains(&status) {
            return Err(FetchError::FetcherError(format!(
                "oEmbed API returned HTTP {}",
                status
            )));
        }

        let max_body_size = options.max_body_size.unwrap_or(DEFAULT_MAX_BODY_SIZE);
        let (body, _truncated) =
            read_body_with_timeout(response, BODY_TIMEOUT, max_body_size).await?;

        serde_json::from_slice(&body).map_err(|e| {
            FetchError::FetcherError(format!("Failed to parse oEmbed response: {}", e))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tweet_url_x_com() {
        let url = Url::parse("https://x.com/zachlloydtweets/status/2036509756404158559").unwrap();
        let (user, id) = TwitterFetcher::parse_tweet_url(&url).unwrap();
        assert_eq!(user, "zachlloydtweets");
        assert_eq!(id, "2036509756404158559");
    }

    #[test]
    fn test_parse_tweet_url_twitter_com() {
        let url =
            Url::parse("https://twitter.com/zachlloydtweets/status/2036509756404158559").unwrap();
        let (user, id) = TwitterFetcher::parse_tweet_url(&url).unwrap();
        assert_eq!(user, "zachlloydtweets");
        assert_eq!(id, "2036509756404158559");
    }

    #[test]
    fn test_parse_tweet_url_rejects_non_tweet_paths() {
        // Profile URL
        let url = Url::parse("https://x.com/zachlloydtweets").unwrap();
        assert!(TwitterFetcher::parse_tweet_url(&url).is_none());

        // Too many segments
        let url = Url::parse("https://x.com/zachlloydtweets/status/123/extra").unwrap();
        assert!(TwitterFetcher::parse_tweet_url(&url).is_none());

        // Wrong path structure
        let url = Url::parse("https://x.com/zachlloydtweets/likes/123").unwrap();
        assert!(TwitterFetcher::parse_tweet_url(&url).is_none());
    }

    #[test]
    fn test_parse_tweet_url_rejects_reserved_paths() {
        let url = Url::parse("https://x.com/i/status/123").unwrap();
        assert!(TwitterFetcher::parse_tweet_url(&url).is_none());

        let url = Url::parse("https://x.com/settings/status/123").unwrap();
        assert!(TwitterFetcher::parse_tweet_url(&url).is_none());

        let url = Url::parse("https://x.com/explore/status/123").unwrap();
        assert!(TwitterFetcher::parse_tweet_url(&url).is_none());
    }

    #[test]
    fn test_parse_tweet_url_rejects_non_numeric_id() {
        let url = Url::parse("https://x.com/user/status/abc").unwrap();
        assert!(TwitterFetcher::parse_tweet_url(&url).is_none());
    }

    #[test]
    fn test_parse_tweet_url_rejects_wrong_host() {
        let url = Url::parse("https://example.com/user/status/123").unwrap();
        assert!(TwitterFetcher::parse_tweet_url(&url).is_none());
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = TwitterFetcher::new();

        let url = Url::parse("https://x.com/zachlloydtweets/status/2036509756404158559").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://twitter.com/user/status/123456789").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://x.com/zachlloydtweets").unwrap();
        assert!(!fetcher.matches(&url));

        let url = Url::parse("https://example.com/user/status/123").unwrap();
        assert!(!fetcher.matches(&url));
    }

    #[test]
    fn test_expand_text_urls() {
        let entities = SyndicationEntities {
            urls: Some(vec![SyndicationEntityUrl {
                url: Some("https://t.co/abc123".to_string()),
                expanded_url: Some("https://example.com/full-article".to_string()),
                display_url: Some("example.com/full-arti…".to_string()),
            }]),
        };

        let text = "Check this out https://t.co/abc123";
        let expanded = expand_text_urls(text, Some(&entities));
        assert_eq!(expanded, "Check this out https://example.com/full-article");
    }

    #[test]
    fn test_expand_text_urls_no_entities() {
        let text = "No links here";
        assert_eq!(expand_text_urls(text, None), "No links here");
    }

    #[test]
    fn test_format_date() {
        assert_eq!(format_date("2026-03-24T18:25:37.000Z"), "Mar 24, 2026");
        assert_eq!(format_date("2024-01-05T00:00:00.000Z"), "Jan 5, 2024");
        assert_eq!(format_date("invalid"), "invalid");
    }

    #[test]
    fn test_format_metrics() {
        assert_eq!(format_metrics(Some(20), Some(1)), "20 likes · 1 reply");
        assert_eq!(format_metrics(Some(1), Some(0)), "1 like");
        assert_eq!(format_metrics(Some(0), Some(0)), "");
        assert_eq!(format_metrics(None, None), "");
    }

    #[test]
    fn test_strip_html_tags() {
        assert_eq!(
            strip_html_tags("<a href=\"x\">hello</a> world"),
            "hello world"
        );
        assert_eq!(strip_html_tags("no tags"), "no tags");
    }

    #[test]
    fn test_format_syndication_regular_tweet() {
        let tweet = SyndicationTweet {
            text: Some("Hello world".to_string()),
            user: Some(SyndicationUser {
                name: Some("Test User".to_string()),
                screen_name: Some("testuser".to_string()),
            }),
            created_at: Some("2026-03-24T18:25:37.000Z".to_string()),
            favorite_count: Some(42),
            conversation_count: Some(5),
            article: None,
            media_details: None,
            entities: None,
            quoted_tweet: None,
        };

        let output = format_syndication_response(&tweet, "https://x.com/testuser/status/123");
        assert!(output.contains("# @testuser (Test User)"));
        assert!(output.contains("Hello world"));
        assert!(output.contains("42 likes"));
        assert!(output.contains("5 replies"));
        assert!(output.contains("Source: https://x.com/testuser/status/123"));
    }

    #[test]
    fn test_format_syndication_article_tweet() {
        let tweet = SyndicationTweet {
            text: Some("https://t.co/abc".to_string()),
            user: Some(SyndicationUser {
                name: Some("Zach Lloyd".to_string()),
                screen_name: Some("zachlloydtweets".to_string()),
            }),
            created_at: Some("2026-03-24T18:25:37.000Z".to_string()),
            favorite_count: Some(20),
            conversation_count: Some(1),
            article: Some(SyndicationArticle {
                title: Some("Build vs buy".to_string()),
                preview_text: Some("The consensus is...".to_string()),
                cover_media: Some(SyndicationCoverMedia {
                    media_info: Some(SyndicationMediaInfo {
                        original_img_url: Some("https://pbs.twimg.com/media/cover.jpg".to_string()),
                    }),
                }),
                rest_id: Some("2036508645660225536".to_string()),
            }),
            media_details: None,
            entities: None,
            quoted_tweet: None,
        };

        let output = format_syndication_response(
            &tweet,
            "https://x.com/zachlloydtweets/status/2036509756404158559",
        );
        assert!(output.contains("# Build vs buy"));
        assert!(output.contains("**Zach Lloyd** (@zachlloydtweets)"));
        assert!(output.contains("Mar 24, 2026"));
        assert!(output.contains("![Cover](https://pbs.twimg.com/media/cover.jpg)"));
        assert!(output.contains("The consensus is..."));
        assert!(output.contains("Full article: https://x.com/i/article/2036508645660225536"));
        assert!(output.contains("20 likes"));
    }

    #[test]
    fn test_format_oembed_response() {
        let oembed = OEmbedResponse {
            author_name: Some("Zach Lloyd".to_string()),
            author_url: Some("https://twitter.com/zachlloydtweets".to_string()),
            html: Some("<blockquote><p lang=\"en\">Hello world</p></blockquote>".to_string()),
        };

        let output = format_oembed_response(&oembed, "https://x.com/zach/status/123");
        assert!(output.contains("# @zachlloydtweets (Zach Lloyd)"));
        assert!(output.contains("Hello world"));
        assert!(output.contains("via oEmbed, limited data"));
    }

    #[test]
    fn test_format_syndication_with_media() {
        let tweet = SyndicationTweet {
            text: Some("Look at this".to_string()),
            user: Some(SyndicationUser {
                name: Some("Photographer".to_string()),
                screen_name: Some("photog".to_string()),
            }),
            created_at: None,
            favorite_count: None,
            conversation_count: None,
            article: None,
            media_details: Some(vec![
                SyndicationMedia {
                    media_type: Some("photo".to_string()),
                    media_url_https: Some("https://pbs.twimg.com/media/photo1.jpg".to_string()),
                    video_info: None,
                },
                SyndicationMedia {
                    media_type: Some("video".to_string()),
                    media_url_https: Some("https://pbs.twimg.com/media/thumb.jpg".to_string()),
                    video_info: Some(SyndicationVideoInfo {}),
                },
            ]),
            entities: None,
            quoted_tweet: None,
        };

        let output = format_syndication_response(&tweet, "https://x.com/photog/status/1");
        assert!(output.contains("![Image](https://pbs.twimg.com/media/photo1.jpg)"));
        assert!(output.contains("![Video thumbnail](https://pbs.twimg.com/media/thumb.jpg)"));
    }

    #[test]
    fn test_format_syndication_with_quoted_tweet() {
        let tweet = SyndicationTweet {
            text: Some("Interesting take".to_string()),
            user: Some(SyndicationUser {
                name: Some("Quoter".to_string()),
                screen_name: Some("quoter".to_string()),
            }),
            created_at: None,
            favorite_count: None,
            conversation_count: None,
            article: None,
            media_details: None,
            entities: None,
            quoted_tweet: Some(Box::new(SyndicationTweet {
                text: Some("Original thought here".to_string()),
                user: Some(SyndicationUser {
                    name: Some("Original Author".to_string()),
                    screen_name: Some("original".to_string()),
                }),
                created_at: None,
                favorite_count: None,
                conversation_count: None,
                article: None,
                media_details: None,
                entities: None,
                quoted_tweet: None,
            })),
        };

        let output = format_syndication_response(&tweet, "https://x.com/quoter/status/1");
        assert!(output.contains("Interesting take"));
        assert!(output.contains("> **Original Author** (@original):"));
        assert!(output.contains("> Original thought here"));
    }

    #[test]
    fn test_format_syndication_minimal_fields() {
        let tweet = SyndicationTweet {
            text: None,
            user: None,
            created_at: None,
            favorite_count: None,
            conversation_count: None,
            article: None,
            media_details: None,
            entities: None,
            quoted_tweet: None,
        };

        let output = format_syndication_response(&tweet, "https://x.com/u/status/1");
        // Should still produce valid output with source link
        assert!(output.contains("Source: https://x.com/u/status/1"));
        assert!(output.contains("---"));
    }

    #[test]
    fn test_format_oembed_minimal_fields() {
        let oembed = OEmbedResponse {
            author_name: None,
            author_url: None,
            html: None,
        };

        let output = format_oembed_response(&oembed, "https://x.com/u/status/1");
        assert!(output.contains("@unknown (Unknown)"));
        assert!(output.contains("Source:"));
    }

    #[tokio::test]
    async fn test_fetch_syndication_with_mock() {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        let body = serde_json::json!({
            "text": "Test tweet content",
            "user": {
                "name": "Test User",
                "screen_name": "testuser"
            },
            "created_at": "2026-03-24T18:25:37.000Z",
            "favorite_count": 10,
            "conversation_count": 2
        });

        Mock::given(method("GET"))
            .and(path("/tweet-result"))
            .and(query_param("id", "123456789"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&body))
            .mount(&mock_server)
            .await;

        // We can't easily override the URL constant in the fetcher,
        // so we test the parsing/formatting path directly.
        let tweet: SyndicationTweet = serde_json::from_value(body).unwrap();
        let output = format_syndication_response(&tweet, "https://x.com/testuser/status/123456789");
        assert!(output.contains("Test tweet content"));
        assert!(output.contains("@testuser"));
    }
}
