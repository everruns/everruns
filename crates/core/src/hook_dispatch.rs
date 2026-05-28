// User-defined hooks: production bash dispatcher.
//
// `VirtualBashHookDispatcher` is the production `BashHookDispatcher` impl.
// It runs the user-authored hook command through the same bashkit
// interpreter the `virtual_bash` capability uses, against a session VFS
// adapter so the hook script sees the same `/workspace` the agent sees.
//
// The hook payload is delivered via:
//   - `$EVERRUNS_HOOK_PAYLOAD_JSON` env var (always set, full JSON payload)
//   - `$EVERRUNS_HOOK_PAYLOAD_PATH` env var pointing at a file written into
//     the session VFS, containing the same JSON payload. The file is cleaned
//     up after the hook returns (best-effort).
//   - Convenience scalars (`$EVERRUNS_HOOK_EVENT`, `$EVERRUNS_HOOK_ID`,
//     `$EVERRUNS_HOOK_SESSION_ID`, `$EVERRUNS_HOOK_TURN_ID`,
//     `$EVERRUNS_HOOK_TOOL_NAME`, `$EVERRUNS_HOOK_TOOL_CALL_ID`).
//
// bashkit does not expose a process-level stdin to user scripts (it
// interprets the script directly rather than spawning a shell), so env-var
// delivery is the closest sandbox-preserving equivalent to the stdin
// contract you see in Claude Code hooks.

use std::sync::Arc;

use async_trait::async_trait;
use bashkit::{Bash, ExecutionLimits, TraceMode};

use crate::capabilities::SessionFileSystemAdapter;
use crate::hook_executor::{
    BashExecOutput, BashHookDispatcher, ExecutorOpts, HOOK_PAYLOAD_DIR, HOOK_PAYLOAD_WORKSPACE_DIR,
    HookPayload, payload_filename, standard_hook_env,
};
use crate::traits::SessionFileSystem;

/// Trimmed-down ExecutionLimits for hook commands. Hooks are short,
/// side-effect-y scripts, not agent-authored programs: keep the
/// max_input_bytes bound but cap commands/iterations/depth lower than
/// `virtual_bash` does so a runaway hook can't hold up tool execution.
fn hook_execution_limits() -> ExecutionLimits {
    ExecutionLimits::new()
        .max_commands(200)
        .max_loop_iterations(2_000)
        .max_function_depth(32)
        .max_input_bytes(64 * 1024) // user-authored command is bounded too
        .max_ast_depth(64)
        .parser_timeout(std::time::Duration::from_secs(2))
}

/// Bashkit-backed dispatcher. Constructed per session (or shared across
/// sessions if `store` is shared) so that the produced
/// `SessionFileSystemAdapter` operates on the right session VFS.
pub struct VirtualBashHookDispatcher {
    store: Arc<dyn SessionFileSystem>,
}

impl VirtualBashHookDispatcher {
    pub fn new(store: Arc<dyn SessionFileSystem>) -> Self {
        Self { store }
    }

    /// Best-effort cleanup of the on-disk payload file. Failures are
    /// logged but never raise — the hook outcome must not depend on
    /// cleanup success.
    async fn cleanup_payload_file(&self, session_id: crate::typed_id::SessionId, path: &str) {
        if let Err(e) = self.store.delete_file(session_id, path, false).await {
            tracing::debug!(
                error = %e,
                path = %path,
                "VirtualBashHookDispatcher: payload file cleanup failed (non-fatal)"
            );
        }
    }
}

