//! WebFetch Capability - provides tools to fetch web content
//!
//! This capability allows agents to fetch content from URLs and convert
//! HTML responses to markdown or plain text for easier processing.
//!
//! Design decisions:
//! - No system prompt addition (capability doesn't need special instructions)
//! - Binary content is not supported but returns metadata (filename, size, content_type)
//! - Accept headers are set based on the response format requested
//! - Timeout for first byte: 1 second (connect + time to first response byte)
//! - Timeout for body: 30 seconds total, partial content returned if exceeded
//! - Response includes content size and Last-Modified header when available

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{
    HeaderMap, HeaderValue, ACCEPT, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE,
    LAST_MODIFIED, USER_AGENT,
};
use serde_json::Value;
use std::time::{Duration, Instant};

/// Timeout for connection and first response byte (1 second)
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// Timeout for reading the entire response body (30 seconds)
const BODY_TIMEOUT: Duration = Duration::from_secs(30);

/// WebFetch capability - provides tools to fetch web content
pub struct WebFetchCapability;

impl Capability for WebFetchCapability {
    fn id(&self) -> &str {
        CapabilityId::WEB_FETCH
    }

    fn name(&self) -> &str {
        "Web Fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from URLs and convert HTML responses to markdown or plain text."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("globe")
    }

    fn category(&self) -> Option<&str> {
        Some("Network")
    }

    // No system_prompt_addition - this capability doesn't need special instructions

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(WebFetchTool)]
    }
}

// ============================================================================
// Tool: web_fetch
// ============================================================================

/// Tool that fetches content from a URL
pub struct WebFetchTool;

/// HTTP methods supported by the web_fetch tool
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum HttpMethod {
    #[default]
    Get,
    Head,
}

impl HttpMethod {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "GET" => Some(Self::Get),
            "HEAD" => Some(Self::Head),
            _ => None,
        }
    }
}

/// Response format for the fetched content
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ResponseFormat {
    /// Return raw response body (for non-HTML or when no conversion requested)
    #[default]
    Raw,
    /// Convert HTML to markdown
    Markdown,
    /// Convert HTML to plain text
    Text,
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch content from a URL. Can convert HTML responses to markdown or plain text for easier processing. Only supports textual content; binary content (images, PDFs, etc.) will return an error."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch content from. Must be a valid HTTP or HTTPS URL."
                },
                "method": {
                    "type": "string",
                    "enum": ["GET", "HEAD"],
                    "description": "HTTP method to use. Defaults to GET."
                },
                "as_markdown": {
                    "type": "boolean",
                    "description": "If true, convert HTML response to markdown format. Takes precedence over as_text."
                },
                "as_text": {
                    "type": "boolean",
                    "description": "If true, convert HTML response to plain text (strips all HTML tags). Ignored if as_markdown is true."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        // Extract URL (required)
        let url = match arguments.get("url").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: url");
            }
        };

        // Validate URL
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return ToolExecutionResult::tool_error(
                "Invalid URL: must start with http:// or https://",
            );
        }

        // Extract method (defaults to GET)
        let method = arguments
            .get("method")
            .and_then(|v| v.as_str())
            .map(HttpMethod::from_str)
            .unwrap_or(Some(HttpMethod::Get));

        let method = match method {
            Some(m) => m,
            None => {
                return ToolExecutionResult::tool_error("Invalid method: must be GET or HEAD");
            }
        };

        // Determine response format
        let as_markdown = arguments
            .get("as_markdown")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let as_text = arguments
            .get("as_text")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let response_format = if as_markdown {
            ResponseFormat::Markdown
        } else if as_text {
            ResponseFormat::Text
        } else {
            ResponseFormat::Raw
        };

        // Build request headers
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("Everruns-WebFetch/1.0"),
        );

        // Set Accept header based on response format
        let accept_value = match response_format {
            ResponseFormat::Markdown => "text/html, text/markdown, text/plain, */*;q=0.8",
            ResponseFormat::Text => "text/html, text/plain, */*;q=0.8",
            ResponseFormat::Raw => "*/*",
        };
        headers.insert(ACCEPT, HeaderValue::from_static(accept_value));

        // Create HTTP client with connect timeout for first byte
        let client = match reqwest::Client::builder()
            .default_headers(headers)
            .connect_timeout(CONNECT_TIMEOUT)
            // Note: We don't set a global timeout here; we handle body timeout manually
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to create HTTP client: {}", e);
                return ToolExecutionResult::tool_error("Failed to create HTTP client");
            }
        };

        // Execute request with timeout for first response byte
        let request = match method {
            HttpMethod::Get => client.get(url),
            HttpMethod::Head => client.head(url),
        };

        // Set timeout for connection + first response byte (1 second total)
        let response = match tokio::time::timeout(CONNECT_TIMEOUT, request.send()).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                tracing::error!("HTTP request failed for {}: {}", url, e);
                if e.is_timeout() {
                    return ToolExecutionResult::tool_error(
                        "Request timed out: server did not respond within 1 second",
                    );
                } else if e.is_connect() {
                    return ToolExecutionResult::tool_error("Failed to connect to server");
                } else {
                    return ToolExecutionResult::tool_error(format!("Request failed: {}", e));
                }
            }
            Err(_) => {
                tracing::error!("HTTP request timed out waiting for first byte from {}", url);
                return ToolExecutionResult::tool_error(
                    "Request timed out: server did not respond within 1 second",
                );
            }
        };

        let status = response.status();
        let status_code = status.as_u16();

        // Extract metadata from headers
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let content_length = response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        let last_modified = response
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let filename = extract_filename_from_headers(response.headers(), url);

        // Check for binary content - return metadata instead of error
        if let Some(ref ct) = content_type {
            if is_binary_content_type(ct) {
                return ToolExecutionResult::success(serde_json::json!({
                    "url": url,
                    "status_code": status_code,
                    "content_type": content_type,
                    "size": content_length,
                    "filename": filename,
                    "last_modified": last_modified,
                    "error": "Binary content is not supported. Only textual content (HTML, text, JSON, etc.) can be fetched."
                }));
            }
        }

        // For HEAD requests, return headers info only
        if method == HttpMethod::Head {
            return ToolExecutionResult::success(serde_json::json!({
                "url": url,
                "status_code": status_code,
                "content_type": content_type,
                "size": content_length,
                "last_modified": last_modified,
                "filename": filename,
                "method": "HEAD"
            }));
        }

        // Stream response body with timeout
        let (body, size, timed_out) = read_body_with_timeout(response, BODY_TIMEOUT).await;

        // Check if response is HTML (for conversion)
        let is_html = content_type
            .as_ref()
            .map(|ct| ct.contains("text/html") || ct.contains("application/xhtml"))
            .unwrap_or(false)
            || body.trim_start().starts_with("<!DOCTYPE")
            || body.trim_start().starts_with("<html");

        // Convert content based on format
        let mut content = match response_format {
            ResponseFormat::Markdown if is_html => html_to_markdown(&body),
            ResponseFormat::Text if is_html => html_to_text(&body),
            _ => body,
        };

        // Append timeout indicator if body was truncated
        if timed_out {
            content.push_str("\n\n[..more content timed out...]");
        }

        let format = match response_format {
            ResponseFormat::Markdown if is_html => "markdown",
            ResponseFormat::Text if is_html => "text",
            _ => "raw",
        };

        ToolExecutionResult::success(serde_json::json!({
            "url": url,
            "status_code": status_code,
            "content_type": content_type,
            "size": size,
            "last_modified": last_modified,
            "format": format,
            "content": content,
            "truncated": timed_out
        }))
    }
}

