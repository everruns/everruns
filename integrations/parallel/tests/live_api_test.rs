//! Live Parallel MCP tests.
//!
//! Gated behind:
//!   cargo test -p everruns-integrations-parallel --features integration
//!
//! Parallel MCP is free by default, so these tests require no secrets.

#![cfg(feature = "integration")]

use everruns_core::{McpToolCallRequest, McpToolCallResponse, McpToolsListRequest};
use everruns_integrations_parallel::PARALLEL_MCP_URL;
use serde_json::json;

#[tokio::test]
async fn smoke_tools_list_free_mcp() {
    let response = reqwest::Client::new()
        .post(PARALLEL_MCP_URL)
        .json(&McpToolsListRequest::default())
        .send()
        .await
        .expect("tools/list request");

    assert!(
        response.status().is_success(),
        "status: {}",
        response.status()
    );

    let body: everruns_core::McpToolsListResponse = response
        .json()
        .await
        .expect("valid MCP tools/list response");
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

    let response = reqwest::Client::new()
        .post(PARALLEL_MCP_URL)
        .json(&request)
        .send()
        .await
        .expect("tools/call request");

    assert!(
        response.status().is_success(),
        "status: {}",
        response.status()
    );

    let body: McpToolCallResponse = response.json().await.expect("valid MCP call response");
    assert!(body.error.is_none(), "MCP error: {:?}", body.error);
    let result = body.result.expect("tool call result");
    assert!(!result.content.is_empty());
}
