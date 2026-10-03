//! Integration tests: tool execute_with_context against wiremock Browserless API.
//!
//! These tests exercise the full tool execution flow:
//! MockConnectionResolver → tool.execute_with_context() → BrowserlessClient → wiremock

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionId;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::{connection_services::UserConnectionResolver, tool_context::ToolContext};
use serde_json::json;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// Force linker to include the integration crate.
use everruns_integrations_browserless as _;

use everruns_contracts::connector::Connector;
use everruns_integrations_browserless::client::BrowserlessClient;
use everruns_integrations_browserless::connection::BrowserlessConnector;

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
        .content("https://example.com", None, None, false, &[])
        .await;
    assert!(result.is_ok());
}

// ============================================================================
// Connection validation: CDP probe tests
// ============================================================================

// `BrowserlessConnector::validate` reads its base URL from the
// `BROWSERLESS_API_BASE` env var (see `browserless_api_base()`), which is
// process-global. Serialize every test below that mutates it so parallel
// `cargo test` threads don't race each other's override.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Point `BROWSERLESS_API_BASE` at `mock_server` for the duration of `f`,
/// holding `ENV_LOCK` so no other test observes the override mid-flight.
/// A `tokio::sync::Mutex` is used (rather than `std::sync::Mutex`) precisely
/// so the guard can be held across `f()`'s `.await`.
async fn with_mock_api_base<F, Fut, T>(mock_server: &MockServer, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let _guard = ENV_LOCK.lock().await;
    // SAFETY: serialized by ENV_LOCK above; no other test reads or writes
    // BROWSERLESS_API_BASE while this guard is held.
    unsafe { std::env::set_var("BROWSERLESS_API_BASE", mock_server.uri()) };
    let result = f().await;
    unsafe { std::env::remove_var("BROWSERLESS_API_BASE") };
    result
}

/// REST ok (204) + CDP probe returns 400 (expected) → validation succeeds.
#[tokio::test]
async fn test_validate_accepts_valid_token() {
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

    let result = with_mock_api_base(&mock_server, || async {
        BrowserlessConnector.validate("test_token").await
    })
    .await;

    assert!(result.is_ok(), "expected Ok, got {result:?}");
}

/// REST probe rejects the token (401) → validation fails before the CDP probe.
#[tokio::test]
async fn test_validate_rejects_invalid_token() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/active"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock_server)
        .await;

    let result = with_mock_api_base(&mock_server, || async {
        BrowserlessConnector.validate("bad_token").await
    })
    .await;

    let err = result.expect_err("expected Err for an invalid token");
    assert!(
        err.contains("Invalid API token"),
        "unexpected message: {err}"
    );
}

/// REST ok, but CDP probe rejects the token (403) → validation fails with a
/// CDP-specific message distinguishing it from a plain invalid-token error.
#[tokio::test]
async fn test_validate_rejects_token_without_cdp_access() {
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

    let result = with_mock_api_base(&mock_server, || async {
        BrowserlessConnector.validate("rest_only_token").await
    })
    .await;

    let err = result.expect_err("expected Err when CDP probe rejects the token");
    assert!(
        err.contains("does not support CDP"),
        "unexpected message: {err}"
    );
}

/// A blank token is rejected the same way an invalid one is: the REST API
/// returns 401 for it, and validate() surfaces the same error without ever
/// attempting the CDP probe.
#[tokio::test]
async fn test_validate_rejects_blank_token() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/active"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&mock_server)
        .await;

    let result = with_mock_api_base(&mock_server, || async {
        BrowserlessConnector.validate("").await
    })
    .await;

    let err = result.expect_err("expected Err for a blank token");
    assert!(
        err.contains("Invalid API token"),
        "unexpected message: {err}"
    );
}
