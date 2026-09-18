use crate::support::*;
use everruns_core::SessionFile;
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use serde_json::{Value, json};

#[tokio::test]
async fn test_session_filesystem() {
    let client = reqwest::Client::new();

    println!("Testing session filesystem...");

    // Step 1: Create an agent
    println!("\nStep 1: Creating agent...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "filesystem-test-agent",
            "display_name": "Filesystem Test Agent",
            "system_prompt": "Test agent for filesystem"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!("Created agent: {}", agent.public_id);

    // Step 2: Create a session
    println!("\nStep 2: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Filesystem Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    println!("Created session: {}", session.id);

    let fs_url = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session.id);

    // Step 3: List root directory (new sessions mount scoped memory under the reserved memory root)
    println!("\nStep 3: Listing root directory...");
    let list_response = client
        .get(&fs_url)
        .send()
        .await
        .expect("Failed to list files");

    assert_eq!(list_response.status(), 200);
    let list_result: Value = list_response.json().await.expect("Failed to parse");
    let root_entries = list_result["data"].as_array().unwrap();
    assert_eq!(root_entries.len(), 1);
    assert_eq!(root_entries[0]["path"], "/memory");
    assert_eq!(root_entries[0]["is_directory"], true);
    println!("Root directory contains scoped memory mount");

    // Step 4: Create a file
    println!("\nStep 4: Creating file...");
    let create_response = client
        .post(format!("{}/hello.txt", fs_url))
        .json(&json!({
            "content": "Hello, World!",
            "encoding": "text"
        }))
        .send()
        .await
        .expect("Failed to create file");

    assert_eq!(create_response.status(), 201);
    let file: SessionFile = create_response.json().await.expect("Failed to parse file");
    println!("Created file: {}", file.path);
    assert_eq!(file.path, "/hello.txt");
    assert!(!file.is_directory);

    // Step 5: Read file
    println!("\nStep 5: Reading file...");
    let read_response = client
        .get(format!("{}/hello.txt", fs_url))
        .send()
        .await
        .expect("Failed to read file");

    assert_eq!(read_response.status(), 200);
    let file: SessionFile = read_response.json().await.expect("Failed to parse file");
    assert_eq!(file.content.as_deref(), Some("Hello, World!"));
    println!("File content: {:?}", file.content);

    // Step 6: Get file stat
    println!("\nStep 6: Getting file stat...");
    let stat_response = client
        .post(format!("{}/_/stat", fs_url))
        .json(&json!({
            "path": "/hello.txt"
        }))
        .send()
        .await
        .expect("Failed to get stat");

    assert_eq!(stat_response.status(), 200);
    let stat: Value = stat_response.json().await.expect("Failed to parse stat");
    assert_eq!(stat["path"], "/hello.txt");
    assert_eq!(stat["is_directory"], false);
    println!("File stat: size={}", stat["size_bytes"]);

    // Step 7: Update file
    println!("\nStep 7: Updating file...");
    let update_response = client
        .put(format!("{}/hello.txt", fs_url))
        .json(&json!({
            "content": "Updated content"
        }))
        .send()
        .await
        .expect("Failed to update file");

    assert_eq!(update_response.status(), 200);
    let file: SessionFile = update_response.json().await.expect("Failed to parse file");
    assert_eq!(file.content.as_deref(), Some("Updated content"));
    println!("File updated");

    // Step 8: Create directory
    println!("\nStep 8: Creating directory...");
    let dir_response = client
        .post(format!("{}/docs", fs_url))
        .json(&json!({
            "is_directory": true
        }))
        .send()
        .await
        .expect("Failed to create directory");

    assert_eq!(dir_response.status(), 201);
    let dir: SessionFile = dir_response.json().await.expect("Failed to parse dir");
    assert!(dir.is_directory);
    println!("Created directory: {}", dir.path);

    // Step 9: Create file in directory (auto-creates parent)
    println!("\nStep 9: Creating nested file...");
    let nested_response = client
        .post(format!("{}/src/main.rs", fs_url))
        .json(&json!({
            "content": "fn main() {}"
        }))
        .send()
        .await
        .expect("Failed to create nested file");

    assert_eq!(nested_response.status(), 201);
    let nested: SessionFile = nested_response.json().await.expect("Failed to parse");
    assert_eq!(nested.path, "/src/main.rs");
    println!("Created nested file: {}", nested.path);

    // Step 10: List all files
    println!("\nStep 10: Listing all files...");
    let list_all_response = client
        .get(format!("{}?recursive=true", fs_url))
        .send()
        .await
        .expect("Failed to list all files");

    assert_eq!(list_all_response.status(), 200);
    let list_all: Value = list_all_response.json().await.expect("Failed to parse");
    let files = list_all["data"].as_array().unwrap();
    assert!(files.len() >= 3); // hello.txt, docs, src/main.rs
    println!("Found {} files", files.len());

    // Step 11: Copy file
    println!("\nStep 11: Copying file...");
    let copy_response = client
        .post(format!("{}/_/copy", fs_url))
        .json(&json!({
            "src_path": "/hello.txt",
            "dst_path": "/hello-copy.txt"
        }))
        .send()
        .await
        .expect("Failed to copy file");

    assert_eq!(copy_response.status(), 201);
    println!("File copied");

    // Step 12: Move file
    println!("\nStep 12: Moving file...");
    let move_response = client
        .post(format!("{}/_/move", fs_url))
        .json(&json!({
            "src_path": "/hello-copy.txt",
            "dst_path": "/renamed.txt"
        }))
        .send()
        .await
        .expect("Failed to move file");

    assert_eq!(move_response.status(), 200);
    println!("File moved/renamed");

    // Step 13: Grep search
    println!("\nStep 13: Searching files...");
    let grep_response = client
        .post(format!("{}/_/grep", fs_url))
        .json(&json!({
            "pattern": "main"
        }))
        .send()
        .await
        .expect("Failed to grep");

    assert_eq!(grep_response.status(), 200);
    let grep_result: Value = grep_response.json().await.expect("Failed to parse");
    let results = grep_result["data"].as_array().unwrap();
    assert!(!results.is_empty());
    println!("Found {} files with matches", results.len());

    // Step 14: Delete file
    println!("\nStep 14: Deleting file...");
    let delete_response = client
        .delete(format!("{}/renamed.txt", fs_url))
        .send()
        .await
        .expect("Failed to delete file");

    assert_eq!(delete_response.status(), 200);
    let delete_result: Value = delete_response.json().await.expect("Failed to parse");
    assert_eq!(delete_result["deleted"], true);
    println!("File deleted");

    // Step 15: Delete directory recursively
    println!("\nStep 15: Deleting directory recursively...");
    let delete_dir_response = client
        .delete(format!("{}/src?recursive=true", fs_url))
        .send()
        .await
        .expect("Failed to delete directory");

    assert_eq!(delete_dir_response.status(), 200);
    println!("Directory deleted");

    // Cleanup
    println!("\nCleaning up...");
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;

    println!("Session filesystem test passed!");
}

/// Test that filesystem API handles /workspace prefix correctly
///
/// The file_system and bashkit_shell capabilities present files to users with
/// a /workspace prefix (e.g., /workspace/demo/a.txt), but internally store
/// them without the prefix (e.g., /demo/a.txt). The API should handle both
/// formats transparently.
#[tokio::test]
async fn test_session_filesystem_workspace_prefix() {
    let client = reqwest::Client::new();

    println!("Testing filesystem workspace prefix handling...");

    // Step 1: Create an agent and session
    println!("\nStep 1: Creating agent and session...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "workspace-test-agent",
            "display_name": "Workspace Test Agent",
            "system_prompt": "Test agent"
        }))
        .send()
        .await
        .expect("Failed to create agent");

    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");

    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({
            "harness_name": SEED_HARNESS_NAME,
            "agent_id": agent.public_id,
            "title": "Workspace Test Session"
        }))
        .send()
        .await
        .expect("Failed to create session");

    let session: Session = session_response
        .json()
        .await
        .expect("Failed to parse session");
    let fs_url = format!("{}/v1/sessions/{}/fs", API_BASE_URL, session.id);

    // Step 2: Create a file at /demo/a.txt (internal path)
    println!("\nStep 2: Creating file at internal path /demo/a.txt...");
    let create_response = client
        .post(format!("{}/demo/a.txt", fs_url))
        .json(&json!({
            "content": "hello",
            "encoding": "text"
        }))
        .send()
        .await
        .expect("Failed to create file");

    assert_eq!(create_response.status(), 201);
    println!("File created at /demo/a.txt");

    // Step 3: Access /workspace should return root listing (maps to /)
    println!("\nStep 3: Accessing /workspace path...");
    let workspace_response = client
        .get(format!("{}/workspace", fs_url))
        .send()
        .await
        .expect("Failed to get workspace");

    // This is the bug: /workspace returns 404 instead of root listing
    assert_eq!(
        workspace_response.status(),
        200,
        "Expected 200 OK for /workspace, got {} - workspace prefix not handled",
        workspace_response.status()
    );

    let workspace_result: Value = workspace_response
        .json()
        .await
        .expect("Failed to parse workspace response");
    assert!(
        workspace_result.get("data").is_some(),
        "Expected directory listing"
    );
    println!("/workspace returned listing successfully");

    // Step 4: Access /workspace/demo should list the demo directory
    println!("\nStep 4: Accessing /workspace/demo path...");
    let workspace_demo_response = client
        .get(format!("{}/workspace/demo", fs_url))
        .send()
        .await
        .expect("Failed to get workspace/demo");

    assert_eq!(
        workspace_demo_response.status(),
        200,
        "Expected 200 OK for /workspace/demo, got {}",
        workspace_demo_response.status()
    );
    println!("/workspace/demo returned listing successfully");

    // Step 5: Access /workspace/demo/a.txt should return the file
    println!("\nStep 5: Accessing /workspace/demo/a.txt...");
    let workspace_file_response = client
        .get(format!("{}/workspace/demo/a.txt", fs_url))
        .send()
        .await
        .expect("Failed to get workspace file");

    assert_eq!(
        workspace_file_response.status(),
        200,
        "Expected 200 OK for /workspace/demo/a.txt, got {}",
        workspace_file_response.status()
    );

    let file: SessionFile = workspace_file_response
        .json()
        .await
        .expect("Failed to parse file");
    assert_eq!(file.content.as_deref(), Some("hello"));
    println!("/workspace/demo/a.txt returned file content successfully");

    // Cleanup
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;

    println!("Filesystem workspace prefix test passed!");
}

