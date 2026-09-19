//! The `bash` tool over real host processes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use everruns_containment::{ContainmentMode, SandboxOptions, SandboxProvider, policy};
use everruns_core::background::{
    BackgroundEventSink, BackgroundExecutableTool, BackgroundOutcome, BackgroundProgress,
};
use everruns_core::exec_tool_result::ExecToolResultPayload;
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase, narrate_shell_exec};
use everruns_core::tool_output_sanitizer::output_verbosity_schema;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_provider::tool_types::{DeferrablePolicy, ToolCall, ToolHints};
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;

use crate::approval::{DenyAll, HostShellApproval, ShellApprovalGate, ShellApprovalRequest};
use crate::config::{ApprovalPolicy, HostShellConfig};

/// The tool the `host_shell` capability contributes.
pub struct BashTool {
    config: HostShellConfig,
}

impl BashTool {
    /// The tool for `config`.
    pub fn new(config: HostShellConfig) -> Self {
        Self { config }
    }
}

impl Default for BashTool {
    fn default() -> Self {
        Self::new(HostShellConfig::default())
    }
}

/// What one child process did.
struct RunOutput {
    stdout: String,
    stderr: String,
    exit_code: i32,
    /// Output hit `max_output_bytes` and the command was killed.
    output_limited: bool,
    duration: Duration,
    containment: ContainmentMode,
}

/// Whether a failure looks like the containment blocked it.
///
/// A heuristic on purpose: the kernel reports a denied write as an ordinary
/// permission error, so nothing distinguishes it from a genuine one. It only
/// ever decides whether to *ask*, never whether to allow.
fn likely_containment_denial(mode: ContainmentMode, exit_code: i32, stderr: &str) -> bool {
    if mode.is_full_access() || exit_code == 0 {
        return false;
    }
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("operation not permitted") || stderr.contains("permission denied")
}

/// A hint for the failure modes a model otherwise burns turns rediscovering.
fn command_failure_hint(exit_code: i32, stderr: &str) -> Option<&'static str> {
    (exit_code == 127 && stderr.to_ascii_lowercase().contains("command not found")).then_some(
        "Command not found. Prefer a built-in tool when one exists (for repository search, \
         grep_files); otherwise use a broadly available fallback such as grep -R, or check PATH \
         and install the executable before retrying.",
    )
}

/// The host directory this session's commands run in.
///
/// Read from the session file store rather than configured separately, so the
/// shell and the structured file tools address the same bytes, and a store that
/// repoints (a worktree switch) moves both at once. A store that is not backed
/// by real disk has no host path to offer, which is a configuration error worth
/// naming rather than a directory worth inventing.
fn resolve_workspace(context: &ToolContext, working_dir: Option<&str>) -> Result<PathBuf, String> {
    let Some(store) = context.file_store.as_ref() else {
        return Err("host_shell needs a session filesystem; none is available here".to_string());
    };
    let requested = working_dir.unwrap_or("/");
    let Some(path) = store.host_path(requested) else {
        return Err(format!(
            "host_shell runs commands on this machine, but this session's workspace is not \
             backed by a directory on it (`{}`). Mount the workspace with a real-disk file \
             store, or use the sandboxed `bashkit_shell` capability instead.",
            store.display_path(&store.resolve_path(requested))
        ));
    };
    if !path.is_dir() {
        return Err(format!(
            "working directory does not exist: {}",
            store.display_path(&store.resolve_path(requested))
        ));
    }
    Ok(path)
}

impl BashTool {
    /// The gate a host installed, or one that refuses everything.
    fn approval_gate(context: &ToolContext) -> Arc<dyn ShellApprovalGate> {
        context
            .extensions
            .get::<HostShellApproval>()
            .map(|extension| extension.gate().clone())
            .unwrap_or_else(|| Arc::new(DenyAll))
    }

