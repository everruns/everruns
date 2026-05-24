// Bash tool for the coding CLI.
//
// Read/write/edit/list/grep/stat all live in the built-in `file_system`
// capability now that ercode selects `RealDiskFileStore` through its platform
// filesystem factory. The bash tool stays custom because the built-in `virtual_bash`
// runs commands against the VFS, not against the real workspace, and the
// security model for unsandboxed shell-on-host needs ercode-specific policy
// (timeout, output cap, approval gate). See EVE-478 for the eventual
// runtime-side story.

use crate::approval::{ApprovalGate, ApprovalRequest};
use async_trait::async_trait;
use everruns_core::exec_tool_result::ExecToolResultPayload;
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Workspace context for the bash tool. Just the root path — path
/// resolution for file ops now lives inside the configured session filesystem.
#[derive(Clone)]
pub struct Workspace {
    root: Arc<PathBuf>,
}

impl Workspace {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: Arc::new(root),
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}

pub struct BashTool {
    ws: Workspace,
    gate: Arc<ApprovalGate>,
    timeout_secs: u64,
    max_output_bytes: usize,
}

impl BashTool {
    pub fn new(ws: Workspace, gate: Arc<ApprovalGate>) -> Self {
        Self {
            ws,
            gate,
            timeout_secs: 120,
            max_output_bytes: 1024 * 1024,
        }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn display_name(&self) -> Option<&str> {
        Some("Bash")
    }
    fn description(&self) -> &str {
        "Run a bash command from the workspace root. Captures stdout/stderr with configurable verbosity. 120s timeout. Requires user approval."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Shell command to run via bash -lc."},
                "output": everruns_core::tool_output_sanitizer::output_verbosity_schema()
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }
    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_persist_output(true)
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let command = match arguments.get("command").and_then(Value::as_str) {
            Some(c) => c.to_string(),
            None => return ToolExecutionResult::tool_error("'command' is required"),
        };
        // EVE-489: default to `auto` (persistence-first). On success, returns
        // a compact ~512 B summary while full output stays in `/outputs/` via
        // ToolOutputPersistenceCapability. On failure, returns a `normal`
        // (~8 KiB) diagnostic window. Explicit modes (silent/concise/normal/
        // verbose/full) still override this behavior.
        let output_mode = arguments
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or("auto");
        let approved = self
            .gate
            .approve(ApprovalRequest::Bash {
                command: command.clone(),
            })
            .await;
        if !approved {
            return ToolExecutionResult::tool_error("user denied bash command");
        }
        let root = self.ws.root().to_path_buf();
        let timeout = std::time::Duration::from_secs(self.timeout_secs);
        let max_bytes = self.max_output_bytes;

        // kill_on_drop ensures a timed-out command is reaped: if we drop the
        // Child (via the timeout future being canceled) the OS process is
        // killed and waited on by tokio's background reaper.
        let mut child = match Command::new("bash")
            .arg("-lc")
            .arg(&command)
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::tool_error(format!("spawn failed: {e}")),
        };
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();

        let run = async {
            let mut out_buf = Vec::with_capacity(4096);
            let mut err_buf = Vec::with_capacity(4096);
            let mut o = vec![0u8; 4096];
            let mut e = vec![0u8; 4096];
            // Track per-stream EOF so we stop polling a closed pipe instead
            // of busy-looping on `Ok(0)` (which would starve the other stream).
            let mut out_done = false;
            let mut err_done = false;
            while !(out_done && err_done) {
                tokio::select! {
                    // Bias the select toward stdout so we drain it first on
                    // every wake — this keeps reasoning about ordering simple.
                    biased;
                    n = stdout.read(&mut o), if !out_done => match n {
                        Ok(0) | Err(_) => out_done = true,
                        Ok(n) => out_buf.extend_from_slice(&o[..n]),
                    },
                    n = stderr.read(&mut e), if !err_done => match n {
                        Ok(0) | Err(_) => err_done = true,
                        Ok(n) => err_buf.extend_from_slice(&e[..n]),
                    },
                }
                if out_buf.len() + err_buf.len() > max_bytes * 2 {
                    // Cap exceeded — kill the child and stop reading. The
                    // remaining drain below is bounded by max_bytes.
                    let _ = child.start_kill();
                    break;
                }
            }
            let status = child.wait().await;
            (status, out_buf, err_buf)
        };
        let (status, mut out_buf, mut err_buf) = match tokio::time::timeout(timeout, run).await {
            Ok(r) => r,
            Err(_) => {
                // child (owned by `run`) is dropped here, kill_on_drop reaps.
                return ToolExecutionResult::tool_error(format!(
                    "command timed out after {}s",
                    self.timeout_secs
                ));
            }
        };
        let out_truncated = out_buf.len() > max_bytes;
        if out_truncated {
            out_buf.truncate(max_bytes);
        }
        let err_truncated = err_buf.len() > max_bytes;
        if err_truncated {
            err_buf.truncate(max_bytes);
        }
        let stdout_text = String::from_utf8_lossy(&out_buf).to_string();
        let stderr_text = String::from_utf8_lossy(&err_buf).to_string();
        let exit_code = status.ok().and_then(|s| s.code()).unwrap_or(-1);
        let payload =
            ExecToolResultPayload::new(&stdout_text, &stderr_text, exit_code, output_mode);
        let ExecToolResultPayload {
            stdout,
            stderr,
            exit_code,
            success,
            truncated,
            total_lines,
            raw_output,
        } = payload;

