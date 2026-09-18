use crate::support::*;
use everruns_core::SessionFile;
use everruns_platform::Agent;
use everruns_platform::Session;
use serde_json::{Value, json};

/// Test capability mounts are applied when session is created
///
/// This test verifies:
/// 1. Create agent with data_knowledge capability (which has a readonly mount)
/// 2. Create session for the agent
/// 3. Verify the /knowledge directory exists with expected scaffold files
/// 4. Verify files are read-only (from readonly mount)
#[tokio::test]
async fn test_capability_mounts_applied_on_session_creation() {
    let client = reqwest::Client::new();

    println!("Testing capability mounts applied on session creation...");

    // Step 1: Create an agent with data_knowledge capability
    println!("\nStep 1: Creating agent with data_knowledge capability...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "mount-test-agent",
            "display_name": "Mount Test Agent",
            "system_prompt": "Test agent for capability mounts",
            "capabilities": [
                {"ref": "data_knowledge", "config": {}},
                {"ref": "session_file_system", "config": {}}
            ]
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(
        agent_response.status(),
        201,
        "Expected 201 Created for agent"
    );

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with capabilities: {:?}",
        agent.public_id, agent.capabilities
    );
    assert_eq!(
        agent.capabilities.len(),
        2,
        "Agent should have 2 capabilities"
    );

    // Step 2: Create a session (this should trigger mount application)
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Mount Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    assert_eq!(session_response.status(), 201);
    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    let fs_url = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session.id);

    // Step 3: Verify /knowledge directory exists
    println!("\nStep 3: Verifying /knowledge directory exists...");
    let stat_response = client
        .post(format!("{}/_/stat", fs_url))
        .json(&json!({"path": "/knowledge"}))
        .send()
        .await
        .expect("Failed to stat /knowledge");

    assert_eq!(stat_response.status(), 200);
    let stat: Value = stat_response.json().await.expect("Failed to parse stat");
    assert_eq!(
        stat["is_directory"], true,
        "/knowledge should be a directory"
    );
    println!("/knowledge directory exists");

    // Step 4: List /knowledge directory contents
    println!("\nStep 4: Listing /knowledge directory...");
    let list_response = client
        .get(format!("{}/knowledge", fs_url))
        .send()
        .await
        .expect("Failed to list /knowledge");

    assert_eq!(list_response.status(), 200);
    let list_result: Value = list_response.json().await.expect("Failed to parse list");
    let files = list_result["data"].as_array().expect("Expected data array");
    println!("Found {} entries in /knowledge", files.len());

    // Should have index.md plus the tables/business/queries scaffold dirs
    let file_names: Vec<&str> = files.iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert!(
        file_names.contains(&"index.md"),
        "Expected index.md in /knowledge"
    );
    assert!(
        file_names.contains(&"tables"),
        "Expected tables/ in /knowledge"
    );
    assert!(
        file_names.contains(&"business"),
        "Expected business/ in /knowledge"
    );
    assert!(
        file_names.contains(&"queries"),
        "Expected queries/ in /knowledge"
    );
    println!("All expected entries present: {:?}", file_names);

    // Step 5: Verify index.md is readable
    println!("\nStep 5: Reading /knowledge/index.md...");
    let read_response = client
        .get(format!("{}/knowledge/index.md", fs_url))
        .send()
        .await
        .expect("Failed to read index.md");

    assert_eq!(read_response.status(), 200);
    let file: SessionFile = read_response.json().await.expect("Failed to parse file");
    assert!(file.content.is_some(), "File should have content");
    let content = file.content.as_ref().unwrap();
    assert!(
        content.contains("Data Knowledge"),
        "index.md should contain Data Knowledge"
    );
    assert!(file.is_readonly, "Mounted file should be readonly");
    println!("index.md is readable and readonly");

    // Step 6: Verify readonly protection - try to update index.md
    println!("\nStep 6: Verifying readonly protection...");
    let update_response = client
        .put(format!("{}/knowledge/index.md", fs_url))
        .json(&json!({
            "content": "modified content"
        }))
        .send()
        .await
        .expect("Failed to send update request");

    // Should fail because file is readonly
    assert_ne!(
        update_response.status(),
        200,
        "Readonly file update should fail"
    );
    println!("Readonly protection working - update rejected");

    // Step 7: Verify nested scaffold content
    println!("\nStep 7: Reading /knowledge/tables/index.md...");
    let tables_response = client
        .get(format!("{}/knowledge/tables/index.md", fs_url))
        .send()
        .await
        .expect("Failed to read tables/index.md");

    assert_eq!(tables_response.status(), 200);
    let tables_file: SessionFile = tables_response.json().await.expect("Failed to parse file");
    let tables_content = tables_file.content.as_ref().unwrap();
    assert!(
        tables_content.contains("Table Documentation"),
        "tables/index.md should contain Table Documentation"
    );
    println!("tables/index.md has expected scaffold content");

    // Step 8: Create a second session - verify mounts are independent
    println!("\nStep 8: Creating second session...");
    let session2_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Second Mount Test Session"
        }))
        .send()
        .await
        .expect("Failed to create second session");

    assert_eq!(session2_response.status(), 201);
    let session2: Session = session2_response
        .json()
        .await
        .expect("Failed to parse session2");
    println!("Created session2: {}", session2.id);

    // Verify second session also has mounts
    let fs_url2 = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session2.id);
    let stat2_response = client
        .post(format!("{}/_/stat", fs_url2))
        .json(&json!({"path": "/knowledge"}))
        .send()
        .await
        .expect("Failed to stat /knowledge in session2");

    assert_eq!(stat2_response.status(), 200);
    let stat2: Value = stat2_response.json().await.expect("Failed to parse stat2");
    assert_eq!(
        stat2["is_directory"], true,
        "session2 should also have /knowledge"
    );
    println!("Second session also has /knowledge directory mounted");

    // Cleanup
    println!("\nCleaning up...");
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session2.id),
        "session2",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;

    println!("Capability mounts test passed!");
}