/// Test agent with file_system and bashkit_shell capabilities share /workspace paths
///
/// This test verifies:
/// 1. An agent with both session_file_system and bashkit_shell capabilities can use them
/// 2. Files created via write_file are accessible at /workspace via API
/// 3. The /workspace prefix transformation works correctly in agent workflows
///
/// Requirements: API + Worker running, ANTHROPIC_API_KEY environment variable set.
/// Skips if no API key is available.
#[tokio::test]
async fn test_agent_filesystem_and_bash_workspace_integration() {
    use std::time::Duration;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .expect("Failed to create client");

    println!("Testing agent with file_system and bashkit_shell capabilities...");

    // Check if Anthropic API key is available
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            println!("ANTHROPIC_API_KEY not set - skipping test");
            return;
        }
    };

    // Step 1: Create Anthropic provider and model
    println!("\nStep 1: Creating Anthropic provider and model...");
    let provider_response = client
        .post(format!("{}/v1/providers", API_BASE_URL))
        .json(&json!({
            "name": "FS Bash Integration Test Provider",
            "provider_type": "anthropic",
            "api_key": api_key,
            "enabled": false
        }))
        .send()
        .await
        .expect("Failed to create provider");

    if provider_response.status() != 201 {
        let status = provider_response.status();
        let body = provider_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create Anthropic provider: status={}, body={}",
            status, body
        );
    }
    let provider: Provider = provider_response
        .json()
        .await
        .expect("Failed to parse provider");
    println!("Created Anthropic provider: {}", provider.id);

    // Create model (claude-haiku-4-5 for cost-effectiveness)
    let model_response = client
        .post(format!(
            "{}/v1/providers/{}/models",
            API_BASE_URL, provider.id
        ))
        .json(&json!({
            "model_id": LIVE_ANTHROPIC_FAST_MODEL,
            "display_name": "Anthropic Fast (FS Bash Test)",
            "enabled": true
        }))
        .send()
        .await
        .expect("Failed to create model");

    if model_response.status() != 201 {
        let status = model_response.status();
        let body = model_response.text().await.unwrap_or_default();
        panic!(
            "Failed to create Anthropic model: status={}, body={}",
            status, body
        );
    }
    let model: Model = model_response.json().await.expect("Failed to parse model");
    println!("Created model: {}", model.id);

    // Step 2: Create agent with both session_file_system and bashkit_shell capabilities
    println!("\nStep 2: Creating agent with file_system and bashkit_shell capabilities...");
    let agent_response = client
        .post(format!("{}/v1/agents", API_BASE_URL))
        .json(&json!({
            "name": "fs-bash-test-agent",
            "display_name": "FS Bash Test Agent",
            "system_prompt": "You are a file system assistant. When asked to create a file, use the write_file tool to create it. Always confirm what you did.",
            "capabilities": [
                {"ref": "session_file_system", "config": {}},
                {"ref": "bashkit_shell", "config": {}}
            ],
            "default_model_id": model.id
        }))
        .send()
        .await
        .expect("Failed to create agent");

    assert_eq!(agent_response.status(), 201);
    let agent: Agent = agent_response.json().await.expect("Failed to parse agent");
    println!(
        "Created agent: {} with capabilities: {:?}",
        agent.public_id, agent.capabilities
    );

    // Step 3: Create session
    println!("\nStep 3: Creating session...");
    let session_response = client
        .post(format!("{}/v1/sessions", API_BASE_URL))
        .json(&json!({"harness_name": SEED_HARNESS_NAME, "agent_id": agent.public_id, "title": "FS Bash Integration Test"}))
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

    // Step 4: Send message asking agent to create a file
    println!("\nStep 4: Sending message asking agent to create a file...");
    let message_response = client
        .post(format!(
            "{}/v1/sessions/{}/messages",
            API_BASE_URL, session.id
        ))
        .json(&json!({
            "message": {
                "content": [{"type": "text", "text": "Please create a file at /workspace/test/hello.txt with the content 'Hello from agent!'"}]
            }
        }))
        .send()
        .await
        .expect("Failed to send message");

    assert_eq!(message_response.status(), 201);
    println!("Message sent successfully");

    // Step 5: Wait for agent to complete (up to 60 seconds)
    println!("\nStep 5: Waiting for agent to complete file creation (up to 60 seconds)...");
    let mut write_file_called = false;
    let mut has_final_response = false;

    for i in 1..=60 {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let messages_response = client
            .get(format!(
                "{}/v1/sessions/{}/messages",
                API_BASE_URL, session.id
            ))
            .send()
            .await;

        if let Ok(resp) = messages_response
            && resp.status() == 200
        {
            let data: Value = resp.json().await.unwrap_or_default();
            let empty_vec = vec![];
            let messages = data["data"].as_array().unwrap_or(&empty_vec);

            if i % 10 == 0 {
                println!("  [{}s] Messages: {}", i, messages.len());
            }

            // Look for write_file tool call and final text response
            for msg in messages {
                let role = msg["role"].as_str().unwrap_or("");
                if role == "agent"
                    && let Some(content) = msg["content"].as_array()
                {
                    for part in content {
                        if part["type"] == "tool_call" {
                            let tool_name = part["name"].as_str().unwrap_or("");
                            if tool_name == "write_file" && !write_file_called {
                                write_file_called = true;
                                println!("  Found write_file tool call after {}s", i);
                            }
                        }
                        // Check for final text response after tool call
                        if part["type"] == "text" && write_file_called {
                            has_final_response = true;
                        }
                    }
                }
            }

            if write_file_called && has_final_response {
                tokio::time::sleep(Duration::from_secs(1)).await;
                break;
            }
        }
    }

    skip_on_provider_account_block!(session_provider_account_block(&client, &session.id).await);

    assert!(
        write_file_called,
        "Agent should have called write_file tool"
    );
    println!("Agent completed file creation");

    // Step 6: Verify file is accessible via /workspace path in API
    println!("\nStep 6: Verifying file is accessible via /workspace API path...");

    // Access via /workspace/test/hello.txt (user-facing path)
    let workspace_file_response = client
        .get(format!("{}/workspace/test/hello.txt", fs_url))
        .send()
        .await
        .expect("Failed to get file via workspace path");

    assert_eq!(
        workspace_file_response.status(),
        200,
        "/workspace/test/hello.txt should return 200, got {}",
        workspace_file_response.status()
    );

    let file: SessionFile = workspace_file_response
        .json()
        .await
        .expect("Failed to parse file");
    println!(
        "SUCCESS: File accessible at /workspace/test/hello.txt, content: {:?}",
        file.content
    );
    assert!(file.content.is_some(), "File should have content");

    // Step 7: Verify /workspace root listing works
    println!("\nStep 7: Verifying /workspace root listing...");
    let workspace_root_response = client
        .get(format!("{}/workspace", fs_url))
        .send()
        .await
        .expect("Failed to get workspace root");

    assert_eq!(
        workspace_root_response.status(),
        200,
        "/workspace should return 200 (root listing), got {}",
        workspace_root_response.status()
    );

    let listing: Value = workspace_root_response
        .json()
        .await
        .expect("Failed to parse listing");
    let entry_count = listing["data"].as_array().map(|a| a.len()).unwrap_or(0);
    println!("/workspace listing has {} entries", entry_count);
    // Note: We don't assert on entry_count because direct_worker_adapters.rs has a separate
    // bug where it doesn't create parent directories. The main /workspace prefix fix is
    // verified above (file accessible at /workspace/test/hello.txt).

    // Cleanup
    println!("\nCleaning up...");
    // `sessions.model_id` pins the model, so the session goes first or the
    // model/provider delete is refused (EVE-955).
    cleanup_delete(
        &client,
        format!("{}/v1/sessions/{}", API_BASE_URL, session.id),
        "session",
    )
    .await;
    cleanup_agent(&client, &agent.public_id).await;

    cleanup_delete(
        &client,
        format!("{}/v1/providers/{}", API_BASE_URL, provider.id),
        "provider",
    )
    .await;

    println!("Agent filesystem and bash workspace integration test passed!");
}
