//! GET `/mcp`: a browser gets a page, MCP clients keep the transport's 405.
use super::*;

async fn get_mcp(server: &TestServer, accept: Option<&str>) -> test_harness::TestResponse {
    let headers = accept.map(|a| vec![("accept", a)]).unwrap_or_default();
    server
        .request_raw(Method::GET, "/mcp", headers, Vec::new())
        .await
}

#[tokio::test]
async fn browser_get_renders_page_for_agents() {
    let server = TestServer::in_memory().await;
    let resp = get_mcp(
        &server,
        Some("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp.headers()["content-type"].to_str().unwrap();
    assert!(content_type.starts_with("text/html"), "{content_type}");
    let csp = resp.headers()["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("script-src 'sha256-"), "{csp}");
    let html = resp.text();
    assert!(html.contains("This URL is for your agent"));
    assert!(html.contains("/mcp</code>"));
}

#[tokio::test]
async fn mcp_client_get_keeps_method_not_allowed() {
    let server = TestServer::in_memory().await;
    for accept in [
        Some("text/event-stream"),
        Some("application/json, text/event-stream"),
        Some("*/*"),
        None,
    ] {
        let resp = get_mcp(&server, accept).await;
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED, "{accept:?}");
        assert_eq!(resp.headers()["allow"], "POST", "{accept:?}");
        assert!(resp.text().is_empty(), "{accept:?}");
    }
}

#[tokio::test]
async fn post_still_speaks_json_rpc() {
    let server = TestServer::in_memory().await;
    let resp = mcp_request_raw(&server, "ping", json!({}), vec![]).await;
    assert_eq!(resp.status(), StatusCode::OK);
}