/// Test MCP server CRUD operations
///
/// This test verifies:
/// 1. MCP server creation
/// 2. MCP server listing
/// 3. MCP server retrieval by ID
/// 4. MCP server update
/// 5. MCP server deletion
#[tokio::test]
async fn test_mcp_server_crud() {
    use everruns_core::McpServer;

    let client = reqwest::Client::new();

    println!("Testing MCP server CRUD operations...");

    // Step 1: Create an MCP server
    println!("\nStep 1: Creating MCP server...");
    let create_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "test-mcp-server",
            "description": "A test MCP server for integration testing",
            "url": "https://mcp.example.com/v1/mcp"
        }))
        .send()
        .await
        .expect("Failed to create MCP server");

    assert_eq!(
        create_response.status(),
        201,
        "Expected 201 Created, got {}",
        create_response.status()
    );

    let server: McpServer = create_response
        .json()
        .await
        .expect("Failed to parse MCP server response");

    println!("Created MCP server: {} ({})", server.name, server.id);
    assert_eq!(server.name, "test-mcp-server");
    assert_eq!(server.url, "https://mcp.example.com/v1/mcp");
    assert_eq!(server.status.to_string(), "active");
    assert!(!server.api_key_set, "API key should not be set");

    // Step 2: List MCP servers
    println!("\nStep 2: Listing MCP servers...");
    let list_response = client
        .get(format!("{}/v1/mcp-servers", API_BASE_URL))
        .send()
        .await
        .expect("Failed to list MCP servers");

    assert_eq!(list_response.status(), 200);
    let list_data: Value = list_response.json().await.expect("Failed to parse");
    let servers: Vec<McpServer> =
        serde_json::from_value(list_data["data"].clone()).expect("Failed to parse servers");
    println!("Found {} MCP server(s)", servers.len());
    assert!(!servers.is_empty());
    assert!(servers.iter().any(|s| s.id == server.id));

    // Step 3: Get MCP server by ID
    println!("\nStep 3: Getting MCP server by ID...");
    let get_response = client
        .get(format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id))
        .send()
        .await
        .expect("Failed to get MCP server");

    assert_eq!(get_response.status(), 200);
    let fetched_server: McpServer = get_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    println!("Fetched MCP server: {}", fetched_server.name);
    assert_eq!(fetched_server.id, server.id);
    assert_eq!(fetched_server.name, "test-mcp-server");

    // Step 4: Update MCP server
    println!("\nStep 4: Updating MCP server...");
    let update_response = client
        .patch(format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id))
        .json(&json!({
            "name": "updated-mcp-server",
            "description": "Updated description",
            "url": "https://mcp.updated.com/v1/mcp"
        }))
        .send()
        .await
        .expect("Failed to update MCP server");

    assert_eq!(update_response.status(), 200);
    let updated_server: McpServer = update_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    println!("Updated MCP server: {}", updated_server.name);
    assert_eq!(updated_server.name, "updated-mcp-server");
    assert_eq!(updated_server.url, "https://mcp.updated.com/v1/mcp");
    assert_eq!(
        updated_server.description,
        Some("Updated description".to_string())
    );

    // Step 5: Update MCP server status to disabled
    println!("\nStep 5: Disabling MCP server...");
    let disable_response = client
        .patch(format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id))
        .json(&json!({
            "status": "disabled"
        }))
        .send()
        .await
        .expect("Failed to disable MCP server");

    assert_eq!(disable_response.status(), 200);
    let disabled_server: McpServer = disable_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    assert_eq!(disabled_server.status.to_string(), "disabled");
    println!("MCP server disabled");

    // Step 6: Create MCP server with API key
    println!("\nStep 6: Creating MCP server with API key...");
    let create_with_key_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "mcp-server-with-key",
            "url": "https://secure.mcp.com/v1/mcp",
            "api_key": "test-api-key-12345"
        }))
        .send()
        .await
        .expect("Failed to create MCP server with API key");

    assert_eq!(create_with_key_response.status(), 201);
    let server_with_key: McpServer = create_with_key_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    assert!(server_with_key.api_key_set, "API key should be set");
    println!(
        "Created MCP server with API key: {} (api_key_set: {})",
        server_with_key.name, server_with_key.api_key_set
    );

    // Step 7: Create MCP server with custom headers
    println!("\nStep 7: Creating MCP server with custom headers...");
    let create_with_headers_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "mcp-server-with-headers",
            "url": "https://headers.mcp.com/v1/mcp",
            "headers": {
                "X-Custom-Header": "custom-value",
                "X-Another-Header": "another-value"
            }
        }))
        .send()
        .await
        .expect("Failed to create MCP server with headers");

    assert_eq!(create_with_headers_response.status(), 201);
    let server_with_headers: McpServer = create_with_headers_response
        .json()
        .await
        .expect("Failed to parse MCP server");
    assert_eq!(server_with_headers.headers.len(), 2);
    // Header names are present in API responses, but values are redacted to
    // prevent secret exposure. Verify the key exists and value is cleared.
    assert!(
        server_with_headers.headers.contains_key("X-Custom-Header"),
        "Header key must be present"
    );
    assert_eq!(
        server_with_headers
            .headers
            .get("X-Custom-Header")
            .map(String::as_str),
        Some(""),
        "Header values must be redacted in API responses"
    );
    println!(
        "Created MCP server with headers: {} ({} headers)",
        server_with_headers.name,
        server_with_headers.headers.len()
    );

    // Step 8: Test validation - empty name
    println!("\nStep 8: Testing validation - empty name...");
    let empty_name_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "",
            "url": "https://mcp.example.com/v1/mcp"
        }))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        empty_name_response.status(),
        400,
        "Expected 400 Bad Request for empty name"
    );
    println!("Empty name correctly rejected");

    // Step 9: Test validation - empty URL
    println!("\nStep 9: Testing validation - empty URL...");
    let empty_url_response = client
        .post(format!("{}/v1/mcp-servers", API_BASE_URL))
        .json(&json!({
            "name": "test-server",
            "url": ""
        }))
        .send()
        .await
        .expect("Failed to send request");

    assert_eq!(
        empty_url_response.status(),
        400,
        "Expected 400 Bad Request for empty URL"
    );
    println!("Empty URL correctly rejected");

    // Step 10: Test 404 for non-existent server
    println!("\nStep 10: Testing 404 for non-existent server...");
    let not_found_response = client
        .get(format!(
            "{}/v1/mcp-servers/mcp_00000000000000000000000000000000",
            API_BASE_URL
        ))
        .send()
        .await
        .expect("Failed to get non-existent server");

    assert_eq!(
        not_found_response.status(),
        404,
        "Expected 404 Not Found for non-existent server"
    );
    println!("Non-existent server correctly returns 404");

    // Cleanup
    println!("\nCleaning up...");
    cleanup_delete(
        &client,
        format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id),
        "MCP server",
    )
    .await;
    client
        .post(format!(
            "{}/v1/mcp-servers/{}/delete",
            API_BASE_URL, server.id
        ))
        .send()
        .await
        .expect("Failed to permanently delete MCP server");
    cleanup_delete(
        &client,
        format!("{}/v1/mcp-servers/{}", API_BASE_URL, server_with_key.id),
        "MCP server with key",
    )
    .await;
    client
        .post(format!(
            "{}/v1/mcp-servers/{}/delete",
            API_BASE_URL, server_with_key.id
        ))
        .send()
        .await
        .expect("Failed to permanently delete MCP server with key");
    cleanup_delete(
        &client,
        format!("{}/v1/mcp-servers/{}", API_BASE_URL, server_with_headers.id),
        "MCP server with headers",
    )
    .await;
    client
        .post(format!(
            "{}/v1/mcp-servers/{}/delete",
            API_BASE_URL, server_with_headers.id
        ))
        .send()
        .await
        .expect("Failed to permanently delete MCP server with headers");

    // Verify deletion
    let verify_deleted = client
        .get(format!("{}/v1/mcp-servers/{}", API_BASE_URL, server.id))
        .send()
        .await
        .expect("Failed to verify deletion");
    assert_eq!(verify_deleted.status(), 404);

    println!("MCP server CRUD test passed!");
}

// =============================================================================
// Agent Execution Tests with Tool Calls
// =============================================================================
// These tests verify end-to-end agent execution with tool calls for each provider.
// The "dad jokes agent" pattern: ask for current time, use it in a joke.