    async fn request_approval(
        gate: &Arc<dyn ShellApprovalGate>,
        command: &str,
        reason: &str,
        full_access: bool,
    ) -> Result<(), ToolExecutionResult> {
        let approved = gate
            .approve(ShellApprovalRequest {
                command: command.to_string(),
                reason: reason.to_string(),
                full_access,
            })
            .await;
        if approved {
            Ok(())
        } else {
            Err(ToolExecutionResult::tool_error(format!(
                "shell command was not approved: {reason}"
            )))
        }
    }

    /// Spawn the command and read both streams until they close, the byte
    /// budget runs out, or the timeout fires.
    async fn run(
        &self,
        command: &str,
        cwd: &Path,
        sandbox: &Arc<dyn SandboxProvider>,
        sink: Option<&Arc<dyn BackgroundEventSink>>,
    ) -> Result<RunOutput, ToolExecutionResult> {
        let timeout_secs = self.config.timeout_secs(sink.is_some());
        let max_bytes = self.config.max_output_bytes;
        let containment = sandbox.mode();

        if let Some(sink) = sink {
            let _ = sink.status("Running bash command").await;
        }

        let mut process = sandbox.command(cwd, command).map_err(|error| {
            ToolExecutionResult::tool_error(format!("containment setup failed: {error:#}"))
        })?;
        everruns_containment::configure_stdio(&mut process);
        let mut child = process
            .spawn()
            .map_err(|error| ToolExecutionResult::tool_error(format!("spawn failed: {error}")))?;
        // Both pipes are configured above, so neither handle can be absent.
        let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take())
        else {
            return Err(ToolExecutionResult::tool_error(
                "the shell process was spawned without pipes",
            ));
        };

        let start = Instant::now();
        let read = async {
            let mut out_buf = Vec::with_capacity(4096);
            let mut err_buf = Vec::with_capacity(4096);
            let mut out_chunk = vec![0u8; 4096];
            let mut err_chunk = vec![0u8; 4096];
            let mut limited = false;
            let mut out_done = false;
            let mut err_done = false;
            while !(out_done && err_done) {
                // Reading both streams concurrently is what keeps a command
                // that floods stderr from deadlocking on a full pipe.
                tokio::select! {
                    read = stdout.read(&mut out_chunk), if !out_done => match read {
                        Ok(0) | Err(_) => out_done = true,
                        Ok(count) => {
                            if accept(&mut out_buf, &out_chunk[..count], max_bytes, "stdout", sink).await {
                                limited = true;
                                let _ = child.start_kill();
                                break;
                            }
                        }
                    },
                    read = stderr.read(&mut err_chunk), if !err_done => match read {
                        Ok(0) | Err(_) => err_done = true,
                        Ok(count) => {
                            if accept(&mut err_buf, &err_chunk[..count], max_bytes, "stderr", sink).await {
                                limited = true;
                                let _ = child.start_kill();
                                break;
                            }
                        }
                    },
                }
            }
            (child.wait().await, out_buf, err_buf, limited)
        };

        let (status, out_buf, err_buf, output_limited) =
            match tokio::time::timeout(Duration::from_secs(timeout_secs), read).await {
                Ok(finished) => finished,
                Err(_) => {
                    // The timeout is where poll loops are born: a foreground
                    // watch dies here and the model falls back to
                    // sleep-and-recheck turns. Name the escape hatch instead.
                    return Err(ToolExecutionResult::tool_error(format!(
                        "command timed out after {timeout_secs}s. If it was waiting on an \
                         external event (a CI run, a deploy, a long build), run it detached and \
                         end the turn rather than polling."
                    )));
                }
            };