#[async_trait]
impl BashHookDispatcher for VirtualBashHookDispatcher {
    async fn dispatch(
        &self,
        payload: &HookPayload,
        command: &str,
        extra_env: &std::collections::BTreeMap<String, String>,
        opts: &ExecutorOpts,
    ) -> Result<BashExecOutput, String> {
        let session_id = payload.session_id;

        // The bashkit adapter strips the `/workspace` prefix before hitting
        // the session VFS. So we expose the workspace-relative path to the
        // script ($EVERRUNS_HOOK_PAYLOAD_PATH) and write/delete via the
        // storage-relative path ourselves.
        let filename = payload_filename(payload);
        let script_path = format!("{HOOK_PAYLOAD_WORKSPACE_DIR}/{filename}");
        let storage_path = format!("{HOOK_PAYLOAD_DIR}/{filename}");
        let standard_env = standard_hook_env(payload, &script_path)?;

        // Write the payload to the session VFS so jq-style scripts can
        // `cat $EVERRUNS_HOOK_PAYLOAD_PATH | jq …`. If we cannot write,
        // proceed anyway — the env-var copy is always present and the
        // path env will resolve to a missing file (the script's choice).
        let payload_json = standard_env
            .iter()
            .find(|(k, _)| k == "EVERRUNS_HOOK_PAYLOAD_JSON")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let wrote_file = match self
            .store
            .write_file(session_id, &storage_path, &payload_json, "utf-8")
            .await
        {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %storage_path,
                    "VirtualBashHookDispatcher: VFS payload write failed; env-var fallback still set"
                );
                false
            }
        };

        // Build the bash interpreter against the session VFS.
        let session_fs = Arc::new(SessionFileSystemAdapter::new(
            session_id,
            self.store.clone(),
        ));
        let mut builder = Bash::builder()
            .fs(session_fs)
            .cwd("/workspace")
            .username("everruns")
            .hostname("everruns-hook")
            .env("HOME", "/home/agent")
            .env("SHELL", "/bin/bash")
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("WORKSPACE", "/workspace")
            .limits(hook_execution_limits())
            .max_memory(10 * 1024 * 1024)
            .trace_mode(TraceMode::Redacted);
        for (k, v) in &standard_env {
            builder = builder.env(k, v);
        }
        for (k, v) in extra_env {
            // Operator-supplied env can override standard env if the operator
            // explicitly named the same key; this is intentional (escape
            // hatch) but applies only to per-spec env, never to other
            // capabilities' contributions because each spec gets its own
            // executor.
            builder = builder.env(k, v);
        }
        let mut bash = builder.build();
        let cancel_token = bash.cancellation_token();

        let timeout = std::time::Duration::from_millis(opts.timeout_ms.max(1) as u64);
        let exec = tokio::time::timeout(timeout, bash.exec(command)).await;

        // Best-effort cleanup happens regardless of outcome.
        if wrote_file {
            self.cleanup_payload_file(session_id, &storage_path).await;
        }

        match exec {
            Ok(Ok(output)) => {
                let mut stdout = output.stdout;
                let mut stderr = output.stderr;
                // Apply the executor-level output cap. Bashkit's own
                // `max_input_bytes` bounds the *script* size; for output we
                // truncate here so the parse step sees a bounded buffer.
                let cap = opts.max_output_bytes;
                if stdout.len() + stderr.len() > cap {
                    let stdout_budget = cap.min(stdout.len());
                    truncate_at_char_boundary(&mut stdout, stdout_budget);
                    let stderr_budget = cap.saturating_sub(stdout.len());
                    truncate_at_char_boundary(&mut stderr, stderr_budget);
                    return Err(format!(
                        "hook output exceeded {} bytes (stdout={}, stderr={})",
                        cap,
                        stdout.len(),
                        stderr.len(),
                    ));
                }
                Ok(BashExecOutput {
                    exit_code: output.exit_code,
                    stdout,
                    stderr,
                })
            }
            Ok(Err(e)) => Err(format!("hook execution error: {e}")),
            Err(_) => {
                cancel_token.store(true, std::sync::atomic::Ordering::Relaxed);
                Err(format!("hook timed out after {} ms", opts.timeout_ms))
            }
        }
    }
}

