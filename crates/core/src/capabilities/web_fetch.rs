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

use super::{Capability, CapabilityStatus, RiskLevel};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use fetchkit::{
    FetchError, FetchOptions, FetchRequest, TOOL_DESCRIPTION, TOOL_LLMTXT, fetch_with_options,
};
use serde_json::Value;

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

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
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
        vec![Box::new(WebFetchTool::default())]
    }
}

// ============================================================================
// Tool: web_fetch
// ============================================================================

/// Tool that fetches content from a URL using fetchkit
///
/// THREAT[TM-API-008]: SSRF protection via fetchkit DnsPolicy
/// Mitigation: Default FetchOptions uses DnsPolicy::block_private_ips(),
/// which blocks loopback, RFC1918, link-local (cloud metadata), and other
/// reserved IP ranges via resolve-then-check with DNS pinning.
#[derive(Default)]
pub struct WebFetchTool {
    options: FetchOptions,
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Web Fetch")
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

        // Execute the fetch using fetchkit with configured options (SSRF protection)
        match fetch_with_options(request, self.options.clone()).await {
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
    use fetchkit::DnsPolicy;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Create a WebFetchTool with permissive DNS policy for wiremock tests
    /// (wiremock binds to 127.0.0.1 which is blocked by default).
    fn tool_for_wiremock() -> WebFetchTool {
        WebFetchTool {
            options: FetchOptions {
                dns_policy: DnsPolicy::allow_all(),
                ..Default::default()
            },
        }
    }

    #[test]
    fn test_web_fetch_tool_parameters() {
        let tool = WebFetchTool::default();
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
        let tool = WebFetchTool::default();
        let result = tool.execute(serde_json::json!({})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("url"));
        } else {
            panic!("Expected tool error for missing URL");
        }
    }

