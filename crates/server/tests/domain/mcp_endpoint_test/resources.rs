//! MCP resources on the Everruns MCP server: org scoping, templates, and
//! unknown URIs.

use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_resources_read_cannot_escape_org_scope() {
    let server = TestServer::in_memory().await;
    let marker = format!("org2-resource-marker-{}", unique_suffix());

    let org2: Value = server
        .post(
            "/v1/orgs",
            json!({ "name": format!("MCP Resource Target {}", unique_suffix()) }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let org2_id = org2["id"].as_str().expect("org2 id").to_string();

    let switch_resp = server
        .post("/v1/users/me/switch-org", json!({ "org_id": org2_id }))
        .await
        .assert_status(StatusCode::OK);
    let org2_cookie = extract_cookie(switch_resp.headers(), "everruns_org");

    let create_agent_resp = mcp_tool_call_with_headers(
        &server,
        "execute",
        json!({
            "commands": format!(
                "create_agent --name '{marker}' --display_name 'Org 2 Resource Marker' --system_prompt 'tenant marker'"
            )
        }),
        vec![("cookie", org2_cookie.as_str())],
    )
    .await;
    assert!(
        !tool_is_error(&create_agent_resp),
        "org2 setup create_agent failed: {}",
        tool_text(&create_agent_resp)
    );

    let default_resources = mcp_call(
        &server,
        "resources/read",
        json!({ "uri": "everruns://agents" }),
    )
    .await;
    assert!(
        default_resources["error"].is_null(),
        "default resources/read failed: {default_resources}"
    );
    let default_text = default_resources["result"]["contents"][0]["text"]
        .as_str()
        .expect("default resource text");
    assert!(
        !default_text.contains(&marker),
        "default org resources/read leaked org2 marker: {default_text}"
    );

    let org2_resources = mcp_call_with_headers(
        &server,
        "resources/read",
        json!({ "uri": "everruns://agents" }),
        vec![("cookie", org2_cookie.as_str())],
    )
    .await;
    assert!(
        org2_resources["error"].is_null(),
        "org2 resources/read failed: {org2_resources}"
    );
    let org2_text = org2_resources["result"]["contents"][0]["text"]
        .as_str()
        .expect("org2 resource text");
    assert!(
        org2_text.contains(&marker),
        "org2 resources/read should see its own marker: {org2_text}"
    );

    // The agent template resolves names inside the caller's org only: another
    // org's agent is "not found", never readable.
    let agent_uri = format!("everruns://agents/{marker}");
    let cross_org = mcp_call(&server, "resources/read", json!({ "uri": agent_uri })).await;
    assert_eq!(cross_org["error"]["code"], -32002, "{cross_org}");
    assert_eq!(cross_org["error"]["data"]["uri"], agent_uri);

    let own_org = mcp_call_with_headers(
        &server,
        "resources/read",
        json!({ "uri": agent_uri }),
        vec![("cookie", org2_cookie.as_str())],
    )
    .await;
    assert!(own_org["error"].is_null(), "{own_org}");
    let own_text = own_org["result"]["contents"][0]["text"]
        .as_str()
        .expect("agent resource text");
    assert!(own_text.contains(&marker), "{own_text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_resource_templates_and_unknown_uri() {
    let server = TestServer::in_memory().await;

    let templates = mcp_call(&server, "resources/templates/list", json!({})).await;
    let uris: Vec<&str> = templates["result"]["resourceTemplates"]
        .as_array()
        .expect("resourceTemplates")
        .iter()
        .filter_map(|t| t["uriTemplate"].as_str())
        .collect();
    assert_eq!(
        uris,
        [
            "everruns://agents/{agent_id}",
            "everruns://sessions/{session_id}"
        ]
    );

    // A cursor is accepted; the catalog is one page.
    let listed = mcp_call(&server, "resources/list", json!({ "cursor": "x" })).await;
    assert!(listed["result"]["resources"].is_array(), "{listed}");
    assert!(listed["result"]["nextCursor"].is_null());

    let unknown = mcp_call(
        &server,
        "resources/read",
        json!({ "uri": "everruns://nothing" }),
    )
    .await;
    assert_eq!(unknown["error"]["code"], -32002, "{unknown}");
    assert_eq!(unknown["error"]["data"]["uri"], "everruns://nothing");
}
