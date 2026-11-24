use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tools_list_carries_public_caching_hints_for_2026() {
    let server = TestServer::new().await;

    let resp = mcp_call_with_headers(
        &server,
        "tools/list",
        json!({}),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    assert_eq!(resp["result"]["resultType"], "complete");
    assert_eq!(resp["result"]["cacheScope"], "public");
    let ttl = resp["result"]["ttlMs"]
        .as_i64()
        .expect("ttlMs must be an integer");
    assert!(ttl >= 0, "ttlMs must be non-negative, got {ttl}");
}

/// `resources/read` returns org-scoped data, so it must never be marked public.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_resources_read_is_private_for_2026() {
    let server = TestServer::new().await;

    let resp = mcp_call_with_headers(
        &server,
        "resources/read",
        json!({ "uri": "everruns://agents" }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    assert_eq!(resp["result"]["resultType"], "complete");
    assert_eq!(
        resp["result"]["cacheScope"], "private",
        "org-scoped payloads must not be shareable across authorization contexts"
    );
    assert!(resp["result"]["ttlMs"].as_i64().is_some());
}

/// 2025-era clients see the era they negotiated — no 2026 result metadata.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tools_list_omits_caching_hints_for_2025() {
    let server = TestServer::new().await;

    let resp = mcp_call_with_headers(
        &server,
        "tools/list",
        json!({}),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_LATEST)],
    )
    .await;

    assert!(resp["result"]["tools"].is_array());
    assert!(resp["result"].get("ttlMs").is_none());
    assert!(resp["result"].get("cacheScope").is_none());
    assert!(resp["result"].get("resultType").is_none());
}
