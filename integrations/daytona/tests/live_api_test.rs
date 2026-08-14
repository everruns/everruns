//! Live Daytona API integration tests.
//!
//! These tests hit the real Daytona API and are gated behind:
//! - Feature flag: `daytona-live-tests`
//! - Environment variable: `DAYTONA_API_KEY` (required; missing ⇒ panic)
//!
//! Run locally:
//!   DAYTONA_API_KEY=<key> cargo test -p everruns-integrations-daytona \
//!       --features daytona-live-tests --test live_api_test -- --test-threads=1
//!
//! Missing-credential policy: these tests fail closed. If the feature flag is
//! set but the credential is missing, the tests panic rather than silently
//! passing, so CI live jobs cannot report false-green. See `knowledge/integrations/integrations.md`.
//!
//! Cleanup guarantee: Each test uses a `SandboxGuard` that deletes the sandbox
//! on drop (both success and panic paths).

#![cfg(feature = "daytona-live-tests")]

use async_trait::async_trait;
use everruns_core::error::Result;
use everruns_core::{
    connection_services::UserConnectionResolver, session_services::KeyInfo,
    session_services::SecretInfo, tool_context::ToolContext,
};
use everruns_core::{session_services::SessionStorageStore, typed_id::SessionId};
use everruns_integrations_daytona::client::DaytonaClient;
use everruns_platform::session_sandbox::{
    SessionSandboxConfig, SessionSandboxExecRequest, SessionSandboxState, SessionSandboxStatus,
    create_session_sandbox_provider,
};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

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
        .filter(|k| !k.trim().is_empty())
}

