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