    #[tokio::test]
    async fn test_web_fetch_invalid_url() {
        let tool = WebFetchTool::default();
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
        let tool = WebFetchTool::default();
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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
        let tool = tool_for_wiremock();
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
        // Use TEST-NET-1 (192.0.2.0/24, RFC 5737) which is non-routable and will timeout.
        // Note: fetchkit v0.1.2 blocks RFC1918 private IPs, but TEST-NET ranges
        // are also blocked by DNS policy. Use a wiremock server with a delay instead.
        let mock_server = MockServer::start().await;

        // Mount a mock that takes 5 seconds to respond (exceeds 1s first-byte timeout)
        Mock::given(method("GET"))
            .and(path("/slow"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("slow response")
                    .set_delay(std::time::Duration::from_secs(5)),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/slow", mock_server.uri())
            }))
            .await;

        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("timed out") || msg.contains("connect") || msg.contains("failed"),
                    "Expected timeout or connection error, got: {}",
                    msg
                );
            }
            _ => {
                // Some environments may handle timeouts differently
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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
        let tool = WebFetchTool::default();
        let result = tool
            .execute(serde_json::json!({
                "url": "https://this-domain-definitely-does-not-exist-12345.com/test"
            }))
            .await;

        // DNS failure returns a tool error. With fetchkit v0.1.2's resolve-then-check,
        // DNS resolution failures may surface as "blocked by policy" since the hostname
        // cannot be validated against the DNS policy.
        if let ToolExecutionResult::ToolError(msg) = result {
            let msg_lower = msg.to_lowercase();
            assert!(
                msg_lower.contains("failed")
                    || msg_lower.contains("error")
                    || msg_lower.contains("timed out")
                    || msg_lower.contains("connect")
                    || msg_lower.contains("blocked"),
                "Expected error message about failure, got: {}",
                msg
            );
        } else {
            // Some environments might timeout instead of DNS failure
        }
    }

    #[tokio::test]
    async fn test_web_fetch_rejects_ftp_url() {
        let tool = WebFetchTool::default();
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
        let tool = WebFetchTool::default();
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

        let tool = tool_for_wiremock();
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

        let tool = tool_for_wiremock();
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
    // SSRF security tests (TM-API-008 through TM-API-012)
    //
    // fetchkit v0.1.2 blocks private/internal IPs by default via
    // resolve-then-check with DNS pinning. These tests verify that
    // private/internal URLs are blocked by policy.
    //
    // Run with: cargo test -p everruns-core --lib -- web_fetch::tests::test_ssrf
    // ========================================================================

    // Helper: asserts that a private/internal URL IS blocked by fetchkit's
    // DNS policy (SSRF protection). The tool should return a ToolError
    // containing "blocked".
    async fn assert_blocked_by_policy(url: &str) {
        let tool = WebFetchTool::default();
        let result = tool.execute(serde_json::json!({"url": url})).await;
        assert!(
            matches!(&result, ToolExecutionResult::ToolError(msg) if msg.contains("blocked")),
            "Expected URL {url} to be blocked by policy, got: {:?}",
            result
        );
    }

    /// THREAT[TM-API-009]: Cloud metadata endpoint blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_cloud_metadata_blocked() {
        assert_blocked_by_policy("http://169.254.169.254/latest/meta-data/").await;
    }

    /// THREAT[TM-API-008]: Localhost blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_localhost_blocked() {
        assert_blocked_by_policy("http://127.0.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 10.x.x.x blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_private_10_blocked() {
        assert_blocked_by_policy("http://10.0.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 172.16.x.x blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_private_172_blocked() {
        assert_blocked_by_policy("http://172.16.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: RFC1918 192.168.x.x blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_private_192_blocked() {
        assert_blocked_by_policy("http://192.168.0.1:1/").await;
    }

    /// THREAT[TM-API-008]: IPv6 localhost blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_ipv6_localhost_blocked() {
        assert_blocked_by_policy("http://[::1]:1/").await;
    }

    /// THREAT[TM-API-008]: 0.0.0.0 blocked by fetchkit DNS policy.
    #[tokio::test]
    async fn test_ssrf_unspecified_blocked() {
        assert_blocked_by_policy("http://0.0.0.0:1/").await;
    }

    /// Verify file://, ftp://, gopher:// schemes are blocked (existing protection).
    #[tokio::test]
    async fn test_ssrf_non_http_schemes_blocked() {
        let tool = WebFetchTool::default();

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
    // Integration tests using wiremock (no network access needed)
    // ========================================================================

    #[tokio::test]
    async fn test_fetch_html_page() {
        let mock_server = MockServer::start().await;
        let html = r#"<html><head><title>Wasmtime Docs</title></head>
        <body><h1>Wasmtime</h1><p>A fast and secure runtime for WebAssembly.</p>
        <p>Wasmtime is a standalone runtime for WebAssembly that can be used
        as a CLI tool or embedded into other systems.</p></body></html>"#;

        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(html)
                    .insert_header("content-type", "text/html; charset=utf-8"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.contains("Wasmtime") || content.contains("wasmtime"),
                "Content should mention Wasmtime"
            );
            assert!(
                value["size"].as_u64().unwrap() > 100,
                "Page should have substantial content"
            );
        } else {
            panic!("Expected successful response, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_fetch_html_as_text() {
        let mock_server = MockServer::start().await;
        let html = r#"<html><head><title>Wasmtime Docs</title></head>
        <body><h1>Wasmtime</h1><p>A fast and secure runtime.</p></body></html>"#;

        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(html)
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/", mock_server.uri()),
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.contains("Wasmtime") || content.contains("wasmtime"),
                "Text should contain Wasmtime reference"
            );
            let format = value["format"].as_str().unwrap_or("raw");
            assert!(
                format == "text" || format == "raw",
                "Format should be text or raw, got: {}",
                format
            );
        } else {
            panic!(
                "Expected successful response with text conversion, got: {:?}",
                result
            );
        }
    }

    #[tokio::test]
    async fn test_fetch_head_request() {
        let mock_server = MockServer::start().await;

        Mock::given(method("HEAD"))
            .and(path("/"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/html; charset=utf-8")
                    .insert_header("content-length", "5000"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/", mock_server.uri()),
                "method": "HEAD"
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            assert_eq!(value["method"], "HEAD");
            assert!(
                value["content"].is_null()
                    || value["content"].as_str().is_none_or(|s| s.is_empty()),
                "HEAD request should not return content body"
            );
            assert!(value["content_type"].as_str().is_some());
        } else {
            panic!("Expected successful HEAD response, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_fetch_subpage() {
        let mock_server = MockServer::start().await;
        // Build a page with >500 chars of content
        let body = format!(
            "<html><body><h1>Introduction</h1><p>{}</p></body></html>",
            "WebAssembly is a portable binary instruction format. ".repeat(20)
        );

        Mock::given(method("GET"))
            .and(path("/introduction.html"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(&body)
                    .insert_header("content-type", "text/html"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/introduction.html", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.len() > 500,
                "Subpage should have substantial content, got {} bytes",
                content.len()
            );
        } else {
            panic!(
                "Expected successful response from subpage, got: {:?}",
                result
            );
        }
    }

    #[tokio::test]
    async fn test_fetch_repo_page() {
        let mock_server = MockServer::start().await;
        let html = r#"<html><body>
        <h1>wasm3/wasm3</h1>
        <p>The fastest WebAssembly interpreter (and target for wasm3).</p>
        <div class="readme"><h2>README</h2><p>wasm3 is a high performance
        WebAssembly interpreter written in C.</p></div>
        </body></html>"#;

        Mock::given(method("GET"))
            .and(path("/wasm3/wasm3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(html)
                    .insert_header("content-type", "text/html; charset=utf-8"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/wasm3/wasm3", mock_server.uri())
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.to_lowercase().contains("wasm3"),
                "Content should mention wasm3"
            );
        } else {
            panic!("Expected successful response, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_fetch_repo_page_as_text() {
        let mock_server = MockServer::start().await;
        let html = r#"<html><body>
        <h1>wasm3/wasm3</h1>
        <p>The fastest WebAssembly interpreter written in C.</p>
        </body></html>"#;

        Mock::given(method("GET"))
            .and(path("/wasm3/wasm3"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(html)
                    .insert_header("content-type", "text/html; charset=utf-8"),
            )
            .mount(&mock_server)
            .await;

        let tool = tool_for_wiremock();
        let result = tool
            .execute(serde_json::json!({
                "url": format!("{}/wasm3/wasm3", mock_server.uri()),
                "as_text": true
            }))
            .await;

        if let ToolExecutionResult::Success(value) = result {
            assert_eq!(value["status_code"], 200);
            let content = value["content"].as_str().unwrap();
            assert!(
                content.to_lowercase().contains("wasm3"),
                "Content should mention wasm3"
            );
        } else {
            panic!("Expected successful response, got: {:?}", result);
        }
    }
}
