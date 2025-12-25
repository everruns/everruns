//! WebFetch Capability - provides tools to fetch web content
//!
//! This capability allows agents to fetch content from URLs and convert
//! HTML responses to markdown or plain text for easier processing.
//!
//! Design decisions:
//! - No system prompt addition (capability doesn't need special instructions)
//! - Binary content is not supported and returns an error
//! - Accept headers are set based on the response format requested

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde_json::Value;

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

        // Create HTTP client
        let client = match reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::error!("Failed to create HTTP client: {}", e);
                return ToolExecutionResult::tool_error("Failed to create HTTP client");
            }
        };

        // Execute request
        let request = match method {
            HttpMethod::Get => client.get(url),
            HttpMethod::Head => client.head(url),
        };

        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("HTTP request failed for {}: {}", url, e);
                if e.is_timeout() {
                    return ToolExecutionResult::tool_error("Request timed out");
                } else if e.is_connect() {
                    return ToolExecutionResult::tool_error("Failed to connect to server");
                } else {
                    return ToolExecutionResult::tool_error(format!("Request failed: {}", e));
                }
            }
        };

        let status = response.status();
        let status_code = status.as_u16();

        // Get content type
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Check for binary content
        if let Some(ref ct) = content_type {
            if is_binary_content_type(ct) {
                return ToolExecutionResult::tool_error(
                    "Binary content is not supported. Only textual content (HTML, text, JSON, etc.) can be fetched.",
                );
            }
        }

        // For HEAD requests, return headers info only
        if method == HttpMethod::Head {
            return ToolExecutionResult::success(serde_json::json!({
                "url": url,
                "status_code": status_code,
                "content_type": content_type,
                "method": "HEAD"
            }));
        }

        // Get response body
        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => {
                tracing::error!("Failed to read response body from {}: {}", url, e);
                return ToolExecutionResult::tool_error("Failed to read response body");
            }
        };

        // Check if response is HTML (for conversion)
        let is_html = content_type
            .as_ref()
            .map(|ct| ct.contains("text/html") || ct.contains("application/xhtml"))
            .unwrap_or(false)
            || body.trim_start().starts_with("<!DOCTYPE")
            || body.trim_start().starts_with("<html");

        // Convert content based on format
        let content = match response_format {
            ResponseFormat::Markdown if is_html => html_to_markdown(&body),
            ResponseFormat::Text if is_html => html_to_text(&body),
            _ => body,
        };

        let format = match response_format {
            ResponseFormat::Markdown if is_html => "markdown",
            ResponseFormat::Text if is_html => "text",
            _ => "raw",
        };

        ToolExecutionResult::success(serde_json::json!({
            "url": url,
            "status_code": status_code,
            "content_type": content_type,
            "format": format,
            "content": content
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

    // Note: Integration tests that actually make HTTP requests should be in
    // a separate integration test file or marked with #[ignore]
    #[tokio::test]
    #[ignore = "requires network access"]
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
    #[ignore = "requires network access"]
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
}
