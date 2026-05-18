// Bash tool for the coding CLI.
//
// Read/write/edit/list/grep/stat all live in the built-in `file_system`
// capability now that ercode plugs `RealDiskFileStore` as the runtime
// FileStore. The bash tool stays custom because the built-in `virtual_bash`
// runs commands against the VFS, not against the real workspace, and the
// security model for unsandboxed shell-on-host needs ercode-specific policy
// (timeout, output cap, approval gate). See EVE-478 for the eventual
// runtime-side story.

use crate::approval::{ApprovalGate, ApprovalRequest};
use async_trait::async_trait;
use everruns_core::tools::{Tool, ToolExecutionResult};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Workspace context for the bash tool. Just the root path — path
/// resolution for file ops now lives inside `RealDiskFileStore`.
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
            max_output_bytes: 64 * 1024,
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
        "Run a bash command from the workspace root. Captures stdout/stderr (truncated). 120s timeout. Requires user approval."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Shell command to run via bash -lc."}
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let command = match arguments.get("command").and_then(Value::as_str) {
            Some(c) => c.to_string(),
            None => return ToolExecutionResult::tool_error("'command' is required"),
        };
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
        let exit_code = status.ok().and_then(|s| s.code());
        ToolExecutionResult::success(json!({
            "command": command,
            "exit_code": exit_code,
            "stdout": stdout_text,
            "stderr": stderr_text,
            "stdout_truncated": out_truncated,
            "stderr_truncated": err_truncated,
        }))
    }
}
