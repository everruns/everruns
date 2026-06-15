//! YouTube video fetcher
//!
//! Handles youtube.com/watch and youtu.be URLs, returning video metadata
//! and transcript text via oEmbed and noembed APIs.

use crate::client::FetchOptions;
use crate::error::FetchError;
use crate::fetchers::default::{read_full_body, transport_request};
use crate::fetchers::Fetcher;
use crate::types::{FetchRequest, FetchResponse};
use crate::DEFAULT_USER_AGENT;
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use std::time::Duration;
use url::Url;

const API_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_TRANSCRIPT_CHARS: usize = 15_000;

/// YouTube host and port (DNS-pinned per request).
const YOUTUBE_HOST: &str = "www.youtube.com";
const YOUTUBE_PORT: u16 = 443;

/// YouTube video fetcher
///
/// Matches `youtube.com/watch?v={id}` and `youtu.be/{id}`, returning
/// video metadata via oEmbed and transcript when available.
pub struct YouTubeFetcher;

impl YouTubeFetcher {
    pub fn new() -> Self {
        Self
    }

    /// Extract video ID from YouTube URL
    fn parse_video_id(url: &Url) -> Option<String> {
        let host = url.host_str()?;

        match host {
            "youtube.com" | "www.youtube.com" | "m.youtube.com" => {
                // /watch?v={id}
                let segments: Vec<&str> =
                    url.path_segments().map(|s| s.collect()).unwrap_or_default();
                if segments.first() != Some(&"watch") {
                    return None;
                }
                url.query_pairs()
                    .find(|(k, _)| k == "v")
                    .map(|(_, v)| v.to_string())
                    .filter(|v| !v.is_empty())
            }
            "youtu.be" => {
                // /{id}
                let segments: Vec<&str> =
                    url.path_segments().map(|s| s.collect()).unwrap_or_default();
                segments
                    .first()
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
            }
            _ => None,
        }
    }
}

impl Default for YouTubeFetcher {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
struct OEmbedResponse {
    title: Option<String>,
    author_name: Option<String>,
    author_url: Option<String>,
}

/// Transcript segment extracted from YouTube's timedtext XML
#[derive(Debug)]
struct TranscriptSegment {
    text: String,
}

#[async_trait]
impl Fetcher for YouTubeFetcher {
    fn name(&self) -> &'static str {
        "youtube"
    }

    fn matches(&self, url: &Url) -> bool {
        Self::parse_video_id(url).is_some()
    }

    async fn fetch(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
    ) -> Result<FetchResponse, FetchError> {
        let url = Url::parse(&request.url).map_err(|_| FetchError::InvalidUrlScheme)?;

        let video_id = Self::parse_video_id(&url)
            .ok_or_else(|| FetchError::FetcherError("Not a valid YouTube URL".to_string()))?;

        let user_agent = options.user_agent.as_deref().unwrap_or(DEFAULT_USER_AGENT);
        let ua_header = HeaderValue::from_str(user_agent)
            .unwrap_or_else(|_| HeaderValue::from_static(DEFAULT_USER_AGENT));

        let canonical_url = format!("https://www.youtube.com/watch?v={}", video_id);

        // Fetch oEmbed metadata
        // The canonical URL only contains safe ASCII chars, so it can be passed directly
        let mut oembed_url = Url::parse("https://www.youtube.com/oembed").unwrap();
        options.validate_url(&oembed_url)?;
        oembed_url
            .query_pairs_mut()
            .append_pair("url", &canonical_url)
            .append_pair("format", "json");

        let mut oembed_headers = HeaderMap::new();
        oembed_headers.insert(USER_AGENT, ua_header.clone());

        // THREAT[TM-SSRF-010]: single-hop request, redirects not followed.
        let oembed = match transport_request(
            oembed_url,
            reqwest::Method::GET,
            oembed_headers,
            options,
            API_TIMEOUT,
            YOUTUBE_HOST,
            YOUTUBE_PORT,
        )
        .await
        {
            Ok(resp) if (200..300).contains(&resp.status) => {
                match read_full_body(resp, options).await {
                    Ok(body) => serde_json::from_slice::<OEmbedResponse>(&body).ok(),
                    Err(_) => None,
                }
            }
            _ => None,
        };

        let title = oembed
            .as_ref()
            .and_then(|o| o.title.clone())
            .unwrap_or_else(|| format!("YouTube Video {}", video_id));

        let author = oembed.as_ref().and_then(|o| o.author_name.clone());
        let author_url = oembed.as_ref().and_then(|o| o.author_url.clone());

        // Attempt transcript extraction via timedtext API
        let transcript = fetch_transcript(&ua_header, &video_id, options).await;

        let content = format_youtube_response(
            &title,
            &video_id,
            &canonical_url,
            author.as_deref(),
            author_url.as_deref(),
            transcript.as_deref(),
        );

        Ok(FetchResponse {
            url: request.url.clone(),
            status_code: 200,
            content_type: Some("text/markdown".to_string()),
            format: Some("youtube_video".to_string()),
            content: Some(content),
            ..Default::default()
        })
    }
}