        ToolExecutionResult::success_with_raw_output(
            json!({
                "command": command,
                "exit_code": exit_code,
                "success": success,
                "stdout": stdout,
                "stderr": stderr,
                "truncated": truncated || out_truncated || err_truncated,
                "total_lines": total_lines,
                "output_limited": out_truncated || err_truncated,
            }),
            raw_output,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::{Capability, ToolOutputPersistenceCapability};
    use everruns_core::{ToolCall, ToolContext};
    use everruns_runtime::RealDiskFileStore;

    #[test]
    fn bash_tool_requests_output_persistence() {
        let tool = BashTool::new(
            Workspace::new(std::env::current_dir().unwrap()),
            ApprovalGate::auto(),
        );

        assert_eq!(tool.hints().persist_output, Some(true));
        assert_eq!(tool.hints().long_running, Some(true));
    }

    #[tokio::test]
    async fn bash_tool_uses_exec_payload_shape_and_raw_output() {
        let dir = tempfile::tempdir().unwrap();
        let tool = BashTool::new(
            Workspace::new(dir.path().to_path_buf()),
            ApprovalGate::auto(),
        );

        let result = tool
            .execute(json!({
                "command": "for i in {1..400}; do echo line-$i; done",
                "output": "silent"
            }))
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        assert_eq!(value["exit_code"], 0);
        assert_eq!(value["success"], true);
        assert_eq!(value["total_lines"], 400);
        assert_eq!(value["truncated"], true);
        assert!(value["stdout"].as_str().unwrap().contains("line-1"));
        assert!(value["stdout"].as_str().unwrap().len() < 2048);
        assert!(value["_raw_output"].as_str().unwrap().contains("line-400"));
    }

    #[tokio::test]
    async fn bash_tool_output_persistence_hook_saves_full_output_to_outputs_folder() {
        let dir = tempfile::tempdir().unwrap();
        let tool = BashTool::new(
            Workspace::new(dir.path().to_path_buf()),
            ApprovalGate::auto(),
        );
        let call = ToolCall {
            id: "call-persist".to_string(),
            name: "bash".to_string(),
            arguments: json!({
                "command": "for i in {1..3000}; do echo saved-line-$i; done",
                "output": "silent"
            }),
        };
        let mut result = tool
            .execute(call.arguments.clone())
            .await
            .into_tool_result(&call.id, &call.name);
        let file_store = Arc::new(RealDiskFileStore::new(dir.path()).unwrap());
        let context = ToolContext::with_file_store(Default::default(), file_store.clone());
        let tool_def = tool.to_definition();

        for hook in ToolOutputPersistenceCapability.post_tool_exec_hooks() {
            hook.after_exec(&call, &tool_def, &mut result, &context)
                .await;
        }

        let output_files = result
            .result
            .as_ref()
            .and_then(|value| value.get("output_files"))
            .and_then(|value| value.as_array())
            .expect("output_files should be populated");
        assert_eq!(output_files.len(), 1);
        assert_eq!(
            output_files[0].as_str(),
            Some("/workspace/outputs/call-persist.stdout")
        );

        let saved = tokio::fs::read_to_string(dir.path().join("outputs/call-persist.stdout"))
            .await
            .expect("persisted stdout should be readable from the outputs folder");
        assert!(saved.contains("saved-line-3000"));
    }

    // ====================================================================
    // EVE-489: persistence-first `auto` output mode
    // ====================================================================

    /// Issue EVE-489 reproducer: successful bash output should be a compact
    /// inline summary when full output is persisted to `/outputs/`. Before
    /// the fix, requesting `output: "normal"` returned ~8 KiB inline even
    /// though the full log was already saved. With `auto` (the new default),
    /// successful runs return ≤512 bytes inline.
    #[tokio::test]
    async fn bash_success_output_should_be_persistent_first_when_output_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let tool = BashTool::new(
            Workspace::new(dir.path().to_path_buf()),
            ApprovalGate::auto(),
        );
        let call = ToolCall {
            id: "call-auto-compact".to_string(),
            name: "bash".to_string(),
            arguments: json!({
                "command": "for i in {1..2000}; do echo success-line-$i; done",
                "output": "auto"
            }),
        };
        let mut result = tool
            .execute(call.arguments.clone())
            .await
            .into_tool_result(&call.id, &call.name);
        let file_store = Arc::new(RealDiskFileStore::new(dir.path()).unwrap());
        let context = ToolContext::with_file_store(Default::default(), file_store);
        let tool_def = tool.to_definition();

        for hook in ToolOutputPersistenceCapability.post_tool_exec_hooks() {
            hook.after_exec(&call, &tool_def, &mut result, &context)
                .await;
        }

        let value = result.result.expect("bash result should be present");
        let stdout = value["stdout"].as_str().expect("stdout should be a string");
        assert_eq!(value["success"], true);
        assert!(
            value["output_files"]
                .as_array()
                .is_some_and(|files| !files.is_empty()),
            "full output should be persisted"
        );
        assert!(
            stdout.len() <= 512,
            "successful persisted bash output should be a compact inline summary, got {} bytes",
            stdout.len()
        );
    }