        Ok(RunOutput {
            stdout: String::from_utf8_lossy(&out_buf).into_owned(),
            stderr: String::from_utf8_lossy(&err_buf).into_owned(),
            // A signalled process has no code. -1 keeps `success` false without
            // inventing a plausible exit status.
            exit_code: status.ok().and_then(|status| status.code()).unwrap_or(-1),
            output_limited,
            duration: start.elapsed(),
            containment,
        })
    }

    /// Run `command` under the approval policy, escalating only where the
    /// policy and a host's gate both allow it.
    async fn execute_with_policy(
        &self,
        command: &str,
        cwd: &Path,
        context: &ToolContext,
        sink: Option<&Arc<dyn BackgroundEventSink>>,
        request_full_access: bool,
        justification: Option<&str>,
    ) -> Result<RunOutput, ToolExecutionResult> {
        // Defense in depth for a direct mistake. Kernel or VM isolation remains
        // the boundary against deliberately obscured signalling.
        if policy::can_signal_host(command, std::process::id(), &host_process_name()) {
            return Err(ToolExecutionResult::tool_error(
                "refusing to signal the agent host process",
            ));
        }

        let gate = Self::approval_gate(context);
        let sandbox = everruns_containment::provider(self.config.sandbox_options());

        if self.config.approval == ApprovalPolicy::Untrusted
            && !policy::is_trusted_read_only(command)
        {
            Self::request_approval(
                &gate,
                command,
                "command is outside the trusted read-only set",
                false,
            )
            .await?;
        }

        if self.config.containment.is_full_access() {
            // Nothing contains this already; there is no boundary left to
            // escalate past, so a request for one is satisfied by running.
            return self.run(command, cwd, &sandbox, sink).await;
        }

        match self.config.approval {
            ApprovalPolicy::OnRequest if request_full_access => {
                let reason = justification
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        ToolExecutionResult::tool_error(
                            "require_escalated needs a non-empty justification",
                        )
                    })?;
                Self::request_approval(&gate, command, reason, true).await?;
                self.run(command, cwd, &full_access(), sink).await
            }
            ApprovalPolicy::OnFailure => {
                let first = self.run(command, cwd, &sandbox, sink).await?;
                if !likely_containment_denial(first.containment, first.exit_code, &first.stderr) {
                    return Ok(first);
                }
                Self::request_approval(
                    &gate,
                    command,
                    "the command failed in a way the containment would explain; retry uncontained",
                    true,
                )
                .await?;
                self.run(command, cwd, &full_access(), sink).await
            }
            _ if request_full_access => Err(ToolExecutionResult::tool_error(format!(
                "approval `{}` does not allow escalating past the `{}` containment",
                self.config.approval.as_str(),
                self.config.containment.as_str()
            ))),
            _ => self.run(command, cwd, &sandbox, sink).await,
        }
    }
}

fn full_access() -> Arc<dyn SandboxProvider> {
    everruns_containment::provider(SandboxOptions::new(ContainmentMode::FullAccess))
}

/// This binary's file name, so `pkill -f <name>` is recognized as self-directed.
fn host_process_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_default()
}

/// Append a chunk, streaming it to a sink, and report whether the budget ran out.
async fn accept(
    buffer: &mut Vec<u8>,
    chunk: &[u8],
    max_bytes: usize,
    stream: &str,
    sink: Option<&Arc<dyn BackgroundEventSink>>,
) -> bool {
    let accepted = chunk.len().min(max_bytes.saturating_sub(buffer.len()));
    if accepted > 0 {
        buffer.extend_from_slice(&chunk[..accepted]);
        if let Some(sink) = sink {
            let _ = sink
                .output(stream, &String::from_utf8_lossy(&chunk[..accepted]))
                .await;
        }
    }
    accepted < chunk.len()
}

/// The script argument, under either spelling.
///
/// `command` is the name Yolop and every other harness uses; `commands` is what
/// `bashkit_shell` takes. Accepting both means swapping the execution backend
/// under an agent does not invalidate its prompt.
fn script_argument(arguments: &Value) -> Option<&str> {
    arguments
        .get("command")
        .or_else(|| arguments.get("commands"))
        .and_then(Value::as_str)
}

