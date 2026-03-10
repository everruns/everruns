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
    rt: tokio::runtime::Handle,
}

impl SandboxGuard {
    fn new(sandbox_id: String) -> Self {
        Self {
            sandbox_id,
            rt: tokio::runtime::Handle::current(),
        }
    }
}

impl Drop for SandboxGuard {
    fn drop(&mut self) {
        let id = self.sandbox_id.clone();
        let Some(api_key) = get_api_key() else {
            eprintln!("[cleanup] No API key, cannot delete sandbox {id}");
            return;
        };
        let client = DaytonaClient::new(api_key);
        self.rt.block_on(async {
            eprintln!("[cleanup] Deleting sandbox {id}");
            match client.delete_sandbox(&id).await {
                Ok(()) => eprintln!("[cleanup] Sandbox {id} deleted"),
                Err(e) => eprintln!("[cleanup] Failed to delete sandbox {id}: {e}"),
            }
        });
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn get_api_key() -> Option<String> {
    std::env::var("DAYTONA_API_KEY").ok().filter(|k| !k.is_empty())
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
            "image": "ubuntu:22.04",
            "resources": {"cpu": 1, "memory": 1},
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
    let result = client.exec(id, "echo hello-everruns", None, None).await;
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

/// Exec with working directory and nonzero exit code.
#[tokio::test]
async fn test_live_exec_cwd_and_exit_code() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "exec-cwd").await;
    let id = &guard.sandbox_id;

    // Exec with cwd
    let result = client
        .exec(id, "pwd", Some("/tmp"), None)
        .await
        .expect("exec with cwd failed");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.result.trim().ends_with("/tmp"),
        "Expected /tmp, got: {}",
        result.result
    );

    // Nonzero exit code
    let result = client
        .exec(id, "exit 42", None, None)
        .await
        .expect("exec with nonzero exit failed");
    assert_eq!(result.exit_code, 42);
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

/// Stop and start a sandbox.
#[tokio::test]
async fn test_live_stop_and_start() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "stop-start").await;
    let id = &guard.sandbox_id;

    // Stop
    client.stop_sandbox(id).await.expect("stop failed");

    let info = client.get_sandbox(id).await.expect("get after stop failed");
    assert!(
        info.state == "stopped" || info.state == "stopping",
        "Expected stopped/stopping, got: {}",
        info.state
    );

    // Start again
    client.start_sandbox(id).await.expect("start failed");
    client
        .wait_for_ready(id)
        .await
        .expect("sandbox did not become ready after restart");

    // Verify we can exec after restart
    let result = client
        .exec(id, "echo restarted", None, None)
        .await
        .expect("exec after restart failed");
    assert_eq!(result.exit_code, 0);
    assert!(result.result.contains("restarted"));
}
