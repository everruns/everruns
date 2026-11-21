#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live Parallel MCP tests.
//!
//! Gated behind:
//!   cargo test -p everruns-integrations --features integration
//!
//! Parallel MCP is free by default, so these tests require no secrets.

#![cfg(feature = "parallel-integration")]

use everruns_contracts::runtime::{McpToolCallRequest, McpToolCallResponse, McpToolsListRequest};
use everruns_integrations::parallel::PARALLEL_MCP_URL;
use serde_json::json;

fn with_api_key(builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    if let Ok(api_key) = std::env::var("PARALLEL_API_KEY") {
        builder.header("x-api-key", api_key)
    } else {
        builder
    }
    .header("Accept", "application/json, text/event-stream")
}

/// mcp.parallel.ai answers Streamable HTTP requests as a one-event SSE stream
/// even when the client accepts JSON; read the JSON-RPC body from either shape.
async fn json_rpc_body<T: serde::de::DeserializeOwned>(response: reqwest::Response) -> T {
    let text = response.text().await.expect("MCP response body");
    let payload = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .find(|data| !data.is_empty())
        .unwrap_or(text.trim());
    serde_json::from_str(payload).expect("valid MCP JSON-RPC response")
}

fn is_free_tier_rate_limit(message: &str) -> bool {
    message.contains("free-tier rate limit for Parallel Search MCP")
}

#[tokio::test]
async fn smoke_tools_list_free_mcp() {
    let response = with_api_key(
        reqwest::Client::new()
            .post(PARALLEL_MCP_URL)
            .json(&McpToolsListRequest::default()),
    )
    .send()
    .await
    .expect("tools/list request");

    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        eprintln!("Skipping unauthenticated Parallel MCP smoke test: free-tier rate limit");
        return;
    }
    assert!(
        response.status().is_success(),
        "status: {}",
        response.status()
    );

    let body: everruns_contracts::runtime::McpToolsListResponse = json_rpc_body(response).await;
    if body
        .error
        .as_ref()
        .is_some_and(|error| is_free_tier_rate_limit(&error.message))
    {
        eprintln!("Skipping unauthenticated Parallel MCP smoke test: free-tier rate limit");
        return;
    }
    let result = body.result.expect("tools/list result");
    let names: Vec<_> = result.tools.iter().map(|tool| tool.name.as_str()).collect();
    assert!(names.contains(&"web_search"));
    assert!(names.contains(&"web_fetch"));
}

#[tokio::test]
async fn smoke_web_search_free_mcp() {
    let request = McpToolCallRequest::new(
        1,
        "web_search".to_string(),
        Some(json!({
            "objective": "Find the official Everruns GitHub organization URL.",
            "search_queries": ["Everruns GitHub"],
            "session_id": "everruns-parallel-live-test-0123456789abcdef"
        })),
    );

    let response = with_api_key(reqwest::Client::new().post(PARALLEL_MCP_URL).json(&request))
        .send()
        .await
        .expect("tools/call request");

    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        eprintln!("Skipping unauthenticated Parallel MCP smoke test: free-tier rate limit");
        return;
    }
    assert!(
        response.status().is_success(),
        "status: {}",
        response.status()
    );

    let body: McpToolCallResponse = json_rpc_body(response).await;
    if body
        .error
        .as_ref()
        .is_some_and(|error| is_free_tier_rate_limit(&error.message))
    {
        eprintln!("Skipping unauthenticated Parallel MCP smoke test: free-tier rate limit");
        return;
    }
    assert!(body.error.is_none(), "MCP error: {:?}", body.error);
    let result = body.result.expect("tool call result");
    assert!(!result.content.is_empty());
}