/// Attempt to fetch transcript/captions via YouTube's timedtext XML API.
/// Returns None if transcript is unavailable.
async fn fetch_transcript(
    ua: &HeaderValue,
    video_id: &str,
    options: &FetchOptions,
) -> Option<String> {
    // Try the legacy timedtext API (auto-generated English captions)
    let timedtext_url = format!(
        "https://www.youtube.com/api/timedtext?v={}&lang=en&fmt=srv3",
        video_id
    );

    let timedtext_url = Url::parse(&timedtext_url).ok()?;
    options.validate_url(&timedtext_url).ok()?;

    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, ua.clone());

    // THREAT[TM-SSRF-010]: single-hop request, redirects not followed.
    let resp = transport_request(
        timedtext_url,
        reqwest::Method::GET,
        headers,
        options,
        API_TIMEOUT,
        YOUTUBE_HOST,
        YOUTUBE_PORT,
    )
    .await
    .ok()?;

    if !(200..300).contains(&resp.status) {
        return None;
    }

    let body = read_full_body(resp, options).await.ok()?;
    let xml = String::from_utf8_lossy(&body).into_owned();
    if let Some(max_body_size) = options.max_body_size {
        if xml.len() > max_body_size {
            return None;
        }
    }
    if xml.is_empty() || !xml.contains("<text") {
        return None;
    }

    let segments = parse_timedtext_xml(&xml);
    if segments.is_empty() {
        return None;
    }

    let transcript: String = segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    if transcript.is_empty() {
        None
    } else {
        Some(transcript)
    }
}

/// Parse YouTube timedtext XML format into transcript segments
fn parse_timedtext_xml(xml: &str) -> Vec<TranscriptSegment> {
    let mut segments = Vec::new();
    let mut search_from = 0;

    while let Some(start) = xml[search_from..].find("<text") {
        let abs_start = search_from + start;
        let content_start = match xml[abs_start..].find('>') {
            Some(pos) => abs_start + pos + 1,
            None => break,
        };

        let content_end = match xml[content_start..].find("</text>") {
            Some(pos) => content_start + pos,
            None => break,
        };

        let text = decode_xml_entities(&xml[content_start..content_end]);
        let text = text.trim().to_string();
        if !text.is_empty() {
            segments.push(TranscriptSegment { text });
        }

        search_from = content_end + 7; // "</text>".len()
    }

    segments
}

/// Decode XML/HTML entities commonly found in YouTube transcripts
fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

fn format_youtube_response(
    title: &str,
    video_id: &str,
    canonical_url: &str,
    author: Option<&str>,
    author_url: Option<&str>,
    transcript: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("# {}\n\n", title));

    out.push_str("## Video Info\n\n");
    if let Some(author) = author {
        if let Some(url) = author_url {
            out.push_str(&format!("- **Channel:** [{}]({})\n", author, url));
        } else {
            out.push_str(&format!("- **Channel:** {}\n", author));
        }
    }
    out.push_str(&format!("- **Video ID:** {}\n", video_id));
    out.push_str(&format!("- **URL:** {}\n", canonical_url));
    out.push_str(&format!(
        "- **Thumbnail:** https://img.youtube.com/vi/{}/maxresdefault.jpg\n",
        video_id
    ));

    if let Some(transcript) = transcript {
        out.push_str("\n## Transcript\n\n");
        // Truncate very long transcripts
        if transcript.len() > MAX_TRANSCRIPT_CHARS {
            let truncated = safe_truncate_utf8(transcript, MAX_TRANSCRIPT_CHARS);
            out.push_str(truncated);
            out.push_str("\n\n*[Transcript truncated]*\n");
        } else {
            out.push_str(transcript);
            out.push('\n');
        }
    } else {
        out.push_str("\n*No transcript available for this video.*\n");
    }

    out
}