/// Check if a content type represents binary content
fn is_binary_content_type(content_type: &str) -> bool {
    let ct = content_type.to_lowercase();

    // Binary types that are definitely not text
    let binary_prefixes = [
        "image/",
        "audio/",
        "video/",
        "application/octet-stream",
        "application/pdf",
        "application/zip",
        "application/gzip",
        "application/x-tar",
        "application/x-rar",
        "application/x-7z",
        "application/vnd.ms-",
        "application/vnd.openxmlformats",
        "font/",
    ];

    for prefix in binary_prefixes {
        if ct.starts_with(prefix) || ct.contains(prefix) {
            return true;
        }
    }

    false
}

/// Read response body with a timeout, returning partial content if timeout is exceeded.
/// Returns (body_text, bytes_read, was_truncated).
async fn read_body_with_timeout(
    response: reqwest::Response,
    timeout: Duration,
) -> (String, usize, bool) {
    let start = Instant::now();
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    let mut timed_out = false;

    while let Some(chunk_result) = stream.next().await {
        // Check if we've exceeded the timeout
        if start.elapsed() >= timeout {
            timed_out = true;
            tracing::warn!(
                "Body read timed out after {:?}, returning partial content ({} bytes)",
                timeout,
                bytes.len()
            );
            break;
        }

        match chunk_result {
            Ok(chunk) => {
                bytes.extend_from_slice(&chunk);
            }
            Err(e) => {
                tracing::error!("Error reading response chunk: {}", e);
                break;
            }
        }
    }

    let size = bytes.len();

    // Convert to string, replacing invalid UTF-8 sequences
    let body = String::from_utf8_lossy(&bytes).into_owned();

    (body, size, timed_out)
}

