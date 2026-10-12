//! Saved per-tool risk labels on MCP servers: listing a server's tools and
//! setting or clearing a person's label over the HTTP API.

use crate::test_harness;

use axum::http::StatusCode;
use everruns_server::storage::UpdateMcpServerTools;
use serde_json::{Value, json};
use test_harness::TestServer;

async fn server_with_tools(server: &TestServer) -> String {
    let created: Value = server
        .post(
            "/v1/mcp-servers",
            json!({
                "name": format!("docs-{}", uuid::Uuid::new_v4().simple()),
                "url": "https://example.com/mcp",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = created["id"].as_str().unwrap().to_string();
    let uuid = id
        .parse::<everruns_contracts::typed_id::McpServerId>()
        .unwrap()
        .uuid();
    server
        .db
        .update_mcp_server_tools(
            1,
            uuid,
            UpdateMcpServerTools {
                cached_tools: json!([
                    {
                        "name": "search",
                        "title": "Search docs",
                        "description": "Find pages",
                        "inputSchema": {"type": "object"},
                        "annotations": {"readOnlyHint": true}
                    },
                    {"name": "publish", "inputSchema": {"type": "object"}}
                ]),
            },
        )
        .await
        .unwrap();
    id
}

async fn put_label(
    server: &TestServer,
    id: &str,
    tool: &str,
    body: Value,
) -> test_harness::TestResponse {
    server
        .put(&format!("/v1/mcp-servers/{id}/tools/{tool}/label"), body)
        .await
}

#[tokio::test]
async fn lists_tools_and_sets_and_clears_a_label() {
    let server = TestServer::in_memory().await;
    let id = server_with_tools(&server).await;

    let tools: Value = server
        .get(&format!("/v1/mcp-servers/{id}/tools"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        tools,
        json!([
            {"name": "publish", "title": null, "description": null, "annotations": null,
             "label": null, "suggested_label": null},
            {"name": "search", "title": "Search docs", "description": "Find pages",
             "annotations": {"readOnlyHint": true}, "label": null, "suggested_label": null}
        ])
    );

    let set: Value = put_label(&server, &id, "search", json!({"label": "read_only"}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(set["name"], "search");
    assert_eq!(set["label"], "read_only");
    put_label(&server, &id, "publish", json!({"label": "changes"}))
        .await
        .assert_status(StatusCode::OK);

    let tools: Value = server
        .get(&format!("/v1/mcp-servers/{id}/tools"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(tools[0]["label"], "changes");
    assert_eq!(tools[1]["label"], "read_only");

    let cleared: Value = put_label(&server, &id, "search", json!({"label": null}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(cleared["label"], Value::Null);
    let tools: Value = server
        .get(&format!("/v1/mcp-servers/{id}/tools"))
        .await
        .json();
    assert_eq!(tools[1]["label"], Value::Null);
    assert_eq!(tools[0]["label"], "changes");
}

#[tokio::test]
async fn rejects_unknown_labels_tools_and_servers() {
    let server = TestServer::in_memory().await;
    let id = server_with_tools(&server).await;

    put_label(&server, &id, "search", json!({"label": "safe"}))
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
    put_label(&server, &id, "not_a_tool", json!({"label": "read_only"}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    let missing = everruns_contracts::typed_id::McpServerId::new().to_string();
    put_label(&server, &missing, "search", json!({"label": "read_only"}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .get(&format!("/v1/mcp-servers/{missing}/tools"))
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // Nothing was saved by the rejected calls.
    let tools: Value = server
        .get(&format!("/v1/mcp-servers/{id}/tools"))
        .await
        .json();
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .all(|t| t["label"].is_null())
    );
}

#[tokio::test]
async fn setting_a_label_confirms_and_drops_the_suggestion() {
    let server = TestServer::in_memory().await;
    let id = server_with_tools(&server).await;
    let uuid = id
        .parse::<everruns_contracts::typed_id::McpServerId>()
        .unwrap()
        .uuid();
    for tool in ["search", "publish"] {
        server
            .db
            .set_mcp_tool_suggestion(1, uuid, tool, "changes")
            .await
            .unwrap();
    }

    // Shown, never applied.
    let tools: Value = server
        .get(&format!("/v1/mcp-servers/{id}/tools"))
        .await
        .json();
    assert_eq!(tools[0]["suggested_label"], "changes");
    assert_eq!(tools[0]["label"], Value::Null);

    let set: Value = put_label(&server, &id, "publish", json!({"label": "changes"}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(set["label"], "changes");
    assert_eq!(set["suggested_label"], Value::Null);
    let tools: Value = server
        .get(&format!("/v1/mcp-servers/{id}/tools"))
        .await
        .json();
    assert_eq!(
        tools[1]["suggested_label"], "changes",
        "other tools keep theirs"
    );
}

#[tokio::test]
async fn suggesting_labels_needs_a_decision_service() {
    // The test server composes the deployment's decisions from the process
    // environment; only the unconfigured case is deterministic here. The
    // rating itself is covered by the domain unit tests with a stub service.
    if ["UTILITY_TYPESAFE_API_KEY", "UTILITY_DECISION_DRIVER"]
        .iter()
        .any(|name| std::env::var(name).is_ok_and(|value| !value.trim().is_empty()))
    {
        return;
    }
    let server = TestServer::in_memory().await;
    let id = server_with_tools(&server).await;
    let response = server
        .post(
            &format!("/v1/mcp-servers/{id}/tools/suggest-labels"),
            json!({}),
        )
        .await
        .assert_status(StatusCode::SERVICE_UNAVAILABLE);
    let body: Value = response.json();
    assert!(
        body.to_string().contains("decision service"),
        "explains what is missing: {body}"
    );
    let missing = everruns_contracts::typed_id::McpServerId::new().to_string();
    server
        .post(
            &format!("/v1/mcp-servers/{missing}/tools/suggest-labels"),
            json!({}),
        )
        .await
        .assert_status(StatusCode::SERVICE_UNAVAILABLE);
}