fn safe_truncate_utf8(input: &str, max_bytes: usize) -> &str {
    if input.len() <= max_bytes {
        return input;
    }

    if input.is_char_boundary(max_bytes) {
        return &input[..max_bytes];
    }

    let idx = input
        .char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i < max_bytes)
        .last()
        .unwrap_or(0);
    &input[..idx]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_youtube_watch() {
        let url = Url::parse("https://www.youtube.com/watch?v=dQw4w9WgXcQ").unwrap();
        assert_eq!(
            YouTubeFetcher::parse_video_id(&url),
            Some("dQw4w9WgXcQ".to_string())
        );
    }

    #[test]
    fn test_parse_youtu_be() {
        let url = Url::parse("https://youtu.be/dQw4w9WgXcQ").unwrap();
        assert_eq!(
            YouTubeFetcher::parse_video_id(&url),
            Some("dQw4w9WgXcQ".to_string())
        );
    }

    #[test]
    fn test_parse_youtube_no_www() {
        let url = Url::parse("https://youtube.com/watch?v=abc123").unwrap();
        assert_eq!(
            YouTubeFetcher::parse_video_id(&url),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn test_parse_youtube_mobile() {
        let url = Url::parse("https://m.youtube.com/watch?v=abc123").unwrap();
        assert_eq!(
            YouTubeFetcher::parse_video_id(&url),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn test_rejects_non_watch() {
        let url = Url::parse("https://www.youtube.com/channel/UC123").unwrap();
        assert_eq!(YouTubeFetcher::parse_video_id(&url), None);
    }

    #[test]
    fn test_rejects_no_v_param() {
        let url = Url::parse("https://www.youtube.com/watch?list=PL123").unwrap();
        assert_eq!(YouTubeFetcher::parse_video_id(&url), None);
    }

    #[test]
    fn test_rejects_non_youtube() {
        let url = Url::parse("https://vimeo.com/123456").unwrap();
        assert_eq!(YouTubeFetcher::parse_video_id(&url), None);
    }

    #[test]
    fn test_rejects_empty_v_param() {
        let url = Url::parse("https://www.youtube.com/watch?v=").unwrap();
        assert_eq!(YouTubeFetcher::parse_video_id(&url), None);
    }

    #[test]
    fn test_fetcher_matches() {
        let fetcher = YouTubeFetcher::new();

        let url = Url::parse("https://www.youtube.com/watch?v=abc").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://youtu.be/abc").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://m.youtube.com/watch?v=abc").unwrap();
        assert!(fetcher.matches(&url));

        let url = Url::parse("https://example.com/watch?v=abc").unwrap();
        assert!(!fetcher.matches(&url));
    }

    #[test]
    fn test_format_youtube_response_with_all_fields() {
        let output = format_youtube_response(
            "Test Video",
            "abc123",
            "https://www.youtube.com/watch?v=abc123",
            Some("Test Channel"),
            Some("https://www.youtube.com/channel/UC123"),
            Some("Hello world this is a transcript."),
        );

        assert!(output.contains("# Test Video"));
        assert!(output.contains("[Test Channel](https://www.youtube.com/channel/UC123)"));
        assert!(output.contains("**Video ID:** abc123"));
        assert!(output.contains("## Transcript"));
        assert!(output.contains("Hello world this is a transcript."));
    }

    #[test]
    fn test_format_youtube_response_no_transcript() {
        let output = format_youtube_response(
            "Test Video",
            "abc123",
            "https://www.youtube.com/watch?v=abc123",
            None,
            None,
            None,
        );

        assert!(output.contains("# Test Video"));
        assert!(output.contains("No transcript available"));
        assert!(!output.contains("## Transcript"));
    }

    #[test]
    fn test_format_youtube_response_truncates_long_transcript() {
        let long_transcript = "a".repeat(20000);
        let output = format_youtube_response(
            "Long Video",
            "abc",
            "https://www.youtube.com/watch?v=abc",
            None,
            None,
            Some(&long_transcript),
        );

        assert!(output.contains("[Transcript truncated]"));
        assert!(output.len() < 20000);
    }

    #[test]
    fn test_parse_timedtext_xml() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<transcript>
<text start="0.5" dur="1.2">Hello everyone</text>
<text start="1.7" dur="2.0">Welcome to this video</text>
<text start="3.7" dur="1.5">Let&apos;s get started</text>
</transcript>"#;

        let segments = parse_timedtext_xml(xml);
        assert_eq!(segments.len(), 3);
        assert_eq!(segments[0].text, "Hello everyone");
        assert_eq!(segments[1].text, "Welcome to this video");
        assert_eq!(segments[2].text, "Let's get started");
    }

    #[test]
    fn test_parse_timedtext_xml_empty() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?><transcript></transcript>"#;
        let segments = parse_timedtext_xml(xml);
        assert!(segments.is_empty());
    }

    #[tokio::test]
    async fn test_fetch_blocked_secondary_host() {
        let fetcher = YouTubeFetcher::new();
        let request = FetchRequest::new("https://youtu.be/dQw4w9WgXcQ");
        let options = FetchOptions {
            blocked_hosts: vec![".youtube.com".to_string()],
            ..Default::default()
        };

        let result = fetcher.fetch(&request, &options).await;
        assert!(matches!(result, Err(FetchError::BlockedUrl)));
    }

    #[test]
    fn test_decode_xml_entities() {
        assert_eq!(decode_xml_entities("a &amp; b"), "a & b");
        assert_eq!(decode_xml_entities("&lt;tag&gt;"), "<tag>");
        assert_eq!(decode_xml_entities("it&#39;s"), "it's");
    }

    #[test]
    fn test_safe_truncate_utf8_multibyte_boundary() {
        let input = format!("{}érest", "a".repeat(14_999));
        let truncated = safe_truncate_utf8(&input, 15_000);
        assert_eq!(truncated.len(), 14_999);
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