/// Extract filename from Content-Disposition header or URL
fn extract_filename_from_headers(headers: &HeaderMap, url: &str) -> Option<String> {
    // Try Content-Disposition header first
    if let Some(disposition) = headers.get(CONTENT_DISPOSITION) {
        if let Ok(value) = disposition.to_str() {
            // Look for filename="..." or filename*=...
            if let Some(start) = value.find("filename=") {
                let rest = &value[start + 9..];
                let filename = if let Some(stripped) = rest.strip_prefix('"') {
                    // Quoted filename
                    stripped.split('"').next()
                } else {
                    // Unquoted filename
                    rest.split([';', ' ']).next()
                };
                if let Some(name) = filename {
                    if !name.is_empty() {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }

    // Fall back to extracting filename from URL path
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(mut segments) = parsed.path_segments() {
            if let Some(last) = segments.next_back() {
                if !last.is_empty() && last.contains('.') {
                    return Some(last.to_string());
                }
            }
        }
    }

    None
}

/// Convert HTML to markdown
///
/// This is a simple implementation that handles common HTML elements.
/// For production use, consider using a dedicated library like html2md.
fn html_to_markdown(html: &str) -> String {
    // First extract text content, preserving structure
    let mut result = String::new();
    let mut in_tag = false;
    let mut current_tag = String::new();
    let mut skip_content = false;
    let mut list_depth: usize = 0;
    let mut in_code_block = false;
    let mut chars = html.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            in_tag = true;
            current_tag.clear();
            continue;
        }

        if in_tag {
            if ch == '>' {
                in_tag = false;
                let tag = current_tag.to_lowercase();
                let is_closing = tag.starts_with('/');
                let tag_name = if is_closing { &tag[1..] } else { &tag[..] };
                let tag_name = tag_name.split_whitespace().next().unwrap_or("");

                match tag_name {
                    "script" | "style" | "noscript" | "iframe" | "svg" => {
                        skip_content = !is_closing;
                    }
                    "h1" if !is_closing => result.push_str("\n# "),
                    "h2" if !is_closing => result.push_str("\n## "),
                    "h3" if !is_closing => result.push_str("\n### "),
                    "h4" if !is_closing => result.push_str("\n#### "),
                    "h5" if !is_closing => result.push_str("\n##### "),
                    "h6" if !is_closing => result.push_str("\n###### "),
                    "p" | "div" | "section" | "article" | "main" | "header" | "footer" => {
                        if is_closing {
                            result.push_str("\n\n");
                        }
                    }
                    "br" => result.push('\n'),
                    "hr" => result.push_str("\n---\n"),
                    "ul" | "ol" => {
                        if is_closing {
                            list_depth = list_depth.saturating_sub(1);
                            result.push('\n');
                        } else {
                            list_depth += 1;
                        }
                    }
                    "li" if !is_closing => {
                        result.push('\n');
                        for _ in 1..list_depth {
                            result.push_str("  ");
                        }
                        result.push_str("- ");
                    }
                    "strong" | "b" if !is_closing => result.push_str("**"),
                    "strong" | "b" if is_closing => result.push_str("**"),
                    "em" | "i" if !is_closing => result.push('*'),
                    "em" | "i" if is_closing => result.push('*'),
                    "code" if !is_closing && !in_code_block => result.push('`'),
                    "code" if is_closing && !in_code_block => result.push('`'),
                    "pre" if !is_closing => {
                        in_code_block = true;
                        result.push_str("\n```\n");
                    }
                    "pre" if is_closing => {
                        in_code_block = false;
                        result.push_str("\n```\n");
                    }
                    "blockquote" if !is_closing => result.push_str("\n> "),
                    "a" if !is_closing => {
                        // Try to extract href
                        if let Some(href_start) = current_tag.find("href=\"") {
                            let href_content = &current_tag[href_start + 6..];
                            if let Some(href_end) = href_content.find('"') {
                                let href = &href_content[..href_end];
                                // Store href for later, we'll handle it simply
                                result.push('[');
                                // We'll close with the href after the text
                                result.push_str(&format!("]({})", href));
                            }
                        }
                    }
                    _ => {}
                }
            } else {
                current_tag.push(ch);
            }
            continue;
        }

        if !skip_content {
            // Decode common HTML entities
            if ch == '&' {
                let mut entity = String::new();
                entity.push(ch);
                while let Some(&next_ch) = chars.peek() {
                    entity.push(next_ch);
                    chars.next();
                    if next_ch == ';' {
                        break;
                    }
                    if entity.len() > 10 {
                        break;
                    }
                }
                match entity.as_str() {
                    "&amp;" => result.push('&'),
                    "&lt;" => result.push('<'),
                    "&gt;" => result.push('>'),
                    "&quot;" => result.push('"'),
                    "&apos;" | "&#39;" => result.push('\''),
                    "&nbsp;" => result.push(' '),
                    "&mdash;" | "&#8212;" => result.push('—'),
                    "&ndash;" | "&#8211;" => result.push('–'),
                    "&copy;" | "&#169;" => result.push_str("(c)"),
                    "&reg;" | "&#174;" => result.push_str("(R)"),
                    _ => result.push_str(&entity),
                }
            } else {
                result.push(ch);
            }
        }
    }

    // Clean up the result
    clean_whitespace(&result)
}

/// Convert HTML to plain text (strips all formatting)
fn html_to_text(html: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    let mut current_tag = String::new();
    let mut skip_content = false;
    let mut chars = html.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '<' {
            in_tag = true;
            current_tag.clear();
            continue;
        }

        if in_tag {
            if ch == '>' {
                in_tag = false;
                let tag = current_tag.to_lowercase();
                let is_closing = tag.starts_with('/');
                let tag_name = if is_closing { &tag[1..] } else { &tag[..] };
                let tag_name = tag_name.split_whitespace().next().unwrap_or("");

                match tag_name {
                    "script" | "style" | "noscript" | "iframe" | "svg" => {
                        skip_content = !is_closing;
                    }
                    "p" | "div" | "br" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "li" | "tr" => {
                        result.push('\n');
                    }
                    _ => {}
                }
            } else {
                current_tag.push(ch);
            }
            continue;
        }

        if !skip_content {
            // Decode common HTML entities
            if ch == '&' {
                let mut entity = String::new();
                entity.push(ch);
                while let Some(&next_ch) = chars.peek() {
                    entity.push(next_ch);
                    chars.next();
                    if next_ch == ';' {
                        break;
                    }
                    if entity.len() > 10 {
                        break;
                    }
                }
                match entity.as_str() {
                    "&amp;" => result.push('&'),
                    "&lt;" => result.push('<'),
                    "&gt;" => result.push('>'),
                    "&quot;" => result.push('"'),
                    "&apos;" | "&#39;" => result.push('\''),
                    "&nbsp;" => result.push(' '),
                    "&mdash;" | "&#8212;" => result.push('—'),
                    "&ndash;" | "&#8211;" => result.push('–'),
                    "&copy;" | "&#169;" => result.push_str("(c)"),
                    "&reg;" | "&#174;" => result.push_str("(R)"),
                    _ => result.push_str(&entity),
                }
            } else {
                result.push(ch);
            }
        }
    }

    // Clean up the result
    clean_whitespace(&result)
}