fn result_json(command: &str, output: &RunOutput, payload: &ExecToolResultPayload) -> Value {
    let mut result = json!({
        "command": command,
        "exit_code": payload.exit_code,
        "success": payload.success,
        "stdout": payload.stdout,
        "stderr": payload.stderr,
        "truncated": payload.truncated || output.output_limited,
        "total_lines": payload.total_lines,
        "output_limited": output.output_limited,
        "containment": output.containment.as_str(),
    });
    if likely_containment_denial(output.containment, payload.exit_code, &output.stderr) {
        result["containment_denial"] = json!("likely");
    }
    if let Some(hint) = command_failure_hint(payload.exit_code, &output.stderr) {
        result["hint"] = json!(hint);
    }
    result
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Bash")
    }

    fn narrate(
        &self,
        tool_call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _context: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_shell_exec(
            &tool_call.arguments,
            self.display_name().unwrap_or("Bash"),
            phase,
            locale,
        ))
    }

    fn description(&self) -> &str {
        #[cfg(windows)]
        {
            "Run a PowerShell command on this machine. Each call is a fresh non-interactive \
             shell already rooted at the workspace, so nothing persists between calls: the \
             working directory and shell variables reset every time. A bare `cd` is pointless, \
             you are already at the workspace root; use paths relative to it, or chain within \
             one call (`cd sub; cmd`)."
        }
        #[cfg(not(windows))]
        {
            "Run a bash command on this machine. Every call starts a fresh non-interactive \
             `bash -lc` already rooted at the workspace, so nothing persists between calls: the \
             working directory, shell variables, and exports reset every time. A bare `cd` is \
             pointless, you are already at the workspace root; use paths relative to it, or \
             chain within one call (`cd sub && cmd`). The tool reports the shell's exit status, \
             so unless your script handles an expected failure, structure it to return nonzero \
             when a step fails."
        }
    }

    fn parameters_schema(&self) -> Value {
        #[cfg(windows)]
        let command_description = "Shell command to run via PowerShell.";
        #[cfg(not(windows))]
        let command_description = "Shell command to run via bash -lc.";
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": command_description},
                "working_dir": {
                    "type": "string",
                    "description": "Directory to run in. Defaults to the workspace root."
                },
                "sandbox_permissions": {
                    "type": "string",
                    "enum": ["use_default", "require_escalated"],
                    "description": "Use require_escalated only when the command must run without \
                                    containment. The agent's approval policy decides whether a \
                                    human may be asked."
                },
                "justification": {
                    "type": "string",
                    "description": "Short user-facing reason for require_escalated."
                },
                "output": output_verbosity_schema()
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_open_world(true)
            .with_persist_output(true)
            .with_supports_background(true)
            // Commands mutate the shared workspace, so concurrent calls in one
            // batch are serialized rather than raced against each other.
            .with_concurrency_class("session_workspace")
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        // A hot-path tool whose exact input contract must stay visible.
        // Deferring it makes models substitute shell interfaces learned
        // elsewhere.
        DeferrablePolicy::Never
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "bash requires session context. This tool must be executed with a session workspace.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(command) = script_argument(&arguments) else {
            return ToolExecutionResult::tool_error("'command' is required");
        };
        let cwd = match resolve_workspace(
            context,
            arguments.get("working_dir").and_then(Value::as_str),
        ) {
            Ok(cwd) => cwd,
            Err(message) => return ToolExecutionResult::tool_error(message),
        };

        // `auto` is persistence-first: a compact summary on success, with the
        // full log in `/outputs/`, and a normal-sized diagnostic window on
        // failure. Explicit modes still override it.
        let output_mode = arguments
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or("auto");
        let request_full_access = arguments.get("sandbox_permissions").and_then(Value::as_str)
            == Some("require_escalated");

        let output = match self
            .execute_with_policy(
                command,
                &cwd,
                context,
                None,
                request_full_access,
                arguments.get("justification").and_then(Value::as_str),
            )
            .await
        {
            Ok(output) => output,
            Err(error) => return error,
        };

        let payload = ExecToolResultPayload::new(
            &output.stdout,
            &output.stderr,
            output.exit_code,
            output_mode,
        );
        let result = result_json(command, &output, &payload);
        ToolExecutionResult::success_with_raw_output(result, payload.raw_output)
    }

    fn as_background_executable(&self) -> Option<&dyn BackgroundExecutableTool> {
        Some(self)
    }
}

