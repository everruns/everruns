//! Live Daytona API integration tests.
//!
//! These tests hit the real Daytona API and are gated behind:
//! - Feature flag: `daytona-live-tests`
//! - Environment variable: `DAYTONA_API_KEY`
//!
//! Run locally:
//!   DAYTONA_API_KEY=<key> cargo test -p everruns-integrations-daytona \
//!       --features daytona-live-tests --test live_api_test -- --test-threads=1
//!
//! Cleanup guarantee: Each test uses a `SandboxGuard` that deletes the sandbox
//! on drop (both success and panic paths).

#![cfg(feature = "daytona-live-tests")]

use everruns_integrations_daytona::client::DaytonaClient;
use serde_json::json;

// ============================================================================
// SandboxGuard — RAII cleanup for Daytona sandboxes
// ============================================================================

/// Ensures sandbox deletion on drop, even if the test panics.
struct SandboxGuard {
    sandbox_id: String,
}

impl SandboxGuard {
    fn new(sandbox_id: String) -> Self {
        Self { sandbox_id }
    }
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        let id = self.sandbox_id.clone();
        let Some(api_key) = get_api_key() else {
            eprintln!("[cleanup] No API key, cannot delete sandbox {id}");
            return;
        };
        // Spawn a blocking thread for cleanup — block_on panics if called
        // during unwind (double panic → abort), so use a dedicated thread.
        let handle = std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("cleanup runtime");
            let client = DaytonaClient::new(api_key);
            rt.block_on(async {
                eprintln!("[cleanup] Deleting sandbox {id}");
                match client.delete_sandbox(&id).await {
                    Ok(()) => eprintln!("[cleanup] Sandbox {id} deleted"),
                    Err(e) => eprintln!("[cleanup] Failed to delete sandbox {id}: {e}"),
                }
            });
        });
        let _ = handle.join();
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn get_api_key() -> Option<String> {
    std::env::var("DAYTONA_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

/// Skip test if DAYTONA_API_KEY is not set. Returns the key.
macro_rules! require_api_key {
    () => {
        match get_api_key() {
            Some(key) => key,
            None => {
                eprintln!("[skip] DAYTONA_API_KEY not set, skipping live test");
                return;
            }
        }
    };
}

/// Create a sandbox with a unique test label and return client + guard.
async fn create_test_sandbox(api_key: String, label: &str) -> (DaytonaClient, SandboxGuard) {
    let client = DaytonaClient::new(api_key);

    let info = client
        .create_sandbox(json!({
            "snapshot": "daytona-small",
            "labels": {"everruns-test": label}
        }))
        .await
        .expect("Failed to create sandbox");

    assert!(!info.id.is_empty(), "Sandbox ID should not be empty");
    eprintln!("[test] Created sandbox {} (label: {label})", info.id);

    client
        .set_autostop(&info.id, 5)
        .await
        .expect("Failed to set autostop");

    client
        .wait_for_ready(&info.id)
        .await
        .expect("Sandbox did not become ready");

    let guard = SandboxGuard::new(info.id);
    (client, guard)
}

// ============================================================================
// Tests
// ============================================================================

/// Full lifecycle: create → exec → file roundtrip → delete.
#[tokio::test]
async fn test_live_sandbox_lifecycle() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "lifecycle").await;
    let id = &guard.sandbox_id;

    // Exec: simple echo
    let result = client
        .exec(id, "echo hello-everruns", None, None, |_| {})
        .await;
    let exec = result.expect("exec failed");
    assert_eq!(exec.exit_code, 0);
    assert!(
        exec.result.contains("hello-everruns"),
        "Unexpected output: {}",
        exec.result
    );

    // File write + read roundtrip
    let content = b"print('hello from everruns live test')\n";
    client
        .file_upload(id, "/tmp/test_live.py", content)
        .await
        .expect("file upload failed");

    let downloaded = client
        .file_download(id, "/tmp/test_live.py")
        .await
        .expect("file download failed");
    assert_eq!(
        downloaded, content,
        "Downloaded content doesn't match uploaded"
    );

    // Verify sandbox status is "started"
    let info = client.get_sandbox(id).await.expect("get_sandbox failed");
    assert_eq!(info.state, "started");

    // Explicit delete (guard will also try, but double-delete should be harmless)
    client
        .delete_sandbox(id)
        .await
        .expect("delete_sandbox failed");
}

