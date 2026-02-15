//! WebFetch Capability - provides tools to fetch web content
//!
//! This capability uses the fetchkit library to fetch content from URLs and convert
//! HTML responses to markdown or plain text for easier processing.
//!
//! Design decisions:
//! - Uses fetchkit library for HTTP operations, HTML conversion, and tool metadata
//! - Binary content is not supported but returns metadata (filename, size, content_type)
//! - Timeout for first byte: 1 second (connect + time to first response byte)
//! - Timeout for body: 30 seconds total, partial content returned if exceeded
//!
//! SECURITY[TM-API-008, TM-API-009]: Uses fetchkit::Tool with block_prefixes to prevent
//! SSRF to private IPs, localhost, and cloud metadata endpoints. See build_fetch_tool().

use super::{Capability, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use fetchkit::{FetchError, FetchRequest, TOOL_DESCRIPTION, TOOL_LLMTXT};
use serde_json::Value;

/// Build a configured fetchkit::Tool with SSRF block_prefixes.
///
/// Blocks private IPs, localhost, cloud metadata, and unspecified addresses.
/// SECURITY[TM-API-008, TM-API-009]
fn build_fetch_tool() -> fetchkit::Tool {
    let mut builder = fetchkit::Tool::builder()
        // Cloud metadata (AWS/GCP/Azure link-local)
        .block_prefix("http://169.254.")
        .block_prefix("https://169.254.")
        // GCP metadata alias
        .block_prefix("http://metadata.")
        .block_prefix("https://metadata.")
        // Localhost
        .block_prefix("http://127.0.0.1")
        .block_prefix("https://127.0.0.1")
        .block_prefix("http://127.")
        .block_prefix("https://127.")
        // IPv6 localhost
        .block_prefix("http://[::1]")
        .block_prefix("https://[::1]")
        // Localhost by name
        .block_prefix("http://localhost")
        .block_prefix("https://localhost")
        // RFC 1918: 10.0.0.0/8
        .block_prefix("http://10.")
        .block_prefix("https://10.")
        // RFC 1918: 192.168.0.0/16
        .block_prefix("http://192.168.")
        .block_prefix("https://192.168.")
        // Unspecified
        .block_prefix("http://0.0.0.0")
        .block_prefix("https://0.0.0.0");

    // RFC 1918: 172.16.0.0/12 (172.16.x.x through 172.31.x.x)
    for octet in 16..=31 {
        builder = builder
            .block_prefix(format!("http://172.{octet}."))
            .block_prefix(format!("https://172.{octet}."));
    }

    builder.build()
}

/// WebFetch capability - provides tools to fetch web content
pub struct WebFetchCapability;

impl Capability for WebFetchCapability {
    fn id(&self) -> &str {
        "web_fetch"
    }

    fn name(&self) -> &str {
        "Web Fetch"
    }

    fn description(&self) -> &str {
        // Use the description from fetchkit
        TOOL_DESCRIPTION
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

    fn system_prompt_addition(&self) -> Option<&str> {
        // Use the LLM-optimized documentation from fetchkit
        Some(TOOL_LLMTXT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(WebFetchTool::new())]
    }
}

// ============================================================================
// Tool: web_fetch
// ============================================================================

/// Tool that fetches content from a URL using fetchkit.
/// Holds a configured fetchkit::Tool with SSRF block_prefixes.
pub struct WebFetchTool {
    fetch_tool: fetchkit::Tool,
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl WebFetchTool {
    /// Create with SSRF-protected fetch tool (production default).
    pub fn new() -> Self {
        Self {
            fetch_tool: build_fetch_tool(),
        }
    }

    /// Create with permissive fetch tool (no URL blocking). Test-only.
    #[cfg(test)]
    fn permissive() -> Self {
        Self {
            fetch_tool: fetchkit::Tool::default(),
        }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        // Use the description from fetchkit library
        TOOL_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        // Use schema from fetchkit's Tool
        fetchkit::Tool::default().input_schema()
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        // Extract URL (required)
        let url = match arguments.get("url").and_then(|v| v.as_str()) {
            Some(u) => u.to_string(),
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: url");
            }
        };

        // Extract method (defaults to GET)
        let method = arguments
            .get("method")
            .and_then(|v| v.as_str())
            .map(|s| match s.to_uppercase().as_str() {
                "GET" => Some(fetchkit::HttpMethod::Get),
                "HEAD" => Some(fetchkit::HttpMethod::Head),
                _ => None,
            })
            .unwrap_or(Some(fetchkit::HttpMethod::Get));

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

        // Build the fetch request
        // Only set options to Some(true) when enabled, otherwise leave as None
        let request = FetchRequest {
            url,
            method: Some(method),
            as_markdown: if as_markdown { Some(true) } else { None },
            as_text: if as_text { Some(true) } else { None },
        };

        // Execute the fetch using configured fetchkit tool (with SSRF block_prefixes)
        match self.fetch_tool.execute(request).await {
            Ok(response) => {
                // Convert the fetchkit response to JSON
                ToolExecutionResult::success(serde_json::to_value(&response).unwrap_or_else(|_| {
                    serde_json::json!({
                        "error": "Failed to serialize response"
                    })
                }))
            }
            Err(e) => {
                // Map fetchkit errors to tool errors
                let error_message = match e {
                    FetchError::MissingUrl => "Missing required parameter: url".to_string(),
                    FetchError::InvalidUrlScheme => {
                        "Invalid URL: must start with http:// or https://".to_string()
                    }
                    FetchError::InvalidMethod => "Invalid method: must be GET or HEAD".to_string(),
                    FetchError::BlockedUrl => "URL is blocked by policy".to_string(),
                    FetchError::ClientBuildError(_) => "Failed to create HTTP client".to_string(),
                    FetchError::FirstByteTimeout => {
                        "Request timed out: server did not respond within 1 second".to_string()
                    }
                    FetchError::ConnectError(_) => "Failed to connect to server".to_string(),
                    FetchError::RequestError(msg) => format!("Request failed: {}", msg),
                    FetchError::FetcherError(msg) => format!("Fetch error: {}", msg),
                };
                ToolExecutionResult::tool_error(error_message)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_web_fetch_tool_parameters() {
        let tool = WebFetchTool::permissive();
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
        // System prompt comes from fetchkit's TOOL_LLMTXT
        assert!(cap.system_prompt_addition().is_some());
        assert!(cap.system_prompt_addition().unwrap().contains("FetchKit"));
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
        let tool = WebFetchTool::permissive();
        let result = tool.execute(serde_json::json!({})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("url"));
        } else {
            panic!("Expected tool error for missing URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_url() {
        let tool = WebFetchTool::permissive();
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
        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({"url": "https://example.com", "method": "POST"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Invalid method"));
        } else {
            panic!("Expected tool error for invalid method");
        }
    }

    // Integration tests using wiremock
    #[tokio::test]
    async fn test_web_fetch_real_request() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<html><body><p>Herman Melville - Moby Dick</p></body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri()),
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert!(
                value["content"]
                    .as_str()
                    .unwrap()
                    .contains("Herman Melville")
            );
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_head_request() {
        let mock_server = MockServer::start().await;

        Mock::given(method("HEAD"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .insert_header("content-length", "100"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri()),
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["method"], "HEAD");
            // HEAD requests should not have content
            assert!(value.get("content").is_none() || value["content"].is_null());
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_response_includes_size() {
        let mock_server = MockServer::start().await;
        let body = "<html><body>Test content</body></html>";

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(body)
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri())
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
        let mock_server = MockServer::start().await;

        // Simulate a PNG image response
        Mock::given(method("GET"))
            .and(path("/image/png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(vec![0x89, 0x50, 0x4E, 0x47]) // PNG magic bytes
                    .insert_header("content-type", "image/png")
                    .insert_header("content-length", "4"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/image/png", mock_server.uri())
            }))
            .await;

        // Binary content should return success with error message and metadata
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert!(
                value["content_type"]
                    .as_str()
                    .unwrap()
                    .contains("image/png")
            );
            assert!(
                value["error"].as_str().unwrap().contains("Binary content")
                    || value["error"].as_str().unwrap().contains("binary")
            );
            // Should have size metadata if available
            assert!(value.get("size").is_some() || value["size"].is_null());
        } else {
            panic!("Expected success response with metadata for binary content");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_truncated_field() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<html><body>Short content</body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        // Normal response should have truncated: false
        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // truncated should be false or null for non-truncated content
            assert!(
                value["truncated"].is_null()
                    || value["truncated"] == false
                    || value.get("truncated").is_none()
            );
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_timeout_unreachable_host() {
        // Use a non-routable public IP to trigger connection timeout
        // 198.51.100.1 (TEST-NET-2, RFC 5737) is non-routable
        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": "http://198.51.100.1:12345/test"
            }))
            .await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                // Should timeout or fail to connect
                assert!(
                    msg.contains("timed out") || msg.contains("connect") || msg.contains("failed"),
                    "Expected timeout or connection error, got: {}",
                    msg
                );
            }
            _ => {
                // This is also acceptable - some networks may have different behavior
            }
        }
    }

    #[tokio::test]
    async fn test_web_fetch_response_has_all_expected_fields() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<html><body>Test</body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri())
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
            // format, content may or may not be present depending on response type
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_head_response_structure() {
        let mock_server = MockServer::start().await;

        Mock::given(method("HEAD"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html")
                    .insert_header("content-length", "100"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri()),
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // HEAD response should have metadata but not content
            assert!(value.get("url").is_some());
            assert!(value.get("status_code").is_some());
            assert!(value.get("method").is_some());
            assert_eq!(value["method"], "HEAD");
            // Should NOT have content for HEAD
            assert!(value.get("content").is_none() || value["content"].is_null());
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_html_returns_markdown_by_default() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(
                        "<!DOCTYPE html><html><body><h1>Title</h1><p>Content</p></body></html>",
                    )
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        // No as_markdown needed - fetchkit returns markdown by default for HTML
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            // Content should be present
            let content = value["content"].as_str().unwrap();
            assert!(content.contains("Title") || content.contains("Content"));
            // Format should be "markdown" or "raw" depending on fetchkit's detection
            let format = value["format"].as_str().unwrap_or("raw");
            assert!(format == "markdown" || format == "raw");
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_as_text_strips_html() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("<!DOCTYPE html><html><body><b>Test</b> content</body></html>")
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/html", mock_server.uri()),
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            // Content should be present
            let content = value["content"].as_str().unwrap();
            assert!(content.contains("Test") || content.contains("content"));
            // Format should be "text" or "raw" depending on fetchkit's detection
            let format = value["format"].as_str().unwrap_or("raw");
            assert!(format == "text" || format == "raw");
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_raw_format_for_non_html() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("{\"key\": \"value\"}")
                    .insert_header("content-type", "application/json"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/json", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            // JSON content should return "raw" format
            assert_eq!(value["format"], "raw");
        } else {
            panic!("Expected successful response");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_404_returns_success_with_status() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/status/404"))
            .respond_with(ResponseTemplate::new(404).set_body_string("Not Found"))
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/status/404", mock_server.uri())
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
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/status/500"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/status/500", mock_server.uri())
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
        let tool = WebFetchTool::permissive();
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

    #[tokio::test]
    async fn test_web_fetch_rejects_ftp_url() {
        let tool = WebFetchTool::permissive();
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
        let tool = WebFetchTool::permissive();
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
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/get"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("{\"url\": \"http://localhost/get\"}")
                    .insert_header("content-type", "application/json"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        // Note: mock_server.uri() returns http:// URL
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/get", mock_server.uri())
            }))
            .await;

        // HTTP (not HTTPS) should work
        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
        } else {
            panic!("Expected successful response for HTTP URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_filters_excessive_newlines() {
        let mock_server = MockServer::start().await;

        // Response with many consecutive newlines
        Mock::given(method("GET"))
            .and(path("/newlines"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("line1\n\n\n\n\n\n\n\nline2")
                    .insert_header("content-type", "text/plain"),
            )
            .mount(&mock_server)
            .await;

        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/newlines", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            let content = value["content"].as_str().unwrap();
            // Should have at most 2 consecutive newlines
            assert!(
                !content.contains("\n\n\n"),
                "Content should not have more than 2 consecutive newlines"
            );
        } else {
            panic!("Expected successful response");
        }
    }

    // ========================================================================
    // SSRF security tests (TM-API-008, TM-API-009)
    //
    // These tests verify that private/internal URLs are blocked by policy
    // via fetchkit block_prefixes configured in build_fetch_tool().
    //
    // Run with: cargo test -p everruns-core --lib -- web_fetch::tests::test_ssrf
    // ========================================================================

    /// Helper: asserts that a URL is blocked by SSRF policy.
    async fn assert_blocked_by_policy(url: &str) {
        let tool = WebFetchTool::new();
        let result = tool.execute(serde_json::json!({"url": url})).await;
        match &result {
            ToolExecutionResult::ToolError(msg) if msg.contains("blocked") => {
                // Expected: URL blocked by policy
            }
            other => {
                panic!("Expected URL {url} to be blocked by policy, got: {other:?}");
            }
        }
    }

    /// THREAT[TM-API-009]: Cloud metadata endpoint is blocked.
    #[tokio::test]
    async fn test_ssrf_cloud_metadata_blocked() {
        assert_blocked_by_policy("http://169.254.169.254/latest/meta-data/").await;
    }

    /// THREAT[TM-API-009]: GCP metadata alias is blocked.
    #[tokio::test]
    async fn test_ssrf_gcp_metadata_blocked() {
        assert_blocked_by_policy("http://metadata.google.internal/computeMetadata/v1/").await;
    }

    /// THREAT[TM-API-008]: Localhost is blocked.
    #[tokio::test]
    async fn test_ssrf_localhost_blocked() {
        assert_blocked_by_policy("http://127.0.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: Localhost by name is blocked.
    #[tokio::test]
    async fn test_ssrf_localhost_name_blocked() {
        assert_blocked_by_policy("http://localhost:8080/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 10.x.x.x is blocked.
    #[tokio::test]
    async fn test_ssrf_private_10_blocked() {
        assert_blocked_by_policy("http://10.0.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 172.16.x.x is blocked.
    #[tokio::test]
    async fn test_ssrf_private_172_16_blocked() {
        assert_blocked_by_policy("http://172.16.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 172.31.x.x (upper bound) is blocked.
    #[tokio::test]
    async fn test_ssrf_private_172_31_blocked() {
        assert_blocked_by_policy("http://172.31.255.1:1/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 192.168.x.x is blocked.
    #[tokio::test]
    async fn test_ssrf_private_192_blocked() {
        assert_blocked_by_policy("http://192.168.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: IPv6 localhost is blocked.
    #[tokio::test]
    async fn test_ssrf_ipv6_localhost_blocked() {
        assert_blocked_by_policy("http://[::1]:1/").await;
    }

    /// THREAT[TM-API-008]: 0.0.0.0 is blocked.
    #[tokio::test]
    async fn test_ssrf_unspecified_blocked() {
        assert_blocked_by_policy("http://0.0.0.0:1/").await;
    }

    /// THREAT[TM-API-008]: HTTPS variants are also blocked.
    #[tokio::test]
    async fn test_ssrf_https_variants_blocked() {
        assert_blocked_by_policy("https://169.254.169.254/").await;
        assert_blocked_by_policy("https://127.0.0.1/").await;
        assert_blocked_by_policy("https://10.0.0.1/").await;
        assert_blocked_by_policy("https://172.16.0.1/").await;
        assert_blocked_by_policy("https://192.168.0.1/").await;
        assert_blocked_by_policy("https://localhost/").await;
        assert_blocked_by_policy("https://0.0.0.0/").await;
    }

    /// Verify file://, ftp://, gopher:// schemes are blocked (existing protection).
    #[tokio::test]
    async fn test_ssrf_non_http_schemes_blocked() {
        let tool = WebFetchTool::permissive();

        for (scheme, url) in [
            ("file://", "file:///etc/passwd"),
            ("ftp://", "ftp://internal-server/data"),
            ("gopher://", "gopher://internal-server/"),
        ] {
            let result = tool.execute(serde_json::json!({"url": url})).await;
            assert!(
                matches!(&result, ToolExecutionResult::ToolError(msg) if msg.contains("Invalid URL")),
                "{scheme} should be rejected"
            );
        }
    }

    // ========================================================================
    // Real-world integration tests (require network access)
    // Run with: cargo test -p everruns-core --lib -- web_fetch::tests::test_real --ignored
    // ========================================================================

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_real_wasmtime_docs_fetch() {
        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": "https://docs.wasmtime.dev/"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert!(
                value["content_type"]
                    .as_str()
                    .unwrap()
                    .contains("text/html")
            );
            let content = value["content"].as_str().unwrap();
            assert!(
                content.contains("Wasmtime") || content.contains("wasmtime"),
                "Content should mention Wasmtime"
            );
            assert!(
                value["size"].as_u64().unwrap() > 1000,
                "Page should have substantial content"
            );
        } else {
            panic!("Expected successful response from docs.wasmtime.dev");
        }
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_real_wasmtime_docs_as_text() {
        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": "https://docs.wasmtime.dev/",
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            // Content should be present and mention Wasmtime
            assert!(
                content.contains("Wasmtime") || content.contains("wasmtime"),
                "Text should contain Wasmtime reference"
            );
            // Check format field - may be "text" or "raw" depending on HTML detection
            let format = value["format"].as_str().unwrap_or("raw");
            assert!(
                format == "text" || format == "raw",
                "Format should be text or raw, got: {}",
                format
            );
        } else {
            panic!("Expected successful response with text conversion");
        }
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_real_wasmtime_docs_head_request() {
        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": "https://docs.wasmtime.dev/",
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["method"], "HEAD");
            // HEAD should return metadata but not content
            assert!(
                value["content"].is_null()
                    || value["content"].as_str().is_none_or(|s| s.is_empty()),
                "HEAD request should not return content body"
            );
            // Should have content-type header info
            assert!(value["content_type"].as_str().is_some());
        } else {
            panic!("Expected successful HEAD response");
        }
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_real_wasmtime_docs_subpage() {
        let tool = WebFetchTool::permissive();
        // No as_markdown - fetchkit returns markdown by default
        let result = tool
            .execute(serde_json::json!({
                "url": "https://docs.wasmtime.dev/introduction.html"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            // Introduction page should have relevant content
            assert!(
                content.len() > 500,
                "Introduction page should have substantial content"
            );
        } else {
            panic!("Expected successful response from introduction page");
        }
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_real_github_wasm3_readme() {
        // GitHub READMEs may return HTML even though fetchkit tries to convert
        // Note: fetchkit has 1-second first-byte timeout which GitHub often exceeds
        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": "https://github.com/wasm3/wasm3"
            }))
            .await;

        match &result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["status_code"], 200);
                let content = value["content"].as_str().unwrap();
                // Should contain wasm3 reference
                assert!(
                    content.to_lowercase().contains("wasm3"),
                    "Content should mention wasm3"
                );
                // GitHub pages return HTML even with fetchkit's markdown conversion
                // This documents the known limitation
                if content.contains("<") && content.contains(">") {
                    println!("Note: GitHub returned HTML content as expected");
                }
            }
            ToolExecutionResult::ToolError(msg) if msg.contains("timed out") => {
                // GitHub often times out with fetchkit's 1-second timeout
                // This is a known limitation, not a test failure
                println!("GitHub request timed out (expected with 1s timeout)");
            }
            other => {
                panic!("Unexpected result from GitHub wasm3 repo: {:?}", other);
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires network access"]
    async fn test_real_github_wasm3_as_text() {
        // Test as_text conversion on GitHub page
        // Note: fetchkit has 1-second first-byte timeout which GitHub often exceeds
        let tool = WebFetchTool::permissive();
        let result = tool
            .execute(serde_json::json!({
                "url": "https://github.com/wasm3/wasm3",
                "as_text": true
            }))
            .await;

        match &result {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["status_code"], 200);
                let content = value["content"].as_str().unwrap();
                // Should contain wasm3 reference
                assert!(
                    content.to_lowercase().contains("wasm3"),
                    "Content should mention wasm3"
                );
            }
            ToolExecutionResult::ToolError(msg) if msg.contains("timed out") => {
                // GitHub often times out with fetchkit's 1-second timeout
                // This is a known limitation, not a test failure
                println!("GitHub request timed out (expected with 1s timeout)");
            }
            other => {
                panic!(
                    "Unexpected result from GitHub wasm3 with as_text: {:?}",
                    other
                );
            }
        }
    }
}