#[async_trait]
impl BackgroundExecutableTool for BashTool {
    async fn execute_background(
        &self,
        arguments: Value,
        context: ToolContext,
        sink: Arc<dyn BackgroundEventSink>,
    ) -> Result<BackgroundOutcome, ToolExecutionResult> {
        let Some(command) = script_argument(&arguments) else {
            return Err(ToolExecutionResult::tool_error("'command' is required"));
        };
        let cwd = resolve_workspace(
            &context,
            arguments.get("working_dir").and_then(Value::as_str),
        )
        .map_err(ToolExecutionResult::tool_error)?;
        let output_mode = arguments
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or("auto");
        let request_full_access = arguments.get("sandbox_permissions").and_then(Value::as_str)
            == Some("require_escalated");

        let output = self
            .execute_with_policy(
                command,
                &cwd,
                &context,
                Some(&sink),
                request_full_access,
                arguments.get("justification").and_then(Value::as_str),
            )
            .await?;

        let payload = ExecToolResultPayload::new(
            &output.stdout,
            &output.stderr,
            output.exit_code,
            output_mode,
        );
        let _ = sink
            .progress(BackgroundProgress {
                current: Some(output.duration.as_millis() as u64),
                total: None,
                unit: Some("ms".to_string()),
                label: Some("runtime".to_string()),
            })
            .await;
        Ok(BackgroundOutcome {
            summary: format!(
                "bash exited with code {} after {} ms",
                payload.exit_code,
                output.duration.as_millis()
            ),
            result: result_json(command, &output, &payload),
            raw_output: Some(payload.raw_output),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn either_spelling_of_the_script_argument_is_accepted() {
        assert_eq!(script_argument(&json!({"command": "ls"})), Some("ls"));
        assert_eq!(script_argument(&json!({"commands": "ls"})), Some("ls"));
        // `command` wins when a caller sends both.
        assert_eq!(
            script_argument(&json!({"command": "ls", "commands": "rm -rf /"})),
            Some("ls")
        );
        assert_eq!(script_argument(&json!({"working_dir": "/src"})), None);
    }

    #[test]
    fn a_denial_is_only_guessed_where_containment_could_explain_it() {
        assert!(likely_containment_denial(
            ContainmentMode::WorkspaceWrite,
            1,
            "bash: /etc/hosts: Permission denied"
        ));
        // Nothing was contained, so nothing the containment did explains this.
        assert!(!likely_containment_denial(
            ContainmentMode::FullAccess,
            1,
            "Permission denied"
        ));
        // A success is never a denial, whatever it printed.
        assert!(!likely_containment_denial(
            ContainmentMode::WorkspaceWrite,
            0,
            "permission denied"
        ));
        assert!(!likely_containment_denial(
            ContainmentMode::WorkspaceWrite,
            1,
            "error: no such file"
        ));
    }

    #[test]
    fn a_missing_executable_earns_a_hint_and_nothing_else_does() {
        assert!(command_failure_hint(127, "bash: rg: command not found").is_some());
        assert!(command_failure_hint(1, "bash: rg: command not found").is_none());
        assert!(command_failure_hint(127, "exited weirdly").is_none());
    }

    #[test]
    fn the_schema_asks_for_a_command_and_refuses_unknown_arguments() {
        let schema = BashTool::default().parameters_schema();
        assert_eq!(schema["required"], json!(["command"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert!(schema["properties"]["working_dir"].is_object());
        assert_eq!(
            schema["properties"]["sandbox_permissions"]["enum"],
            json!(["use_default", "require_escalated"])
        );
    }

    #[tokio::test]
    async fn the_output_budget_reports_when_it_runs_out() {
        let mut buffer = Vec::new();
        assert!(!accept(&mut buffer, b"hello", 8, "stdout", None).await);
        assert!(accept(&mut buffer, b" world", 8, "stdout", None).await);
        assert_eq!(buffer, b"hello wo");
    }

    #[tokio::test]
    async fn a_tool_call_without_session_context_is_refused() {
        let result = BashTool::default().execute(json!({"command": "ls"})).await;
        assert!(format!("{result:?}").contains("session context"));
    }
}