/// Exec with working directory, nonzero exit code, and session death recovery.
#[tokio::test]
async fn test_live_exec_cwd_and_exit_code() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "exec-cwd").await;
    let id = &guard.sandbox_id;

    // Exec with cwd
    let result = client
        .exec(id, "pwd", Some("/tmp"), None, |_| {})
        .await
        .expect("exec with cwd failed");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.result.trim().ends_with("/tmp"),
        "Expected /tmp, got: {}",
        result.result
    );

    // Nonzero exit code — use `sh -c` to run in a subshell so the
    // persistent session shell is not killed by `exit`.
    let result = client
        .exec(id, "sh -c 'exit 42'", None, None, |_| {})
        .await
        .expect("exec with nonzero exit failed");
    assert_ne!(result.exit_code, 0, "Expected nonzero exit code, got 0");

    // Bare `exit` kills the persistent session shell. The client must
    // detect the dead session and return an error instead of hanging.
    let result = client.exec(id, "exit 1", None, None, |_| {}).await;
    assert!(
        result.is_err(),
        "bare `exit` should be detected as session termination"
    );

    // Subsequent exec should recover — ensure_session re-creates it.
    let result = client
        .exec(id, "echo recovered", None, None, |_| {})
        .await
        .expect("exec after session recovery failed");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.result.contains("recovered"),
        "Expected 'recovered' in output: {}",
        result.result
    );
}

/// Folder creation and file listing.
#[tokio::test]
async fn test_live_folder_and_list() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "folder-list").await;
    let id = &guard.sandbox_id;

    // Create folder
    client
        .create_folder(id, "/tmp/test_dir", "755")
        .await
        .expect("create_folder failed");

    // Write a file inside
    client
        .file_upload(id, "/tmp/test_dir/hello.txt", b"world")
        .await
        .expect("file_upload failed");

    // List files
    let entries = client
        .file_list(id, "/tmp/test_dir")
        .await
        .expect("file_list failed");

    let names: Vec<&str> = entries.iter().filter_map(|e| e["name"].as_str()).collect();
    assert!(
        names.contains(&"hello.txt"),
        "Expected hello.txt in listing, got: {names:?}"
    );

    // Delete file
    client
        .file_delete(id, "/tmp/test_dir/hello.txt")
        .await
        .expect("file_delete failed");

    // Verify deleted
    let entries_after = client
        .file_list(id, "/tmp/test_dir")
        .await
        .expect("file_list after delete failed");
    let names_after: Vec<&str> = entries_after
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(
        !names_after.contains(&"hello.txt"),
        "hello.txt should be deleted"
    );
}

/// exec_streaming: shell redirections (`2>/dev/null`) must not leak as literal filenames (EVE-185).
#[tokio::test]
async fn test_live_exec_streaming_returns_output() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "exec-streaming").await;
    let id = &guard.sandbox_id;

    let mut chunks = Vec::new();
    let result = client
        .exec(id, "echo hello-streaming", None, Some(30_000), |chunk| {
            chunks.push(chunk.to_string());
        })
        .await
        .expect("exec_streaming failed");

    assert_eq!(result.exit_code, 0, "Expected exit code 0");
    assert!(
        result.result.contains("hello-streaming"),
        "Full output missing marker: {}",
        result.result
    );
    assert!(
        !chunks.is_empty(),
        "Expected at least one output chunk from streaming callback"
    );
}

