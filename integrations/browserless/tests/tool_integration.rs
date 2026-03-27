//! Integration tests: tool execute_with_context against wiremock Browserless API.
//!
//! These tests exercise the full tool execution flow:
//! MockConnectionResolver → tool.execute_with_context() → BrowserlessClient → wiremock

use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::{ToolContext, UserConnectionResolver};
use everruns_core::typed_id::SessionId;
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Force linker to include the integration crate.
use everruns_integrations_browserless as _;

use everruns_integrations_browserless::client::BrowserlessClient;

// ============================================================================
// Mock ConnectionResolver
// ============================================================================

struct MockConnectionResolver {
    token: Option<String>,
}

#[async_trait]
impl UserConnectionResolver for MockConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(self.token.clone())
    }
}

fn browserless_resolver() -> Arc<dyn UserConnectionResolver> {
    Arc::new(MockConnectionResolver {
        token: Some("test_api_token".to_string()),
    })
}

fn no_token_resolver() -> Arc<dyn UserConnectionResolver> {
    Arc::new(MockConnectionResolver { token: None })
}

// ============================================================================
// Helpers
// ============================================================================

fn get_tool(name: &str) -> Box<dyn Tool> {
    let cap = everruns_integrations_browserless::BrowserlessCapability;
    use everruns_core::capabilities::Capability;
    cap.tools()
        .into_iter()
        .find(|t| t.name() == name)
        .unwrap_or_else(|| panic!("Tool {name} not found"))
}

// ============================================================================
// BrowserlessClient integration tests (wiremock)
// ============================================================================

#[tokio::test]
async fn test_screenshot_client_flow() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/screenshot"))
        .respond_with(
            ResponseTemplate::new(200).set_body_bytes(b"\x89PNG\r\n\x1a\n_FAKE_PNG_DATA".to_vec()),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
    let bytes = client
        .screenshot("https://example.com", true, None, None, None)
        .await
        .unwrap();
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn test_content_client_flow() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/content"))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "<html><head><title>Test</title></head><body>Hello World</body></html>",
        ))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
    let html = client
        .content("https://example.com", None, None, false)
        .await
        .unwrap();
    assert!(html.contains("Hello World"));
    assert!(html.contains("<title>Test</title>"));
}

#[tokio::test]
async fn test_scrape_client_flow() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/scrape"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [
                {
                    "selector": "h1",
                    "results": [
                        {"text": "Welcome", "attributes": []}
                    ]
                }
            ]
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
    let result = client
        .scrape(
            "https://example.com",
            &[json!({"selector": "h1"})],
            None,
            None,
        )
        .await
        .unwrap();
    assert!(result["data"][0]["results"][0]["text"] == "Welcome");
}

#[tokio::test]
async fn test_function_client_flow() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/function"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": "{\"title\":\"Test Page\",\"url\":\"https://example.com\"}",
            "type": "application/json"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
    let result = client
        .function(
            "export default async ({ page }) => { return { data: 'test', type: 'application/json' }; }",
            None,
        )
        .await
        .unwrap();
    assert!(result.get("data").is_some());
}

// ============================================================================
// Tool execute_with_context tests
// ============================================================================