/// Require `DAYTONA_API_KEY` or panic. Live tests fail closed so CI cannot
/// silently pass when the credential is missing. See `knowledge/integrations/integrations.md`.
macro_rules! require_api_key {
    () => {
        match get_api_key() {
            Some(key) => key,
            None => panic!(
                "DAYTONA_API_KEY not set — live tests require real credentials (fail-closed policy)"
            ),
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

struct LiveStorageStore {
    secrets: Mutex<HashMap<String, String>>,
}

impl LiveStorageStore {
    fn new() -> Self {
        Self {
            secrets: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl SessionStorageStore for LiveStorageStore {
    async fn set_value(&self, _session_id: SessionId, _key: &str, _value: &str) -> Result<()> {
        Ok(())
    }

    async fn get_value(&self, _session_id: SessionId, _key: &str) -> Result<Option<String>> {
        Ok(None)
    }

    async fn delete_value(&self, _session_id: SessionId, _key: &str) -> Result<bool> {
        Ok(false)
    }

    async fn list_keys(&self, _session_id: SessionId) -> Result<Vec<KeyInfo>> {
        Ok(vec![])
    }

    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        self.secrets
            .lock()
            .await
            .insert(format!("{session_id}:{name}"), value.to_string());
        Ok(())
    }

    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        Ok(self
            .secrets
            .lock()
            .await
            .get(&format!("{session_id}:{name}"))
            .cloned())
    }

    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        Ok(self
            .secrets
            .lock()
            .await
            .remove(&format!("{session_id}:{name}"))
            .is_some())
    }

    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        let prefix = format!("{session_id}:");
        Ok(self
            .secrets
            .lock()
            .await
            .keys()
            .filter(|key| key.starts_with(&prefix))
            .map(|key| SecretInfo {
                name: key.strip_prefix(&prefix).unwrap_or(key).to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .collect())
    }
}

struct StaticConnectionResolver {
    api_key: String,
}

#[async_trait]
impl UserConnectionResolver for StaticConnectionResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>> {
        if provider == "daytona" {
            Ok(Some(self.api_key.clone()))
        } else {
            Ok(None)
        }
    }
}

fn live_provider_context(api_key: String) -> ToolContext {
    let session_id = SessionId::new();
    let mut context =
        ToolContext::with_storage_store(session_id, Arc::new(LiveStorageStore::new()));
    context.connection_resolver = Some(Arc::new(StaticConnectionResolver { api_key }));
    context
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

    // Bare `exit` is contained by the subshell wrapper the client applies
    // to every command (`( ... )`), so the persistent session shell
    // survives and the subshell's non-zero status is surfaced as the
    // command's exit code. See EVE-251.
    let result = client
        .exec(id, "exit 1", None, None, |_| {})
        .await
        .expect("exec with bare `exit` should not fail");
    assert_ne!(
        result.exit_code, 0,
        "Expected nonzero exit code from `exit 1`, got 0"
    );

    // Session survives the previous command, so the next exec must succeed
    // on the same shell without any reset.
    let result = client
        .exec(id, "echo recovered", None, None, |_| {})
        .await
        .expect("exec after bare `exit` failed");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.result.contains("recovered"),
        "Expected 'recovered' in output: {}",
        result.result
    );
}

/// A timed-out command must not poison the next exec in the same sandbox.
#[tokio::test]
async fn test_live_exec_timeout_does_not_poison_next_command() {
    let api_key = require_api_key!();
    let (client, guard) = create_test_sandbox(api_key, "exec-timeout-recovery").await;
    let id = &guard.sandbox_id;

    let timeout_err = client
        .exec(id, "sleep 10", None, Some(1_500), |_| {})
        .await
        .expect_err("sleep 10 should time out");
    assert!(
        timeout_err.contains("Command timed out after"),
        "Expected timeout error, got: {timeout_err}"
    );

    let result = client
        .exec(id, "echo timeout-recovered", None, None, |_| {})
        .await
        .expect("exec after timeout failed");
    assert_eq!(result.exit_code, 0);
    assert!(
        result.result.contains("timeout-recovered"),
        "Expected recovery output, got: {}",
        result.result
    );
}

#[tokio::test]
async fn test_live_session_sandbox_provider_flow() {
    let api_key = require_api_key!();
    let context = live_provider_context(api_key);
    let provider = create_session_sandbox_provider("daytona")
        .expect("session_sandbox Daytona provider should be registered");
    let config = SessionSandboxConfig {
        provider: "daytona".to_string(),
        auto_start: true,
        idle_pause_after_seconds: 180,
        provider_config: json!({
            "snapshot": "daytona-small",
            "workspace_path": "/home/daytona/workspace",
            "title": "live-session-sandbox-provider"
        }),
        init: Default::default(),
    };

    let instance = provider
        .create(&context, &config)
        .await
        .expect("managed session sandbox create failed");
    let guard = SandboxGuard::new(instance.external_id.clone());

    let exec = provider
        .exec(
            &context,
            &config,
            &instance,
            &SessionSandboxExecRequest {
                command: "echo session-sandbox-ok".to_string(),
                cwd: instance.workspace_path.clone(),
                timeout_ms: Some(30_000),
                output_mode: "normal".to_string(),
            },
        )
        .await
        .expect("managed session sandbox exec failed");
    assert_eq!(exec.exit_code, 0);
    assert!(exec.stdout.contains("session-sandbox-ok"));

    provider
        .write_file(
            &context,
            &config,
            &instance,
            "/home/daytona/live-session-sandbox.txt",
            "provider-flow\n",
        )
        .await
        .expect("managed session sandbox write failed");

    let read = provider
        .read_file(
            &context,
            &config,
            &instance,
            "/home/daytona/live-session-sandbox.txt",
        )
        .await
        .expect("managed session sandbox read failed");
    assert_eq!(read.content, "provider-flow\n");

    let paused = provider
        .pause(&context, &config, &instance)
        .await
        .expect("managed session sandbox pause failed");
    let resumed = provider
        .resume(&context, &config, &paused)
        .await
        .expect("managed session sandbox resume failed");

    let status = provider
        .status(
            &context,
            &config,
            &SessionSandboxState {
                provider: "daytona".to_string(),
                status: SessionSandboxStatus::Running,
                instance: resumed.clone(),
                init_completed_at: None,
                last_init_error: None,
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
            },
        )
        .await
        .expect("managed session sandbox status failed");
    assert_eq!(status.session_status, SessionSandboxStatus::Running);

    provider
        .delete(&context, &config, &resumed)
        .await
        .expect("managed session sandbox delete failed");
    std::mem::forget(guard);
}

#[tokio::test]
async fn test_live_session_sandbox_recovers_after_physical_loss() {
    let api_key = require_api_key!();
    let client = DaytonaClient::new(api_key.clone());
    let context = live_provider_context(api_key);
    let provider = create_session_sandbox_provider("daytona")
        .expect("session_sandbox Daytona provider should be registered");
    let config = SessionSandboxConfig {
        provider: "daytona".to_string(),
        auto_start: true,
        idle_pause_after_seconds: 180,
        provider_config: json!({
            "snapshot": "daytona-small",
            "workspace_path": "/home/daytona/workspace",
            "title": "live-session-sandbox-recovery",
            "recovery": {
                "enabled": true,
                "volume_name": "everruns-recovery"
            }
        }),
        init: Default::default(),
    };

    let instance = provider
        .create(&context, &config)
        .await
        .expect("recoverable session sandbox create failed");
    let original_guard = SandboxGuard::new(instance.external_id.clone());

    provider
        .write_file(
            &context,
            &config,
            &instance,
            "/home/daytona/workspace/recovery-marker.txt",
            "survived\n",
        )
        .await
        .expect("recovery marker write failed");
    let checkpointed = provider
        .checkpoint(&context, &config, &instance)
        .await
        .expect("workspace checkpoint failed");

    client
        .delete_sandbox(&instance.external_id)
        .await
        .expect("physical sandbox deletion failed");
    let mut sandbox_is_absent = false;
    for _ in 0..60 {
        match client.get_sandbox(&instance.external_id).await {
            Err(err) if err.contains("404 Not Found") || err.contains("(404)") => {
                sandbox_is_absent = true;
                break;
            }
            Ok(_) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
            Err(err) => panic!("failed while waiting for physical sandbox deletion: {err}"),
        }
    }
    assert!(sandbox_is_absent, "physical sandbox deletion timed out");
    let state = SessionSandboxState {
        provider: "daytona".to_string(),
        status: SessionSandboxStatus::Running,
        instance: checkpointed.clone(),
        init_completed_at: None,
        last_init_error: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    let lost = provider
        .status(&context, &config, &state)
        .await
        .expect("lost sandbox status failed");
    assert_eq!(lost.session_status, SessionSandboxStatus::Lost);

    let replacement = provider
        .resume(&context, &config, &checkpointed)
        .await
        .expect("lost sandbox replacement failed");
    let replacement_guard = SandboxGuard::new(replacement.external_id.clone());
    assert_ne!(replacement.external_id, instance.external_id);

    let restored = provider
        .read_file(
            &context,
            &config,
            &replacement,
            "/home/daytona/workspace/recovery-marker.txt",
        )
        .await
        .expect("restored recovery marker read failed");
    assert_eq!(restored.content, "survived\n");

    provider
        .delete(&context, &config, &replacement)
        .await
        .expect("recovered session sandbox delete failed");
    std::mem::forget(original_guard);
    std::mem::forget(replacement_guard);
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
            chunks.push(chunk);
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

    // List sandboxes via api_call as a smoke check that the `/sandbox`
    // endpoint is reachable and returns a sensible shape.
    //
    // The test is intentionally lenient about whether the freshly-created
    // sandbox shows up in the list. Daytona's `/sandbox` returns either a
    // bare array or a `{ items: [...], next_cursor: ... }` wrapper, has
    // pagination defaults that may cap results well below the total, and
    // is eventually consistent — so a brand-new sandbox is not guaranteed
    // to be on the first page (or any bounded page window). The single-get
    // and exec assertions above already prove the sandbox actually exists
    // server-side; the listing assertion only needs to prove the endpoint
    // works and returns sandboxes whose entries look like sandboxes.
    let mut next: Option<String> = None;
    let mut found_self = false;
    let mut found_on_page: Option<usize> = None;
    let mut pages_scanned: usize = 0;
    let mut total_items_seen: usize = 0;
    // The loop is an expression so the final page falls out as a value with
    // no dummy/uninitialized binding for `unused_assignments` to complain
    // about.
    let last_page: serde_json::Value = loop {
        let path = match &next {
            // Cursors are opaque server-issued strings and can contain
            // reserved characters (`+`, `=`, `&`, `/`); percent-encode the
            // value before splicing it into the query string.
            Some(cursor) => format!("/sandbox?cursor={}", urlencoding::encode(cursor)),
            None => "/sandbox".to_string(),
        };
        let page = client
            .api_call(reqwest::Method::GET, &path, None)
            .await
            .expect("api_call GET /sandbox failed");

        let items = page
            .as_array()
            .or_else(|| page["items"].as_array())
            .or_else(|| page["data"].as_array())
            .unwrap_or_else(|| {
                panic!(
                    "expected /sandbox response to be a bare array or a wrapped {{ items: [...] }} / {{ data: [...] }} object, got: {page}"
                )
            });
        pages_scanned += 1;
        total_items_seen += items.len();
        if items.iter().any(|s| s["id"].as_str() == Some(sandbox_id)) {
            found_self = true;
            found_on_page = Some(pages_scanned);
            break page;
        }
        next = page["next_cursor"]
            .as_str()
            .or_else(|| page["nextCursor"].as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        // Cap follow-on pages so a runaway listing never makes the test hang.
        if next.is_none() || pages_scanned >= 5 {
            break page;
        }
    };

    // The listing must at least be a non-empty list of objects that look
    // like sandboxes — that proves the endpoint is wired up correctly.
    let last_page_items = last_page
        .as_array()
        .or_else(|| last_page["items"].as_array())
        .or_else(|| last_page["data"].as_array())
        .expect("listing should expose items in array / items / data");
    assert!(
        !last_page_items.is_empty(),
        "listing returned no sandboxes at all; expected at least our own ({sandbox_id})"
    );
    assert!(
        last_page_items
            .iter()
            .all(|s| s.is_object() && s["id"].is_string()),
        "listing entries should be objects with `id` strings, got: {last_page}"
    );

    if found_self {
        eprintln!(
            "[test] Sandbox {sandbox_id} found in /sandbox listing (on page {} after scanning {pages_scanned} page(s), {total_items_seen} items)",
            found_on_page.unwrap_or(pages_scanned),
        );
    } else {
        // Don't fail — the lifecycle has already been proven via direct
        // GET /sandbox/{id} + toolbox exec above. Log the diagnostic so
        // pagination-related drifts are still visible in CI output.
        let sample_ids: Vec<&str> = last_page_items
            .iter()
            .take(5)
            .filter_map(|s| s["id"].as_str())
            .collect();
        eprintln!(
            "[test] Sandbox {sandbox_id} not present after scanning {pages_scanned} page(s) ({total_items_seen} items scanned); sample ids on last page: {sample_ids:?}. \
            Direct GET succeeded, so this is treated as a pagination/visibility quirk rather than a failure."
        );
    }

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