/// Shell profile sourcing: tools installed mid-session are visible in PATH.
///
/// Simulates the real-world scenario where an agent installs a tool (e.g. rustup)
/// and the next command needs it in PATH. Without profile sourcing, every command
/// after install would need a manual `. ~/.cargo/env &&` prefix.
#[tokio::test]
async fn test_live_shell_profile_sourcing() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "profile-sourcing").await;
    let id = &guard.sandbox_id;

    // 1. Write a fake profile file that exports a custom PATH entry and variable.
    //    This simulates what rustup/nvm do when installed.
    //    mkdir -p ~/.cargo first — Daytona sandboxes don't have it by default.
    let result = client
        .exec(
            id,
            r#"mkdir -p /tmp/fake-tool ~/.cargo && printf '#!/bin/sh\necho fake-tool-output\n' > /tmp/fake-tool/my-tool && chmod +x /tmp/fake-tool/my-tool && printf 'export PATH="/tmp/fake-tool:$PATH"\nexport FAKE_TOOL_VERSION="1.0.0"\n' > ~/.cargo/env"#,
            None,
            None,
            |_| {},
        )
        .await
        .expect("setup fake tool failed");
    assert_eq!(result.exit_code, 0, "setup failed: {}", result.result);

    // 2. In a subsequent exec, the tool should be in PATH because the preamble
    //    sources ~/.cargo/env before running the user command.
    let result = client
        .exec(id, "my-tool", None, None, |_| {})
        .await
        .expect("my-tool exec failed");
    assert_eq!(
        result.exit_code, 0,
        "my-tool not found in PATH — profile sourcing broken. Output: {}",
        result.result
    );
    assert!(
        result.result.contains("fake-tool-output"),
        "Expected fake-tool-output, got: {}",
        result.result
    );

    // 3. Verify the env var exported by the profile is also available.
    let result = client
        .exec(id, "echo $FAKE_TOOL_VERSION", None, None, |_| {})
        .await
        .expect("env var check failed");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.result.contains("1.0.0"),
        "Expected FAKE_TOOL_VERSION=1.0.0, got: {}",
        result.result
    );
}

/// Shell profile sourcing with .profile: env vars exported in .profile are available.
#[tokio::test]
async fn test_live_profile_sourcing() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "profile-sourcing-dotprofile").await;
    let id = &guard.sandbox_id;

    // Write a .profile that exports a custom variable (backup any existing one).
    let result = client
        .exec(
            id,
            r#"[ -f ~/.profile ] && cp ~/.profile ~/.profile.bak; echo 'export FROM_PROFILE="yes"' >> ~/.profile"#,
            None,
            None,
            |_| {},
        )
        .await
        .expect("profile setup failed");
    assert_eq!(result.exit_code, 0);

    // Subsequent exec should have the variable available via preamble sourcing.
    let result = client
        .exec(id, "echo FROM_PROFILE=$FROM_PROFILE", None, None, |_| {})
        .await
        .expect("profile check exec failed");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.result.contains("FROM_PROFILE=yes"),
        "Expected FROM_PROFILE=yes, got: {}",
        result.result
    );

    // Restore .profile.
    let _ = client
        .exec(
            id,
            "[ -f ~/.profile.bak ] && mv ~/.profile.bak ~/.profile || true",
            None,
            None,
            |_| {},
        )
        .await;
}

/// Stop and start a sandbox.
#[tokio::test]
async fn test_live_stop_and_start() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "stop-start").await;
    let id = &guard.sandbox_id;

    // Stop and wait for it to fully stop
    client.stop_sandbox(id).await.expect("stop failed");

    for _ in 0..30 {
        let info = client.get_sandbox(id).await.expect("get after stop failed");
        if info.state == "stopped" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }

    let info = client.get_sandbox(id).await.expect("get after stop failed");
    assert_eq!(info.state, "stopped", "Sandbox did not stop in time");

    // Start again
    client.start_sandbox(id).await.expect("start failed");
    client
        .wait_for_ready(id)
        .await
        .expect("sandbox did not become ready after restart");

    // Verify we can exec after restart
    let result = client
        .exec(id, "echo restarted", None, None, |_| {})
        .await
        .expect("exec after restart failed");
    assert_eq!(result.exit_code, 0);
    assert!(result.result.contains("restarted"));
}