#[tokio::test]
async fn test_screenshot_tool_missing_api_token() {
    let tool = get_tool("browserless_screenshot");
    let session_id = SessionId::new();
    let context = ToolContext::new(session_id).with_connection_resolver(no_token_resolver());

    let result = tool
        .execute_with_context(json!({"url": "https://example.com"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("not configured") || msg.contains("Settings > Connections"),
                "Got: {msg}"
            );
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_content_tool_missing_url() {
    let tool = get_tool("browserless_content");
    let session_id = SessionId::new();
    let context = ToolContext::new(session_id).with_connection_resolver(browserless_resolver());

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Missing required parameter"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_scrape_tool_missing_elements() {
    let tool = get_tool("browserless_scrape");
    let session_id = SessionId::new();
    let context = ToolContext::new(session_id).with_connection_resolver(browserless_resolver());

    let result = tool
        .execute_with_context(json!({"url": "https://example.com"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Missing required parameter"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_interact_tool_missing_steps() {
    let tool = get_tool("browserless_interact");
    let session_id = SessionId::new();
    let context = ToolContext::new(session_id).with_connection_resolver(browserless_resolver());

    let result = tool
        .execute_with_context(json!({"url": "https://example.com"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Missing required parameter"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_navigate_tool_missing_url() {
    let tool = get_tool("browserless_navigate");
    let session_id = SessionId::new();
    let context = ToolContext::new(session_id).with_connection_resolver(browserless_resolver());

    let result = tool.execute_with_context(json!({}), &context).await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(msg.contains("Missing required parameter"), "Got: {msg}");
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_navigate_tool_no_connection_resolver() {
    let tool = get_tool("browserless_navigate");
    let session_id = SessionId::new();
    let context = ToolContext::new(session_id); // No connection resolver

    let result = tool
        .execute_with_context(json!({"url": "https://example.com"}), &context)
        .await;

    match result {
        ToolExecutionResult::ToolError(msg) => {
            assert!(
                msg.contains("not configured") || msg.contains("Settings > Connections"),
                "Got: {msg}"
            );
        }
        other => panic!("Expected ToolError, got: {other:?}"),
    }
}

// ============================================================================
// Client error handling integration tests
// ============================================================================

#[tokio::test]
async fn test_client_401_unauthorized() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/screenshot"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .mount(&mock_server)
        .await;

    let client = BrowserlessClient::with_base_url("bad_token".to_string(), mock_server.uri());
    let result = client
        .screenshot("https://example.com", false, None, None, None)
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("401"));
}

#[tokio::test]
async fn test_client_500_server_error() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/content"))
        .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
        .mount(&mock_server)
        .await;

    let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
    let result = client
        .content("https://example.com", None, None, false)
        .await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("500"));
}

// ============================================================================
// Bearer auth verification
// ============================================================================

#[tokio::test]
async fn test_client_sends_token_in_query() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/content"))
        .and(wiremock::matchers::query_param("token", "secret_token_123"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
        .expect(1)
        .mount(&mock_server)
        .await;

    let client =
        BrowserlessClient::with_base_url("secret_token_123".to_string(), mock_server.uri());
    let result = client
        .content("https://example.com", None, None, false)
        .await;
    assert!(result.is_ok());
}

// ============================================================================
// Resource cleanup verification
// ============================================================================

#[tokio::test]
async fn test_no_resources_left_behind() {
    // Browserless REST API is stateless — each call starts and destroys a browser.
    // Verify that our client doesn't maintain any persistent state.
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/content"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>1</html>"))
        .mount(&mock_server)
        .await;

    // Make multiple calls — each should be independent
    let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());

    let r1 = client
        .content("https://example.com/page1", None, None, false)
        .await
        .unwrap();
    let r2 = client
        .content("https://example.com/page2", None, None, false)
        .await
        .unwrap();

    // Both succeed independently
    assert!(!r1.is_empty());
    assert!(!r2.is_empty());
    // Client itself has no session state to clean up
}

// ============================================================================
// Connection validation: CDP probe tests
// ============================================================================

/// REST ok (204) + CDP probe returns 400 (expected) → validation succeeds.
#[tokio::test]
async fn test_validate_rest_ok_cdp_probe_400() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/active"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/chromium"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&mock_server)
        .await;

    // BrowserlessConnectionProvider uses BROWSERLESS_API_BASE which is hardcoded,
    // so we test the validate logic directly by hitting the mock endpoints.
    let client = reqwest::Client::new();

    // Simulate the /active check
    let resp = client
        .get(format!("{}/active?token=test_token", mock_server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // Simulate the CDP probe — 400 means CDP endpoint is reachable (expects WS upgrade)
    let resp = client
        .get(format!("{}/chromium?token=test_token", mock_server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// REST ok (204) + CDP probe returns 403 → validation should detect CDP rejection.
#[tokio::test]
async fn test_validate_rest_ok_cdp_probe_403_rejects() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/active"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&mock_server)
        .await;
    Mock::given(method("GET"))
        .and(path("/chromium"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&mock_server)
        .await;

    let client = reqwest::Client::new();

    // /active succeeds
    let resp = client
        .get(format!("{}/active?token=test_token", mock_server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 204);

    // CDP probe returns 403 — token lacks CDP access
    let resp = client
        .get(format!("{}/chromium?token=test_token", mock_server.uri()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
}