fn truncate_at_char_boundary(s: &mut String, mut end: usize) {
    if end >= s.len() {
        return;
    }
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

// ============================================================================
// Tests (integration with real bashkit)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result;
    use crate::hook_executor::{BashHookExecutor, HOOK_PAYLOAD_DIR, HookExecutor};
    use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use crate::traits::SessionFileSystem;
    use crate::typed_id::SessionId;
    use crate::user_hook_types::{HookEvent, HookId, HookOutcome};
    use chrono::Utc;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct MockFileStore {
        files: Mutex<HashMap<String, String>>,
    }

    impl MockFileStore {
        fn read(&self, path: &str) -> Option<String> {
            self.files.lock().unwrap().get(path).cloned()
        }
    }

    #[async_trait]
    impl SessionFileSystem for MockFileStore {
        async fn read_file(
            &self,
            _session_id: SessionId,
            path: &str,
        ) -> Result<Option<SessionFile>> {
            let entry = self.files.lock().unwrap().get(path).cloned();
            Ok(entry.map(|content| {
                let size = content.len() as i64;
                SessionFile {
                    id: Uuid::new_v4(),
                    session_id: Uuid::nil(),
                    path: path.to_string(),
                    name: path.rsplit('/').next().unwrap_or("").to_string(),
                    content: Some(content),
                    encoding: "utf-8".to_string(),
                    is_directory: false,
                    is_readonly: false,
                    size_bytes: size,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                }
            }))
        }

        async fn write_file(
            &self,
            _session_id: SessionId,
            path: &str,
            content: &str,
            _encoding: &str,
        ) -> Result<SessionFile> {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), content.to_string());
            Ok(SessionFile {
                id: Uuid::new_v4(),
                session_id: Uuid::nil(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                content: Some(content.to_string()),
                encoding: "utf-8".to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            path: &str,
            _recursive: bool,
        ) -> Result<bool> {
            Ok(self.files.lock().unwrap().remove(path).is_some())
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> Result<Vec<FileInfo>> {
            Ok(vec![])
        }

        async fn stat_file(&self, _session_id: SessionId, _path: &str) -> Result<Option<FileStat>> {
            Ok(None)
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(&self, _session_id: SessionId, _path: &str) -> Result<FileInfo> {
            Err(anyhow::anyhow!("not implemented").into())
        }
    }

    fn payload(event: HookEvent, data: serde_json::Value) -> HookPayload {
        HookPayload {
            event,
            hook_id: HookId::for_user("t"),
            session_id: SessionId::from(Uuid::nil()),
            turn_id: Some("trn_test".into()),
            org_id: None,
            agent_id: Some("agt_test".into()),
            ts: "2026-05-28T00:00:00Z".into(),
            data,
        }
    }

    fn opts() -> ExecutorOpts {
        ExecutorOpts {
            timeout_ms: 5_000,
            max_output_bytes: 64 * 1024,
        }
    }

    async fn run(
        store: Arc<dyn SessionFileSystem>,
        command: &str,
        env: std::collections::BTreeMap<String, String>,
        payload: HookPayload,
    ) -> HookOutcome {
        let dispatcher = Arc::new(VirtualBashHookDispatcher::new(store));
        let exec = BashHookExecutor::with_dispatcher(command.to_string(), env, dispatcher);
        exec.run(payload, &opts()).await
    }

    #[tokio::test]
    async fn allow_by_default_with_zero_exit() {
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::default());
        let outcome = run(
            store,
            "echo -n",
            Default::default(),
            payload(HookEvent::PreToolUse, json!({})),
        )
        .await;
        assert!(matches!(outcome, HookOutcome::Allow), "{:?}", outcome);
    }

    #[tokio::test]
    async fn block_via_json_decision() {
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::default());
        let cmd = r#"printf '%s' '{"decision":"block","reason":"nope","user_message":"blocked"}'"#;
        let outcome = run(
            store,
            cmd,
            Default::default(),
            payload(HookEvent::PreToolUse, json!({})),
        )
        .await;
        match outcome {
            HookOutcome::Block {
                reason,
                user_message,
            } => {
                assert_eq!(reason, "nope");
                assert_eq!(user_message.as_deref(), Some("blocked"));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn block_via_exit_code_fallback() {
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::default());
        // Empty stdout + non-zero exit + stderr reason.
        let cmd = "echo blocked-reason >&2; exit 1";
        let outcome = run(
            store,
            cmd,
            Default::default(),
            payload(HookEvent::PreToolUse, json!({})),
        )
        .await;
        match outcome {
            HookOutcome::Block { reason, .. } => {
                assert!(reason.contains("blocked-reason"), "reason = {reason:?}");
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mutate_with_patch() {
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::default());
        let cmd = r#"printf '%s' '{"decision":"mutate","patch":{"arguments":{"command":"ls"}}}'"#;
        let outcome = run(
            store,
            cmd,
            Default::default(),
            payload(
                HookEvent::PreToolUse,
                json!({"tool_name": "bash", "tool_call_id": "call_1", "arguments": {}}),
            ),
        )
        .await;
        match outcome {
            HookOutcome::Mutate { patch, .. } => {
                assert_eq!(patch["arguments"]["command"], "ls");
            }
            other => panic!("expected Mutate, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_json_stdout_is_error() {
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::default());
        let outcome = run(
            store,
            "echo not-json",
            Default::default(),
            payload(HookEvent::PostToolUse, json!({})),
        )
        .await;
        assert!(
            matches!(outcome, HookOutcome::Error { .. }),
            "{:?}",
            outcome
        );
    }

    #[tokio::test]
    async fn payload_env_var_visible_to_script() {
        // The script echoes the EVERRUNS_HOOK_EVENT into a sentinel file on
        // the session VFS so we can assert delivery without depending on
        // bashkit's stdout for non-decision text.
        let cmd = "echo $EVERRUNS_HOOK_EVENT > /workspace/.seen-event";
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        let outcome = run(
            store,
            cmd,
            Default::default(),
            payload(HookEvent::PreToolUse, json!({})),
        )
        .await;
        assert!(matches!(outcome, HookOutcome::Allow), "{:?}", outcome);
        assert_eq!(
            mock.read("/.seen-event")
                .or_else(|| mock.read("/workspace/.seen-event"))
                .map(|s| s.trim().to_string()),
            Some("pre_tool_use".to_string())
        );
        let _ = store;
    }

    #[tokio::test]
    async fn tool_name_env_var_set_for_tool_events() {
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        let cmd = "echo $EVERRUNS_HOOK_TOOL_NAME > /workspace/.tool-name";
        let _ = run(
            store,
            cmd,
            Default::default(),
            payload(
                HookEvent::PreToolUse,
                json!({"tool_name": "edit_file", "tool_call_id": "c"}),
            ),
        )
        .await;
        assert_eq!(
            mock.read("/.tool-name")
                .or_else(|| mock.read("/workspace/.tool-name"))
                .map(|s| s.trim().to_string()),
            Some("edit_file".to_string())
        );
    }

    #[tokio::test]
    async fn payload_file_written_then_cleaned_up() {
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        // Copy the payload file contents into a workspace file BEFORE the
        // dispatcher cleans up the source.
        let cmd = r#"cat "$EVERRUNS_HOOK_PAYLOAD_PATH" > /workspace/.snapshot.json"#;
        let outcome = run(
            store,
            cmd,
            Default::default(),
            payload(HookEvent::PostToolUse, json!({"tool_name":"x"})),
        )
        .await;
        assert!(matches!(outcome, HookOutcome::Allow), "{:?}", outcome);

        let files = mock.files.lock().unwrap();
        // The snapshot file we copied through should exist with the
        // payload JSON inside.
        let snapshot = files
            .get("/.snapshot.json")
            .or_else(|| files.get("/workspace/.snapshot.json"))
            .expect("snapshot file written");
        assert!(snapshot.contains("\"event\":\"post_tool_use\""));
        // The original payload file in /.hooks/ should have been removed.
        let lingering: Vec<_> = files
            .keys()
            .filter(|k| k.starts_with(HOOK_PAYLOAD_DIR))
            .collect();
        assert!(
            lingering.is_empty(),
            "payload file not cleaned up: {lingering:?}"
        );
    }

    #[tokio::test]
    async fn extra_env_overrides_standard_env() {
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        let mut env = std::collections::BTreeMap::new();
        env.insert("EVERRUNS_HOOK_EVENT".to_string(), "overridden".to_string());
        let _ = run(
            store,
            "echo $EVERRUNS_HOOK_EVENT > /workspace/.event",
            env,
            payload(HookEvent::PreToolUse, json!({})),
        )
        .await;
        assert_eq!(
            mock.read("/.event")
                .or_else(|| mock.read("/workspace/.event"))
                .map(|s| s.trim().to_string()),
            Some("overridden".to_string())
        );
    }

    #[tokio::test]
    async fn timeout_returns_error_outcome() {
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::default());
        let dispatcher = Arc::new(VirtualBashHookDispatcher::new(store));
        let exec = BashHookExecutor::with_dispatcher(
            // Busy loop that bashkit's loop limit + our wall-clock timeout
            // should jointly bound. We deliberately set a very short
            // timeout to exercise the timeout path.
            "while true; do :; done".to_string(),
            Default::default(),
            dispatcher,
        );
        let opts = ExecutorOpts {
            timeout_ms: 50,
            max_output_bytes: 64 * 1024,
        };
        let outcome = exec
            .run(payload(HookEvent::PreToolUse, json!({})), &opts)
            .await;
        // Either the wall-clock timeout or bashkit's own loop cap may fire
        // first depending on platform timing. Both must yield Error (which
        // the adapter then converts per on_error).
        match outcome {
            HookOutcome::Error { message } => {
                assert!(
                    message.contains("timed out") || message.contains("execution error"),
                    "unexpected error message: {message}"
                );
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn jq_can_read_payload_from_env_path() {
        // Smoke test for the documented `jq` workflow. We use a tiny
        // shell-only json walk because the bashkit interpreter does not
        // ship `jq` as a built-in.
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        let cmd = r#"grep -o '"event":"[^"]*"' "$EVERRUNS_HOOK_PAYLOAD_PATH" > /workspace/.grep"#;
        let outcome = run(
            store,
            cmd,
            Default::default(),
            payload(HookEvent::SessionStart, json!({"agent_id":"agt_x"})),
        )
        .await;
        assert!(matches!(outcome, HookOutcome::Allow), "{:?}", outcome);
        assert_eq!(
            mock.read("/.grep")
                .or_else(|| mock.read("/workspace/.grep"))
                .map(|s| s.trim().to_string()),
            Some("\"event\":\"session_start\"".to_string())
        );
    }
}