    #[tokio::test]
    async fn bash_defaults_to_auto_mode_for_compact_success() {
        // No `output` parameter at all — the new default must behave like `auto`.
        let dir = tempfile::tempdir().unwrap();
        let tool = BashTool::new(
            Workspace::new(dir.path().to_path_buf()),
            ApprovalGate::auto(),
        );

        let result = tool
            .execute(json!({
                "command": "for i in {1..2000}; do echo line-$i; done"
            }))
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        assert_eq!(value["success"], true);
        let stdout = value["stdout"].as_str().unwrap();
        assert!(
            stdout.len() <= 512,
            "default mode should compact successful output, got {} bytes",
            stdout.len()
        );
        // raw_output retains full content for persistence hook.
        let raw = value["_raw_output"].as_str().unwrap();
        assert!(raw.contains("line-2000"));
    }

    #[tokio::test]
    async fn bash_auto_failure_returns_diagnostic_inline_output() {
        let dir = tempfile::tempdir().unwrap();
        let tool = BashTool::new(
            Workspace::new(dir.path().to_path_buf()),
            ApprovalGate::auto(),
        );

        // Produce lots of stdout, then exit non-zero with a useful stderr line.
        let result = tool
            .execute(json!({
                "command": "for i in {1..2000}; do echo line-$i; done; echo 'error: something broke' 1>&2; exit 7",
                "output": "auto"
            }))
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success-wrapped tool result");
        };
        assert_eq!(value["success"], false);
        assert_eq!(value["exit_code"], 7);
        let stderr = value["stderr"].as_str().unwrap();
        assert!(
            stderr.contains("error: something broke"),
            "failure stderr should expose diagnostics inline, got: {stderr}"
        );
        let stdout = value["stdout"].as_str().unwrap();
        // Failure path should give substantially more than the success compact budget.
        assert!(
            stdout.len() > 512,
            "auto+failure stdout should not collapse to the success compact budget, got {} bytes",
            stdout.len()
        );
    }

    #[tokio::test]
    async fn bash_explicit_normal_still_returns_larger_inline_output() {
        let dir = tempfile::tempdir().unwrap();
        let tool = BashTool::new(
            Workspace::new(dir.path().to_path_buf()),
            ApprovalGate::auto(),
        );

        let result = tool
            .execute(json!({
                "command": "for i in {1..2000}; do echo line-$i; done",
                "output": "normal"
            }))
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        let stdout = value["stdout"].as_str().unwrap();
        // Explicit `normal` must keep the larger inline window even on success.
        assert!(
            stdout.len() > 512,
            "explicit normal should not collapse to auto-success budget, got {} bytes",
            stdout.len()
        );
        assert!(
            stdout.len() <= 8 * 1024,
            "explicit normal should respect NORMAL_BUDGET, got {} bytes",
            stdout.len()
        );
    }

    #[tokio::test]
    async fn bash_explicit_full_returns_unlimited_inline_output() {
        let dir = tempfile::tempdir().unwrap();
        let tool = BashTool::new(
            Workspace::new(dir.path().to_path_buf()),
            ApprovalGate::auto(),
        );

        let result = tool
            .execute(json!({
                "command": "for i in {1..200}; do echo line-$i; done",
                "output": "full"
            }))
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        let stdout = value["stdout"].as_str().unwrap();
        // `full` must include every line — first and last.
        assert!(
            stdout.contains("line-1\n"),
            "stdout must contain first line"
        );
        assert!(stdout.contains("line-200"), "stdout must contain last line");
        assert_eq!(value["truncated"], false);
    }
}