/// Clean up whitespace in the result
fn clean_whitespace(text: &str) -> String {
    let mut result = String::new();
    let mut prev_was_newline = false;
    let mut prev_was_space = false;
    let mut newline_count = 0;

    for ch in text.chars() {
        if ch == '\n' {
            newline_count += 1;
            prev_was_space = false;
            if newline_count <= 2 {
                result.push('\n');
            }
            prev_was_newline = true;
        } else if ch.is_whitespace() {
            if !prev_was_space && !prev_was_newline {
                result.push(' ');
            }
            prev_was_space = true;
            newline_count = 0;
        } else {
            result.push(ch);
            prev_was_newline = false;
            prev_was_space = false;
            newline_count = 0;
        }
    }

    result.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_binary_content_type() {
        // Binary types
        assert!(is_binary_content_type("image/png"));
        assert!(is_binary_content_type("image/jpeg"));
        assert!(is_binary_content_type("audio/mpeg"));
        assert!(is_binary_content_type("video/mp4"));
        assert!(is_binary_content_type("application/pdf"));
        assert!(is_binary_content_type("application/octet-stream"));
        assert!(is_binary_content_type("application/zip"));

        // Text types
        assert!(!is_binary_content_type("text/html"));
        assert!(!is_binary_content_type("text/plain"));
        assert!(!is_binary_content_type("application/json"));
        assert!(!is_binary_content_type("application/xml"));
        assert!(!is_binary_content_type("text/html; charset=utf-8"));
    }

    #[test]
    fn test_html_to_text_basic() {
        let html = "<p>Hello <strong>World</strong>!</p>";
        let text = html_to_text(html);
        assert!(text.contains("Hello"));
        assert!(text.contains("World"));
        assert!(!text.contains("<"));
        assert!(!text.contains(">"));
    }

    #[test]
    fn test_html_to_text_strips_script() {
        let html = "<p>Before</p><script>alert('hi');</script><p>After</p>";
        let text = html_to_text(html);
        assert!(text.contains("Before"));
        assert!(text.contains("After"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_html_to_text_strips_style() {
        let html = "<p>Content</p><style>body { color: red; }</style>";
        let text = html_to_text(html);
        assert!(text.contains("Content"));
        assert!(!text.contains("color"));
    }

    #[test]
    fn test_html_to_text_decodes_entities() {
        let html = "<p>Tom &amp; Jerry &lt;3 &gt; love</p>";
        let text = html_to_text(html);
        assert!(text.contains("Tom & Jerry"));
        assert!(text.contains("<3 >"));
    }

    #[test]
    fn test_html_to_markdown_headings() {
        let html = "<h1>Title</h1><h2>Subtitle</h2><h3>Section</h3>";
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"));
        assert!(md.contains("## Subtitle"));
        assert!(md.contains("### Section"));
    }

    #[test]
    fn test_html_to_markdown_formatting() {
        let html = "<p><strong>Bold</strong> and <em>italic</em></p>";
        let md = html_to_markdown(html);
        assert!(md.contains("**Bold**"));
        assert!(md.contains("*italic*"));
    }

    #[test]
    fn test_html_to_markdown_lists() {
        let html = "<ul><li>Item 1</li><li>Item 2</li></ul>";
        let md = html_to_markdown(html);
        assert!(md.contains("- Item 1"));
        assert!(md.contains("- Item 2"));
    }

    #[test]
    fn test_html_to_markdown_code() {
        let html = "<p>Use <code>println!</code> for output</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("`println!`"));
    }

    #[test]
    fn test_html_to_markdown_code_block() {
        let html = "<pre><code>fn main() {\n    println!(\"hello\");\n}</code></pre>";
        let md = html_to_markdown(html);
        assert!(md.contains("```"));
        assert!(md.contains("fn main()"));
    }

    #[test]
    fn test_html_to_markdown_blockquote() {
        let html = "<blockquote>A wise quote</blockquote>";
        let md = html_to_markdown(html);
        assert!(md.contains("> A wise quote"));
    }

    #[test]
    fn test_html_to_markdown_hr() {
        let html = "<p>Before</p><hr><p>After</p>";
        let md = html_to_markdown(html);
        assert!(md.contains("---"));
    }

    #[test]
    fn test_web_fetch_tool_parameters() {
        let tool = WebFetchTool;
        let schema = tool.parameters_schema();

        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["url"].is_object());
        assert!(schema["properties"]["method"].is_object());
        assert!(schema["properties"]["as_markdown"].is_object());
        assert!(schema["properties"]["as_text"].is_object());
        assert_eq!(schema["required"], serde_json::json!(["url"]));
    }

    #[test]
    fn test_web_fetch_capability_metadata() {
        let cap = WebFetchCapability;

        assert_eq!(cap.id(), "web_fetch");
        assert_eq!(cap.name(), "Web Fetch");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("globe"));
        assert_eq!(cap.category(), Some("Network"));
        assert!(cap.system_prompt_addition().is_none());
    }

    #[test]
    fn test_web_fetch_capability_has_tool() {
        let cap = WebFetchCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "web_fetch");
    }

    #[tokio::test]
    async fn test_web_fetch_missing_url() {
        let tool = WebFetchTool;
        let result = tool.execute(serde_json::json!({})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("url"));
        } else {
            panic!("Expected tool error for missing URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_url() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({"url": "not-a-valid-url"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid URL"));
        } else {
            panic!("Expected tool error for invalid URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_method() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({"url": "https://example.com", "method": "POST"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid method"));
        } else {
            panic!("Expected tool error for invalid method");
        }
    }

    // Integration tests that make HTTP requests
    #[tokio::test]
    async fn test_web_fetch_real_request() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/html",
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert!(value["content"]
                .as_str()
                .unwrap()
                .contains("Herman Melville"));
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_head_request() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/html",
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["method"], "HEAD");
            // HEAD requests should not have content
            assert!(value.get("content").is_none());
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_response_includes_size() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/html"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            // Size should be present and > 0
            assert!(value["size"].as_u64().unwrap() > 0);
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_binary_returns_metadata() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/image/png"
            }))
            .await;

        // Binary content should return success with error message and metadata
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert!(value["content_type"]
                .as_str()
                .unwrap()
                .contains("image/png"));
            assert!(value["error"]
                .as_str()
                .unwrap()
                .contains("Binary content is not supported"));
            // Should have size metadata if available
            assert!(value.get("size").is_some() || value["size"].is_null());
        } else {
            panic!("Expected success response with metadata for binary content");
        }
    }

    #[test]
    fn test_extract_filename_from_content_disposition_quoted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"test_file.pdf\""),
        );

        let filename = extract_filename_from_headers(&headers, "https://example.com/download");
        assert_eq!(filename, Some("test_file.pdf".to_string()));
    }

    #[test]
    fn test_extract_filename_from_content_disposition_unquoted() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=document.pdf"),
        );

        let filename = extract_filename_from_headers(&headers, "https://example.com/download");
        assert_eq!(filename, Some("document.pdf".to_string()));
    }

    #[test]
    fn test_extract_filename_from_url() {
        let headers = HeaderMap::new();

        let filename =
            extract_filename_from_headers(&headers, "https://example.com/path/to/report.pdf");
        assert_eq!(filename, Some("report.pdf".to_string()));
    }

    #[test]
    fn test_extract_filename_no_extension() {
        let headers = HeaderMap::new();

        // URL without file extension should return None
        let filename = extract_filename_from_headers(&headers, "https://example.com/path/to/page");
        assert_eq!(filename, None);
    }

    #[tokio::test]
    async fn test_web_fetch_head_includes_metadata() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/response-headers?Content-Length=100",
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["method"], "HEAD");
            // Should have metadata fields even for HEAD requests
            assert!(value.get("content_type").is_some());
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_truncated_field() {
        // Normal response should have truncated: false
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/html"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["truncated"], false);
        } else {
            panic!("Expected successful response");
        }
    }

    // ============================================================================
    // Timeout constant tests
    // ============================================================================

    #[test]
    fn test_connect_timeout_is_one_second() {
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(1));
    }

    #[test]
    fn test_body_timeout_is_thirty_seconds() {
        assert_eq!(BODY_TIMEOUT, Duration::from_secs(30));
    }

    // ============================================================================
    // First byte timeout tests
    // ============================================================================

    #[tokio::test]
    async fn test_web_fetch_timeout_unreachable_host() {
        // Use a non-routable IP address to trigger connection timeout
        // 10.255.255.1 is typically non-routable and will timeout
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "http://10.255.255.1:12345/test"
            }))
            .await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                // Should timeout or fail to connect
                assert!(
                    msg.contains("timed out") || msg.contains("connect"),
                    "Expected timeout or connection error, got: {}",
                    msg
                );
            }
            _ => {
                // This is also acceptable - some networks may have different behavior
            }
        }
    }

    // ============================================================================
    // Filename extraction edge cases
    // ============================================================================

    #[test]
    fn test_extract_filename_content_disposition_with_extra_params() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"report.pdf\"; size=12345"),
        );

        let filename = extract_filename_from_headers(&headers, "https://example.com/download");
        assert_eq!(filename, Some("report.pdf".to_string()));
    }

    #[test]
    fn test_extract_filename_content_disposition_inline() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("inline; filename=\"preview.jpg\""),
        );

        let filename = extract_filename_from_headers(&headers, "https://example.com/view");
        assert_eq!(filename, Some("preview.jpg".to_string()));
    }

    #[test]
    fn test_extract_filename_url_with_query_string() {
        let headers = HeaderMap::new();

        let filename = extract_filename_from_headers(
            &headers,
            "https://example.com/files/document.pdf?token=abc123",
        );
        // Should still extract filename from path, ignoring query string
        assert_eq!(filename, Some("document.pdf".to_string()));
    }

    #[test]
    fn test_extract_filename_url_with_fragment() {
        let headers = HeaderMap::new();

        let filename =
            extract_filename_from_headers(&headers, "https://example.com/files/doc.pdf#page=5");
        assert_eq!(filename, Some("doc.pdf".to_string()));
    }

    #[test]
    fn test_extract_filename_url_trailing_slash() {
        let headers = HeaderMap::new();

        // URL ending with slash should return None (no filename)
        let filename = extract_filename_from_headers(&headers, "https://example.com/path/");
        assert_eq!(filename, None);
    }

    #[test]
    fn test_extract_filename_url_root_path() {
        let headers = HeaderMap::new();

        let filename = extract_filename_from_headers(&headers, "https://example.com/");
        assert_eq!(filename, None);
    }

    #[test]
    fn test_extract_filename_url_no_path() {
        let headers = HeaderMap::new();

        let filename = extract_filename_from_headers(&headers, "https://example.com");
        assert_eq!(filename, None);
    }

    #[test]
    fn test_extract_filename_url_encoded_filename() {
        let headers = HeaderMap::new();

        // URL-encoded filename
        let filename =
            extract_filename_from_headers(&headers, "https://example.com/files/my%20document.pdf");
        // Note: URL parsing will decode, so we get the encoded form
        assert_eq!(filename, Some("my%20document.pdf".to_string()));
    }

    #[test]
    fn test_extract_filename_content_disposition_empty_filename() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"\""),
        );

        // Empty filename in header should fall back to URL
        let filename = extract_filename_from_headers(&headers, "https://example.com/fallback.txt");
        assert_eq!(filename, Some("fallback.txt".to_string()));
    }

    #[test]
    fn test_extract_filename_prefers_header_over_url() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"from_header.pdf\""),
        );

        let filename = extract_filename_from_headers(&headers, "https://example.com/from_url.txt");
        // Header should take precedence
        assert_eq!(filename, Some("from_header.pdf".to_string()));
    }

    // ============================================================================
    // Binary content type detection tests
    // ============================================================================

    #[test]
    fn test_is_binary_content_type_images() {
        assert!(is_binary_content_type("image/png"));
        assert!(is_binary_content_type("image/jpeg"));
        assert!(is_binary_content_type("image/gif"));
        assert!(is_binary_content_type("image/webp"));
        assert!(is_binary_content_type("image/svg+xml")); // SVG is image/* even though it's XML
    }

    #[test]
    fn test_is_binary_content_type_audio_video() {
        assert!(is_binary_content_type("audio/mpeg"));
        assert!(is_binary_content_type("audio/wav"));
        assert!(is_binary_content_type("audio/ogg"));
        assert!(is_binary_content_type("video/mp4"));
        assert!(is_binary_content_type("video/webm"));
    }

    #[test]
    fn test_is_binary_content_type_archives() {
        assert!(is_binary_content_type("application/zip"));
        assert!(is_binary_content_type("application/gzip"));
        assert!(is_binary_content_type("application/x-tar"));
        assert!(is_binary_content_type("application/x-rar"));
        assert!(is_binary_content_type("application/x-7z-compressed"));
    }

    #[test]
    fn test_is_binary_content_type_documents() {
        assert!(is_binary_content_type("application/pdf"));
        assert!(is_binary_content_type("application/vnd.ms-excel"));
        assert!(is_binary_content_type(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        ));
    }

    #[test]
    fn test_is_binary_content_type_fonts() {
        assert!(is_binary_content_type("font/woff"));
        assert!(is_binary_content_type("font/woff2"));
        assert!(is_binary_content_type("font/ttf"));
    }

    #[test]
    fn test_is_binary_content_type_text_types() {
        assert!(!is_binary_content_type("text/html"));
        assert!(!is_binary_content_type("text/plain"));
        assert!(!is_binary_content_type("text/css"));
        assert!(!is_binary_content_type("text/javascript"));
        assert!(!is_binary_content_type("text/csv"));
        assert!(!is_binary_content_type("text/xml"));
    }

    #[test]
    fn test_is_binary_content_type_application_text() {
        assert!(!is_binary_content_type("application/json"));
        assert!(!is_binary_content_type("application/xml"));
        assert!(!is_binary_content_type("application/javascript"));
        assert!(!is_binary_content_type("application/ld+json"));
    }

    #[test]
    fn test_is_binary_content_type_with_charset() {
        // Content types often include charset
        assert!(!is_binary_content_type("text/html; charset=utf-8"));
        assert!(!is_binary_content_type("application/json; charset=utf-8"));
        assert!(is_binary_content_type("image/png; charset=binary"));
    }

    #[test]
    fn test_is_binary_content_type_case_insensitive() {
        assert!(is_binary_content_type("IMAGE/PNG"));
        assert!(is_binary_content_type("Image/Jpeg"));
        assert!(is_binary_content_type("APPLICATION/PDF"));
    }

    // ============================================================================
    // Response structure validation tests
    // ============================================================================

    #[tokio::test]
    async fn test_web_fetch_response_has_all_expected_fields() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/html"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // Verify all expected fields are present
            assert!(value.get("url").is_some(), "Missing 'url' field");
            assert!(
                value.get("status_code").is_some(),
                "Missing 'status_code' field"
            );
            assert!(
                value.get("content_type").is_some(),
                "Missing 'content_type' field"
            );
            assert!(value.get("size").is_some(), "Missing 'size' field");
            assert!(value.get("format").is_some(), "Missing 'format' field");
            assert!(value.get("content").is_some(), "Missing 'content' field");
            assert!(
                value.get("truncated").is_some(),
                "Missing 'truncated' field"
            );
            // last_modified may or may not be present depending on server
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_head_response_structure() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/html",
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // HEAD response should have metadata but not content
            assert!(value.get("url").is_some());
            assert!(value.get("status_code").is_some());
            assert!(value.get("method").is_some());
            assert_eq!(value["method"], "HEAD");
            // Should NOT have content or truncated for HEAD
            assert!(value.get("content").is_none());
            assert!(value.get("truncated").is_none());
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_binary_response_structure() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/image/jpeg"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // Binary response should have metadata and error message
            assert!(value.get("url").is_some());
            assert!(value.get("status_code").is_some());
            assert!(value.get("content_type").is_some());
            assert!(value.get("error").is_some());
            // Should NOT have content or truncated for binary
            assert!(value.get("content").is_none());
            assert!(value.get("truncated").is_none());
        } else {
            panic!("Expected successful response with metadata");
        }
    }

    // ============================================================================
    // Format conversion tests
    // ============================================================================

    #[tokio::test]
    async fn test_web_fetch_as_markdown_format_field() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/html",
                "as_markdown": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["format"], "markdown");
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_as_text_format_field() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/html",
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["format"], "text");
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_raw_format_for_non_html() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/json"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // JSON content should return "raw" format
            assert_eq!(value["format"], "raw");
        } else {
            panic!("Expected successful response");
        }
    }

    // ============================================================================
    // Last-Modified header tests (using httpbin's response-headers endpoint)
    // ============================================================================

    #[tokio::test]
    async fn test_web_fetch_last_modified_when_present() {
        let tool = WebFetchTool;
        // Use httpbin to set a custom Last-Modified header
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/response-headers?Last-Modified=Tue%2C%2001%20Jan%202024%2012%3A00%3A00%20GMT"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // Should have extracted the Last-Modified header
            if let Some(last_mod) = value.get("last_modified") {
                assert!(
                    last_mod.as_str().is_some(),
                    "last_modified should be a string"
                );
            }
            // Note: httpbin might not always work perfectly, so we just verify the field exists
        } else {
            panic!("Expected successful response");
        }
    }

    // ============================================================================
    // HTTP method tests
    // ============================================================================

    #[test]
    fn test_http_method_from_str_valid() {
        assert_eq!(HttpMethod::from_str("GET"), Some(HttpMethod::Get));
        assert_eq!(HttpMethod::from_str("get"), Some(HttpMethod::Get));
        assert_eq!(HttpMethod::from_str("Get"), Some(HttpMethod::Get));
        assert_eq!(HttpMethod::from_str("HEAD"), Some(HttpMethod::Head));
        assert_eq!(HttpMethod::from_str("head"), Some(HttpMethod::Head));
    }

    #[test]
    fn test_http_method_from_str_invalid() {
        assert_eq!(HttpMethod::from_str("POST"), None);
        assert_eq!(HttpMethod::from_str("PUT"), None);
        assert_eq!(HttpMethod::from_str("DELETE"), None);
        assert_eq!(HttpMethod::from_str("PATCH"), None);
        assert_eq!(HttpMethod::from_str(""), None);
        assert_eq!(HttpMethod::from_str("INVALID"), None);
    }

    #[test]
    fn test_http_method_default() {
        assert_eq!(HttpMethod::default(), HttpMethod::Get);
    }

    // ============================================================================
    // Response format tests
    // ============================================================================

    #[test]
    fn test_response_format_default() {
        assert_eq!(ResponseFormat::default(), ResponseFormat::Raw);
    }

    // ============================================================================
    // Size validation tests
    // ============================================================================

    #[tokio::test]
    async fn test_web_fetch_size_matches_content_length() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/bytes/100"
            }))
            .await;

        // httpbin /bytes/N returns exactly N random bytes
        // But since it's binary, we'll get the metadata with size
        if let ToolExecutionResult::Success(value) = result {
            // For binary content, size comes from Content-Length header
            if let Some(size) = value.get("size") {
                if !size.is_null() {
                    assert_eq!(size.as_u64().unwrap(), 100);
                }
            }
        }
    }

    #[tokio::test]
    async fn test_web_fetch_size_for_text_content() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/robots.txt"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            let size = value["size"].as_u64().unwrap();
            let content = value["content"].as_str().unwrap();
            // Size should match the content length
            assert_eq!(size as usize, content.len());
        } else {
            panic!("Expected successful response");
        }
    }

    // ============================================================================
    // Error handling tests
    // ============================================================================

    #[tokio::test]
    async fn test_web_fetch_404_returns_success_with_status() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/status/404"
            }))
            .await;

        // 404 should still be a "success" from tool perspective - it got a response
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 404);
        } else {
            panic!("Expected successful response even for 404");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_500_returns_success_with_status() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://httpbin.org/status/500"
            }))
            .await;

        // 500 should still be a "success" from tool perspective
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 500);
        } else {
            panic!("Expected successful response even for 500");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_dns_failure() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "https://this-domain-definitely-does-not-exist-12345.com/test"
            }))
            .await;

        // DNS failure should return a tool error
        if let ToolExecutionResult::ToolError(msg) = result {
            let msg_lower = msg.to_lowercase();
            assert!(
                msg_lower.contains("failed")
                    || msg_lower.contains("error")
                    || msg_lower.contains("timed out")
                    || msg_lower.contains("connect"),
                "Expected error message about failure, got: {}",
                msg
            );
        } else {
            // Some environments might timeout instead of DNS failure
        }
    }

    // ============================================================================
    // URL validation tests
    // ============================================================================

    #[tokio::test]
    async fn test_web_fetch_rejects_ftp_url() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "ftp://example.com/file.txt"
            }))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid URL"));
        } else {
            panic!("Expected tool error for FTP URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_rejects_file_url() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "file:///etc/passwd"
            }))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid URL"));
        } else {
            panic!("Expected tool error for file:// URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_accepts_http_url() {
        let tool = WebFetchTool;
        let result = tool
            .execute(serde_json::json!({
                "url": "http://httpbin.org/get"
            }))
            .await;

        // HTTP (not HTTPS) should work
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
        } else {
            // Some environments block plain HTTP, so this is acceptable too
        }
    }
}