/// api_call: create sandbox via raw API, verify labels are set, exec via toolbox, delete.
#[tokio::test]
async fn test_live_api_call_sandbox_lifecycle_with_labels() {
    let api_key = require_api_key!();
    let client = DaytonaClient::new(api_key);

    // Create sandbox via api_call with labels
    let create_body = json!({
        "snapshot": "daytona-small",
        "autoStopInterval": 5,
        "autoArchiveInterval": 30,
        "autoDeleteInterval": 60,
        "labels": {
            "everruns": "true",
            "everruns.test": "api_call_lifecycle"
        }
    });
    let response = client
        .api_call(reqwest::Method::POST, "/sandbox", Some(create_body))
        .await
        .expect("api_call POST /sandbox failed");

    let sandbox_id = response["id"].as_str().expect("No sandbox ID in response");
    assert!(!sandbox_id.is_empty());
    eprintln!("[test] Created sandbox via api_call: {sandbox_id}");
    let guard = SandboxGuard::new(sandbox_id.to_string());

    // Wait for ready
    client
        .wait_for_ready(sandbox_id)
        .await
        .expect("Sandbox did not become ready");

    // GET sandbox details via api_call, verify labels survived creation
    let details = client
        .api_call(
            reqwest::Method::GET,
            &format!("/sandbox/{sandbox_id}"),
            None,
        )
        .await
        .expect("api_call GET /sandbox/{id} failed");

    assert_eq!(details["id"].as_str().unwrap(), sandbox_id);
    let labels = &details["labels"];
    assert_eq!(
        labels["everruns"].as_str().unwrap_or(""),
        "true",
        "everruns label missing or wrong: {labels}"
    );
    assert_eq!(
        labels["everruns.test"].as_str().unwrap_or(""),
        "api_call_lifecycle",
        "test label missing: {labels}"
    );
    eprintln!("[test] Labels verified: {labels}");

    // Execute command via toolbox api_call
    let exec_result = client
        .api_call(
            reqwest::Method::POST,
            &format!("/toolbox/{sandbox_id}/process/execute"),
            Some(json!({"command": "echo api-call-works"})),
        )
        .await
        .expect("toolbox exec via api_call failed");
    assert!(
        exec_result["result"]
            .as_str()
            .unwrap_or("")
            .contains("api-call-works"),
        "Exec output missing marker: {exec_result}"
    );
    eprintln!("[test] Toolbox exec works via api_call");

    // List files via toolbox api_call
    let files = client
        .api_call(
            reqwest::Method::GET,
            &format!("/toolbox/{sandbox_id}/files?path=/tmp"),
            None,
        )
        .await
        .expect("toolbox file list via api_call failed");
    assert!(files.is_array(), "Expected array of files, got: {files}");
    eprintln!("[test] Toolbox file listing works via api_call");

    // List all sandboxes via api_call, verify ours appears
    let all = client
        .api_call(reqwest::Method::GET, "/sandbox", None)
        .await
        .expect("api_call GET /sandbox failed");
    let found = all
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s["id"].as_str() == Some(sandbox_id));
    assert!(found, "Sandbox {sandbox_id} not found in list");

    // Delete via api_call
    let del = client
        .api_call(
            reqwest::Method::DELETE,
            &format!("/sandbox/{sandbox_id}"),
            None,
        )
        .await
        .expect("api_call DELETE /sandbox/{id} failed");
    assert!(del.is_object());
    eprintln!("[test] Deleted sandbox via api_call: {sandbox_id}");

    // Guard will also try to delete (harmless double-delete)
    drop(guard);
}
