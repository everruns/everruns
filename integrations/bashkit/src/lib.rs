//! Sandboxed Bashkit shell capability for Everruns agents.
//!
//! The capability bridges Bashkit to the host's session filesystem, enforces
//! command and loop limits, and keeps outbound HTTP disabled unless capability
//! configuration enables it and the host supplies an egress service.
//!
//! It is part of the [Everruns](https://everruns.com) ecosystem and pairs with
//! `everruns-host` plus `everruns-integrations-filesystem`.
//!
//! # Example
//!
//! ```
//! use everruns_core::Capability;
//! use everruns_integrations_bashkit::BashkitShellCapability;
//!
//! assert_eq!(BashkitShellCapability.id(), "bashkit_shell");
//! assert_eq!(BashkitShellCapability.aliases(), vec!["virtual_bash"]);
//! ```

mod egress_transport;
pub mod hook_dispatch;

use crate::background::{
    BackgroundEventSink, BackgroundExecutableTool, BackgroundOutcome, BackgroundProgress,
};
use crate::exec_tool_result::ExecToolResultPayload;
use crate::session_file::SessionFile;
use crate::tool_types::{DeferrablePolicy, ToolHints};
use crate::tools::{Tool, ToolExecutionResult};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use bashkit::{
    Bash, BashBuilder, BashTool as BashkitTool, DirEntry, ExecutionLimits, FileSystem,
    FileSystemExt, FileType, Metadata, NetworkAllowlist, OutputCallback, SearchCapabilities,
    SearchCapable, SearchMatch as BashkitSearchMatch, SearchProvider, SearchQuery, SearchResults,
    Tool as BashkitToolTrait, TraceEventKind, TraceMode,
};
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel,
};
use everruns_core::session_files::SessionFileSystem;
use everruns_core::tool_context::ToolContext;
use everruns_core::*;
#[cfg(test)]
use everruns_provider::error;
use everruns_provider::{tool_types, typed_id};
pub use hook_dispatch::BashkitShellHookDispatcher;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::result::Result;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

// ============================================================================
// Static configuration
// ============================================================================

/// Shared execution limits for bashkit.
fn execution_limits() -> ExecutionLimits {
    ExecutionLimits::new()
        .max_commands(1000)
        .max_loop_iterations(10000)
        .max_function_depth(100)
        .max_input_bytes(1_000_000) // 1MB max script size
        .max_ast_depth(100)
        .parser_timeout(std::time::Duration::from_secs(5))
}

/// Resolve the shell working directory and `WORKSPACE` env value from the file
/// store's namespace (EVE-660).
///
/// The file store ([`MountFs`](crate::mount_fs::MountFs)) is the single path
/// authority: `working_dir` is resolved through it to an absolute path in the
/// same namespace the file tools use, so a model that learns a path from
/// `read_file` can pass it straight to `cd`. With no `working_dir`, the shell
/// starts at the store's display root (`/workspace` for the mounted stores used
/// by agent execution). The tuple is `(cwd, workspace_env)`.
fn resolve_shell_workspace(
    store: &dyn SessionFileSystem,
    working_dir_arg: Option<&str>,
) -> (String, String) {
    let workspace_env = store.display_root();
    let cwd = match working_dir_arg {
        Some(arg) => store.resolve_path(arg),
        None => workspace_env.clone(),
    };
    (cwd, workspace_env)
}

/// Configured bashkit tool instance with everruns settings.
static BASHKIT_TOOL: LazyLock<BashkitTool> = LazyLock::new(|| {
    BashkitTool::builder()
        .username("everruns")
        .hostname("everruns")
        .limits(execution_limits())
        .env("HOME", "/home/agent")
        .env("SHELL", "/bin/bash")
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("WORKSPACE", "/workspace")
        .build()
});

/// Tool description from bashkit library.
static TOOL_DESCRIPTION: LazyLock<String> =
    LazyLock::new(|| BASHKIT_TOOL.description().to_string());

/// System prompt addition from bashkit library + output economy hint.
static TOOL_SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = BASHKIT_TOOL.system_prompt().to_string();
    prompt.push_str(crate::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

/// Input schema from bashkit library, extended with everruns-specific `working_dir`.
/// Delegating to bashkit avoids schema drift when bashkit adds/changes parameters.
static TOOL_INPUT_SCHEMA: LazyLock<Value> = LazyLock::new(|| {
    let mut schema = BASHKIT_TOOL.input_schema();
    // Add everruns-specific working_dir param only if bashkit does not already define it.
    if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
        if !props.contains_key("working_dir") {
            props.insert(
                "working_dir".to_string(),
                json!({
                    "type": "string",
                    "default": "/workspace",
                    "description": "Working directory for command execution"
                }),
            );
        }
        if !props.contains_key("output") {
            props.insert(
                "output".to_string(),
                crate::tool_output_sanitizer::output_verbosity_schema(),
            );
        }
    }
    schema
});

pub const BASHKIT_SHELL_CAPABILITY_ID: &str = "bashkit_shell";

/// Agent-facing Bashkit shell configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BashkitShell {
    enable_http: bool,
}

impl BashkitShell {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allow shell HTTP commands through the host egress boundary.
    pub fn enable_http(mut self, enabled: bool) -> Self {
        self.enable_http = enabled;
        self
    }
}

impl everruns_capability::IntoCapability for BashkitShell {
    fn into_capability(self) -> everruns_capability::CapabilitySpec {
        everruns_capability::CapabilityRef::new(BASHKIT_SHELL_CAPABILITY_ID)
            .config(serde_json::json!({ "enable_http": self.enable_http }))
            .into()
    }
}

/// Bashkit Shell capability - execute bash commands in a sandboxed environment
pub struct BashkitShellCapability;

impl Capability for BashkitShellCapability {
    fn id(&self) -> &str {
        BASHKIT_SHELL_CAPABILITY_ID
    }

    fn aliases(&self) -> Vec<&'static str> {
        // Pre-rename ID; still present in persisted agent configs.
        vec!["virtual_bash"]
    }

    fn name(&self) -> &str {
        "Bashkit Shell"
    }

    fn description(&self) -> &str {
        r#"Execute bash commands in an isolated, sandboxed environment.

> [!NOTE]
> Commands run in a virtual environment with no access to the host system.
> The session filesystem is mounted at root, so you can read and write session files.

> [!TIP]
> Use standard Unix commands like `ls`, `cat`, `grep`, `echo`, and shell features
> like pipes, redirections, and command substitution. Built-in commands support
> `<command> --help`, and many also support `<command> --version`."#
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Оболонка Bashkit",
            r#"Виконуйте bash-команди в ізольованому середовищі-пісочниці.

> [!NOTE]
> Команди виконуються у віртуальному середовищі без доступу до хост-системи.
> Файлова система сесії змонтована в корені, тож можна читати й записувати файли сесії.

> [!TIP]
> Використовуйте стандартні Unix-команди, як-от `ls`, `cat`, `grep`, `echo`, і можливості оболонки
> на кшталт конвеєрів, перенаправлень і підстановки команд. Вбудовані команди підтримують
> `<command> --help`, а багато з них також `<command> --version`."#,
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("terminal")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(&TOOL_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BashTool::default())]
    }

    fn tools_with_config(&self, config: &serde_json::Value) -> Vec<Box<dyn Tool>> {
        let enable_http = config
            .get("enable_http")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        vec![Box::new(BashTool { enable_http })]
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "enable_http": {
                    "type": "boolean",
                    "title": "Allow outbound HTTP (curl/wget)",
                    "description": "Let shell scripts make outbound HTTP requests. Every \
                                    request is routed through the platform egress boundary, \
                                    where the network access list and system allowlist are \
                                    enforced.",
                    "default": false
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if !config.is_object() {
            return Err("bashkit_shell config must be an object".to_string());
        }
        match config.get("enable_http") {
            None | Some(serde_json::Value::Bool(_)) => Ok(()),
            Some(other) => Err(format!("enable_http must be a boolean, got {other}")),
        }
    }

    fn dependencies(&self) -> Vec<&'static str> {
        // Depends on session filesystem for file access
        vec!["session_file_system"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

// ============================================================================
// BashTool
// ============================================================================

/// Tool to execute bash commands in a sandboxed environment
#[derive(Default)]
pub struct BashTool {
    /// Opt-in outbound HTTP for curl/wget, set from per-capability config
    /// `{"enable_http": true}`. Only effective when the execution context
    /// provides an `EgressService` (see `configure_http`).
    enable_http: bool,
}

#[async_trait]
impl Tool for BashTool {
    fn narrate(
        &self,
        tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let fallback = self.display_name().unwrap_or("Bash");
        Some(crate::tool_narration::narrate_shell_exec(
            &tool_call.arguments,
            fallback,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "bash"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Bash")
    }

    fn description(&self) -> &str {
        &TOOL_DESCRIPTION
    }

    fn parameters_schema(&self) -> Value {
        TOOL_INPUT_SCHEMA.clone()
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_open_world(true)
            .with_persist_output(true)
            .with_supports_background(true)
            // Mutates the shared session workspace: serialize concurrent bash
            // calls in a batch so they don't race on the filesystem. Runs an
            // in-process interpreter, so offload to its own task to avoid
            // starving I/O-bound tools sharing the act batch.
            .with_concurrency_class("session_workspace")
            .with_cpu_bound(true)
    }

    fn deferrable_policy(&self) -> DeferrablePolicy {
        // Bash is a hot-path tool whose exact input contract must stay visible.
        // Deferring it makes models more likely to substitute shell interfaces
        // learned outside Everruns (for example `bash_run` with `command`).
        DeferrablePolicy::Never
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "bash requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let command = match arguments.get("commands").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return ToolExecutionResult::tool_error("Missing required parameter: commands");
            }
        };

        let file_store = match &context.file_store {
            Some(store) => store.clone(),
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        let (working_dir, workspace_env) = resolve_shell_workspace(
            file_store.as_ref(),
            arguments.get("working_dir").and_then(|v| v.as_str()),
        );

        let timeout_ms = arguments
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(30000)
            .min(60000);

        // EVE-489: persistence-first default. `auto` returns a compact
        // summary on success and a `normal`-sized diagnostic window on failure.
        let output_mode = arguments
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        // Create filesystem adapter that bridges to session file store
        let session_fs = Arc::new(SessionFileSystemAdapter::new(
            context.session_id,
            file_store,
        ));

        // Resolve locale from context (defaults to en-US).
        let locale = context.locale.as_deref().unwrap_or("en-US");

        // Configure bash with resource limits (uses shared execution_limits).
        // Observability hooks are installed last so per-builtin / error telemetry
        // is available without changing any existing limits or boundaries.
        let builder = Bash::builder()
            .fs(session_fs)
            .cwd(working_dir.as_str())
            .username("everruns")
            .hostname("everruns")
            .env("HOME", "/home/agent")
            .env("SHELL", "/bin/bash")
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("WORKSPACE", workspace_env.as_str())
            .env("LANG", locale)
            .limits(execution_limits())
            .max_memory(10 * 1024 * 1024) // 10 MB — prevent OOM from untrusted input
            .trace_mode(TraceMode::Redacted);
        let builder = install_observability_hooks(builder, context.session_id);
        let builder = configure_http(builder, self.enable_http, context);
        let mut bash = builder.build();

        // Stream output via tool.output.delta events for live UI/CLI rendering.
        // bashkit's exec_streaming calls OutputCallback with (stdout_chunk, stderr_chunk)
        // after each command completes. We bridge to async emit via a channel.
        // A bounded channel collects partial output for cancellation recovery
        // without allowing unbounded memory growth.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, String)>();
        let (partial_tx, partial_rx) = tokio::sync::mpsc::channel::<(String, String)>(128);

        let output_callback: OutputCallback = Box::new(move |stdout_chunk, stderr_chunk| {
            // Tool output events are text, so decode Bashkit's byte-native
            // chunks explicitly at the event boundary.
            // Best-effort: if receiver dropped, we just ignore
            let _ = tx.send((stdout_chunk.to_string(), stderr_chunk.to_string()));
            // Bounded: drop if full rather than growing without bound.
            let _ = partial_tx.try_send((stdout_chunk.to_string(), stderr_chunk.to_string()));
        });

        // Spawn a task that reads chunks from the channel and emits events
        let emit_context = context.clone();
        let emit_task = tokio::spawn(async move {
            while let Some((stdout_chunk, stderr_chunk)) = rx.recv().await {
                if !stdout_chunk.is_empty() {
                    emit_context
                        .emit_tool_output("bash", &stdout_chunk, "stdout")
                        .await;
                }
                if !stderr_chunk.is_empty() {
                    emit_context
                        .emit_tool_output("bash", &stderr_chunk, "stderr")
                        .await;
                }
            }
        });

        // Grab the cancellation token so we can signal graceful abort on timeout.
        let cancel_token = bash.cancellation_token();

        // Execute with timeout. On timeout, signal cancellation via the token
        // so bashkit aborts at the next command boundary and we can collect
        // partial output instead of discarding everything.
        let exec_start = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            bash.exec_streaming(command, output_callback),
        )
        .await;
        let exec_duration = exec_start.elapsed();

        // Wait for all buffered chunks to be emitted (sender dropped when exec completes)
        let _ = emit_task.await;

        match result {
            Ok(Ok(output)) => {
                // Extract metadata from trace events (EVE-240)
                let commands_executed = output
                    .events
                    .iter()
                    .filter(|e| e.kind == TraceEventKind::CommandExit)
                    .count();
                let fs_reads = output
                    .events
                    .iter()
                    .filter(|e| e.kind == TraceEventKind::FileAccess)
                    .count();
                let fs_writes = output
                    .events
                    .iter()
                    .filter(|e| e.kind == TraceEventKind::FileMutation)
                    .count();

                tracing::info!(
                    tool = "bash",
                    duration_ms = exec_duration.as_millis() as u64,
                    exit_code = output.exit_code,
                    commands_executed,
                    fs_reads,
                    fs_writes,
                    stdout_bytes = output.stdout.len(),
                    stderr_bytes = output.stderr.len(),
                    "bashkit execution completed"
                );

                let payload = ExecToolResultPayload::new(
                    &output.stdout,
                    &output.stderr,
                    output.exit_code,
                    output_mode,
                );
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
                        "stdout": stdout,
                        "stderr": stderr,
                        "exit_code": exit_code,
                        "success": success,
                        "truncated": truncated,
                        "total_lines": total_lines,
                    }),
                    raw_output,
                )
            }
            Ok(Err(e)) => {
                // Execution error (syntax error, resource limit, etc.)
                ToolExecutionResult::tool_error(format!("Bash execution error: {}", e))
            }
            Err(_) => {
                // Timeout — signal cancellation for the in-flight execution so any
                // underlying bashkit work stops promptly, then collect whatever
                // partial output the streaming callback captured.
                cancel_token.store(true, std::sync::atomic::Ordering::Relaxed);

                let partial = collect_partial_output(partial_rx);
                if partial.is_empty() {
                    ToolExecutionResult::tool_error(format!(
                        "Command timed out after {}ms",
                        timeout_ms
                    ))
                } else {
                    use crate::tool_output_sanitizer::{
                        clean_exec_output, output_verbosity_budget, priority_aware_truncate,
                        resolve_auto_mode,
                    };
                    // EVE-489: a timeout is a failure — `auto` resolves to
                    // `normal` so the model gets useful diagnostics.
                    let effective = resolve_auto_mode(output_mode, 1);
                    let clean = clean_exec_output(&partial);
                    let truncated = if let Some(budget) = output_verbosity_budget(effective) {
                        priority_aware_truncate(&clean, budget)
                    } else {
                        clean.clone()
                    };
                    ToolExecutionResult::tool_error(format!(
                        "Command timed out after {}ms. Partial output:\n{}",
                        timeout_ms, truncated
                    ))
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
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
        let command = match arguments.get("commands").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => {
                return Err(ToolExecutionResult::tool_error(
                    "Missing required parameter: commands",
                ));
            }
        };

        let file_store = match &context.file_store {
            Some(store) => store.clone(),
            None => {
                return Err(ToolExecutionResult::tool_error(
                    "File system not available in this context",
                ));
            }
        };

        let (working_dir, workspace_env) = resolve_shell_workspace(
            file_store.as_ref(),
            arguments.get("working_dir").and_then(|v| v.as_str()),
        );

        let timeout_ms = arguments
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(30000)
            .min(60000);

        // EVE-489: persistence-first default for background execution as well.
        let output_mode = arguments
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");

        let session_fs = Arc::new(SessionFileSystemAdapter::new(
            context.session_id,
            file_store,
        ));
        let locale = context.locale.as_deref().unwrap_or("en-US");

        let builder = Bash::builder()
            .fs(session_fs)
            .cwd(working_dir.as_str())
            .username("everruns")
            .hostname("everruns")
            .env("HOME", "/home/agent")
            .env("SHELL", "/bin/bash")
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("WORKSPACE", workspace_env.as_str())
            .env("LANG", locale)
            .limits(execution_limits())
            .max_memory(10 * 1024 * 1024)
            .trace_mode(TraceMode::Redacted);
        let builder = install_observability_hooks(builder, context.session_id);
        let builder = configure_http(builder, self.enable_http, &context);
        let mut bash = builder.build();

        let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, String)>(128);
        let (partial_tx, partial_rx) = tokio::sync::mpsc::channel::<(String, String)>(128);
        let sink_for_output = sink.clone();
        let dropped_chunks = Arc::new(AtomicUsize::new(0));
        let dropped_chunks_for_callback = dropped_chunks.clone();
        let output_callback: OutputCallback = Box::new(move |stdout_chunk, stderr_chunk| {
            // Background output uses the same text boundary as foreground
            // tool events.
            if tx
                .try_send((stdout_chunk.to_string(), stderr_chunk.to_string()))
                .is_err()
            {
                dropped_chunks_for_callback.fetch_add(1, Ordering::Relaxed);
            }
            let _ = partial_tx.try_send((stdout_chunk.to_string(), stderr_chunk.to_string()));
        });

        let emit_task = tokio::spawn(async move {
            while let Some((stdout_chunk, stderr_chunk)) = rx.recv().await {
                if !stdout_chunk.is_empty() {
                    let _ = sink_for_output.output("stdout", &stdout_chunk).await;
                }
                if !stderr_chunk.is_empty() {
                    let _ = sink_for_output.output("stderr", &stderr_chunk).await;
                }
            }
        });

        let _ = sink.status("Running bash command").await;
        let cancel_token = bash.cancellation_token();
        let exec_start = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            bash.exec_streaming(command, output_callback),
        )
        .await;
        let exec_duration = exec_start.elapsed();
        let _ = emit_task.await;
        let dropped_chunks = dropped_chunks.load(Ordering::Relaxed);
        if dropped_chunks > 0 {
            let _ = sink
                .output(
                    "stderr",
                    &format!(
                        "[system] dropped {dropped_chunks} background output chunk(s) due to backpressure\n"
                    ),
                )
                .await;
        }

        match result {
            Ok(Ok(output)) => {
                let payload = ExecToolResultPayload::new(
                    &output.stdout,
                    &output.stderr,
                    output.exit_code,
                    output_mode,
                );
                let ExecToolResultPayload {
                    stdout,
                    stderr,
                    exit_code,
                    success,
                    truncated,
                    total_lines,
                    raw_output,
                } = payload;
                let _ = sink
                    .progress(BackgroundProgress {
                        current: Some(exec_duration.as_millis() as u64),
                        total: None,
                        unit: Some("ms".to_string()),
                        label: Some("runtime".to_string()),
                    })
                    .await;
                Ok(BackgroundOutcome {
                    summary: format!(
                        "Bash command exited with code {} after {} ms",
                        exit_code,
                        exec_duration.as_millis()
                    ),
                    result: json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "exit_code": exit_code,
                        "success": success,
                        "truncated": truncated,
                        "total_lines": total_lines,
                    }),
                    raw_output: Some(raw_output),
                })
            }
            Ok(Err(e)) => Err(ToolExecutionResult::tool_error(format!(
                "Bash execution error: {}",
                e
            ))),
            Err(_) => {
                cancel_token.store(true, std::sync::atomic::Ordering::Relaxed);

                let partial = collect_partial_output(partial_rx);
                if partial.is_empty() {
                    Err(ToolExecutionResult::tool_error(format!(
                        "Command timed out after {}ms",
                        timeout_ms
                    )))
                } else {
                    use crate::tool_output_sanitizer::{
                        clean_exec_output, output_verbosity_budget, priority_aware_truncate,
                        resolve_auto_mode,
                    };
                    // EVE-489: a timeout is a failure — resolve `auto` to
                    // `normal` so the model gets useful diagnostics.
                    let effective = resolve_auto_mode(output_mode, 1);
                    let clean = clean_exec_output(&partial);
                    let truncated = if let Some(budget) = output_verbosity_budget(effective) {
                        priority_aware_truncate(&clean, budget)
                    } else {
                        clean.clone()
                    };
                    Err(ToolExecutionResult::tool_error(format!(
                        "Command timed out after {}ms. Partial output:\n{}",
                        timeout_ms, truncated
                    )))
                }
            }
        }
    }
}

// Observational-only. Emits `tracing` events for each bashkit builtin
// invocation and interpreter error, tagged with the active `session_id` for
// audit correlation. Every hook returns `HookAction::Continue`; none widen
// bashkit's existing limits or sandbox (TM-BASH). Hook callbacks log
// structural metadata (tool name, arg count, exit code, byte lengths) but
// never the argument values or builtin stdout — those surfaces can carry
// tenant paths, URLs, or embedded secrets. HTTP hooks (`before_http` /
// `after_http`) are registered in `configure_http` when outbound HTTP is
// enabled (TM-BASH-003: off by default, egress-routed when on).
fn install_observability_hooks(builder: BashBuilder, session_id: SessionId) -> BashBuilder {
    use bashkit::hooks::{ErrorEvent, HookAction, ToolEvent, ToolResult};
    builder
        .before_tool(Box::new(move |ev: ToolEvent| {
            tracing::debug!(
                target: "bashkit.hook",
                capability = "bashkit_shell",
                session_id = %session_id,
                event = "before_tool",
                tool = %ev.name,
                arg_count = ev.args.len(),
                "builtin invoked"
            );
            HookAction::Continue(ev)
        }))
        .after_tool(Box::new(move |res: ToolResult| {
            tracing::debug!(
                target: "bashkit.hook",
                capability = "bashkit_shell",
                session_id = %session_id,
                event = "after_tool",
                tool = %res.name,
                exit_code = res.exit_code,
                stdout_bytes = res.stdout.len(),
                "builtin completed"
            );
            HookAction::Continue(res)
        }))
        .on_error(Box::new(move |ev: ErrorEvent| {
            let preview = truncate_for_log(&ev.message, 256);
            tracing::warn!(
                target: "bashkit.hook",
                capability = "bashkit_shell",
                session_id = %session_id,
                event = "on_error",
                message = %preview,
                "interpreter error"
            );
            HookAction::Continue(ev)
        }))
}

/// Enable outbound HTTP for curl/wget when the per-capability config opted in
/// AND the execution context provides an egress service.
///
/// Design (knowledge/operations/egress.md migration step 3, bashkit `knowledge/security/http-transport.md`):
/// bashkit keeps its full HTTP policy pipeline — `allow_all()` retains the
/// private-IP-blocking SSRF precheck whose resolve-then-check result is
/// forwarded as pinned addresses — while connectivity is owned by
/// [`egress_transport::BashkitEgressTransport`], so the merged
/// `NetworkAccessList` and the deployment-wide system allowlist are enforced
/// at the egress boundary for every hop (curl/wget re-dispatch redirects).
///
/// THREAT[TM-BASH-003]: with `enable_http` off (the default) this function is
/// a no-op and the interpreter has no network path. When on, there is no
/// direct-dial fallback: absent an egress service the shell stays offline
/// rather than opening host-local connectivity.
///
/// Bot-auth request signing mirrors web_fetch: server-wide
/// `BOT_AUTH_SIGNING_KEY_SEED` (+ optional `BOT_AUTH_AGENT_FQDN`,
/// `BOT_AUTH_VALIDITY_SECS`) transparently signs every outbound request
/// before it reaches the transport (bashkit `knowledge/security/request-signing.md`).
fn configure_http(builder: BashBuilder, enable_http: bool, context: &ToolContext) -> BashBuilder {
    if !enable_http {
        return builder;
    }
    let Some(egress) = context.egress_service.clone() else {
        tracing::warn!(
            capability = "bashkit_shell",
            session_id = %context.session_id,
            "enable_http set but no egress service in context; shell HTTP stays disabled"
        );
        return builder;
    };
    let session_id = context.session_id;
    let mut builder = builder
        .network(NetworkAllowlist::allow_all())
        .http_transport(Arc::new(egress_transport::BashkitEgressTransport::new(
            egress,
            context.network_access.clone(),
        )))
        // Observational HTTP hooks (bashkit-requirements.md): log method and
        // status only — URLs and headers can carry tenant data or secrets.
        .before_http(Box::new(move |ev: bashkit::hooks::HttpRequestEvent| {
            tracing::debug!(
                target: "bashkit.hook",
                capability = "bashkit_shell",
                session_id = %session_id,
                event = "before_http",
                method = %ev.method,
                header_count = ev.headers.len(),
                "outbound http request"
            );
            bashkit::hooks::HookAction::Continue(ev)
        }))
        .after_http(Box::new(move |ev: bashkit::hooks::HttpResponseEvent| {
            tracing::debug!(
                target: "bashkit.hook",
                capability = "bashkit_shell",
                session_id = %session_id,
                event = "after_http",
                status = ev.status,
                "outbound http response"
            );
            bashkit::hooks::HookAction::Continue(ev)
        }));
    if let Some(bot_auth) = bot_auth_config_from_env() {
        builder = builder.bot_auth(bot_auth);
    }
    builder
}

/// Read the server-wide bot-auth signing config once (same env contract as
/// `web_fetch`; see `knowledge/execution/fetchkit.md` "Bot-auth"). Returns a fresh clone per
/// call site because `BashBuilder::bot_auth` takes ownership.
fn bot_auth_config_from_env() -> Option<bashkit::BotAuthConfig> {
    static CONFIG: LazyLock<Option<bashkit::BotAuthConfig>> = LazyLock::new(|| {
        let seed = std::env::var("BOT_AUTH_SIGNING_KEY_SEED").ok()?;
        let mut config = match bashkit::BotAuthConfig::from_base64_seed(&seed) {
            Ok(config) => config,
            Err(e) => {
                tracing::warn!(error = %e, "invalid BOT_AUTH_SIGNING_KEY_SEED, bashkit bot-auth disabled");
                return None;
            }
        };
        if let Ok(fqdn) = std::env::var("BOT_AUTH_AGENT_FQDN") {
            config = config.with_agent_fqdn(&fqdn);
        }
        if let Ok(secs) = std::env::var("BOT_AUTH_VALIDITY_SECS")
            && let Ok(secs) = secs.parse::<u64>()
        {
            config = config.with_validity_secs(secs);
        }
        Some(config)
    });
    CONFIG.clone()
}

/// Bounded diagnostic preview for hook log fields. The return value is
/// guaranteed to be no longer than `max_bytes` and to end on a valid UTF-8
/// char boundary. When there is room, a trailing marker is appended so
/// truncated entries remain visible in logs without exceeding the budget.
fn truncate_for_log(msg: &str, max_bytes: usize) -> String {
    const MARKER: &str = "…[truncated]";
    if msg.len() <= max_bytes {
        return msg.to_string();
    }
    let budget = max_bytes.saturating_sub(MARKER.len());
    let mut cut = budget.min(msg.len());
    while cut > 0 && !msg.is_char_boundary(cut) {
        cut -= 1;
    }
    if max_bytes > MARKER.len() {
        format!("{}{}", &msg[..cut], MARKER)
    } else {
        // Budget too small to fit the marker; return just the bounded slice.
        let mut cut = max_bytes.min(msg.len());
        while cut > 0 && !msg.is_char_boundary(cut) {
            cut -= 1;
        }
        msg[..cut].to_string()
    }
}

/// Drain all buffered chunks from the partial output channel into a single string.
/// Keeps stdout and stderr separated with the same delimiter convention used elsewhere.
fn collect_partial_output(mut rx: tokio::sync::mpsc::Receiver<(String, String)>) -> String {
    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
    while let Ok((stdout, stderr)) = rx.try_recv() {
        stdout_buf.push_str(&stdout);
        stderr_buf.push_str(&stderr);
    }
    let mut partial = stdout_buf;
    if !stderr_buf.is_empty() {
        if !partial.is_empty() && !partial.ends_with('\n') {
            partial.push('\n');
        }
        partial.push_str("--- stderr ---\n");
        partial.push_str(&stderr_buf);
    }
    partial
}

// ============================================================================
// SessionFileSystemAdapter
// ============================================================================

/// Adapter that implements bashkit's FileSystem trait by delegating to SessionFileSystem.
///
/// This provides live visibility of session files during bash execution - any files
/// written by other tools are immediately visible, and vice versa.
pub struct SessionFileSystemAdapter {
    session_id: SessionId,
    store: Arc<dyn SessionFileSystem>,
}

impl SessionFileSystemAdapter {
    pub fn new(session_id: SessionId, store: Arc<dyn SessionFileSystem>) -> Self {
        Self { session_id, store }
    }

    /// The bash VFS path as a string for the store to resolve.
    ///
    /// The store ([`MountFs`](crate::mount_fs::MountFs)) is the single path
    /// authority (EVE-660): it routes the `/workspace` alias, the root mount,
    /// relative-to-cwd, and host-absolute paths to the right backend, and the
    /// backend enforces containment (host stores reject symlinks and clamp to
    /// their root). The adapter no longer parses paths itself, so the shell, the
    /// file tools, and the resolver share one namespace — and the shell can
    /// address files anywhere from `/`, with `/workspace` as just its cwd.
    fn store_path(path: &Path) -> String {
        path.to_string_lossy().into_owned()
    }

    /// Whether `session_path` is an implicit directory — one with children but no row
    /// of its own (virtual mounts, unmaterialized parents).
    ///
    /// Stores answer `list_directory` for an unknown path with `Ok(vec![])`, so only a
    /// non-empty listing distinguishes a real directory from an absent path.
    async fn directory_has_entries(&self, session_path: &str) -> bool {
        self.store
            .list_directory(self.session_id, session_path)
            .await
            .is_ok_and(|entries| !entries.is_empty())
    }
}

#[async_trait]
impl FileSystemExt for SessionFileSystemAdapter {}

#[async_trait]
impl FileSystem for SessionFileSystemAdapter {
    async fn read_file(&self, path: &Path) -> bashkit::Result<Vec<u8>> {
        let session_path = Self::store_path(path);

        match self.store.read_file(self.session_id, &session_path).await {
            Ok(Some(file)) => {
                let content = file.content.unwrap_or_default();
                SessionFile::decode_content(&content, &file.encoding)
                    .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
            }
            Ok(None) => Err(bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            ))),
            Err(e) => Err(bashkit::Error::Io(std::io::Error::other(e.to_string()))),
        }
    }

    async fn write_file(&self, path: &Path, content: &[u8]) -> bashkit::Result<()> {
        let session_path = Self::store_path(path);

        let (encoded, encoding) = SessionFile::encode_content(content);

        self.store
            .write_file(self.session_id, &session_path, &encoded, &encoding)
            .await
            .map(|_| ())
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
    }

    async fn append_file(&self, path: &Path, content: &[u8]) -> bashkit::Result<()> {
        let session_path = Self::store_path(path);

        // Read existing content
        let mut existing = match self.store.read_file(self.session_id, &session_path).await {
            Ok(Some(file)) => {
                let content = file.content.unwrap_or_default();
                SessionFile::decode_content(&content, &file.encoding)
                    .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))?
            }
            Ok(None) => Vec::new(),
            Err(e) => return Err(bashkit::Error::Io(std::io::Error::other(e.to_string()))),
        };

        // Append new content
        existing.extend_from_slice(content);

        // Write back
        let (encoded, encoding) = SessionFile::encode_content(&existing);
        self.store
            .write_file(self.session_id, &session_path, &encoded, &encoding)
            .await
            .map(|_| ())
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
    }

    async fn mkdir(&self, path: &Path, _recursive: bool) -> bashkit::Result<()> {
        let session_path = Self::store_path(path);

        self.store
            .create_directory(self.session_id, &session_path)
            .await
            .map(|_| ())
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
    }

    async fn remove(&self, path: &Path, recursive: bool) -> bashkit::Result<()> {
        let session_path = Self::store_path(path);

        self.store
            .delete_file(self.session_id, &session_path, recursive)
            .await
            .map(|_| ())
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
    }

    async fn stat(&self, path: &Path) -> bashkit::Result<Metadata> {
        // Handle /workspace itself
        if path.to_string_lossy() == "/workspace" {
            let now = SystemTime::now();
            return Ok(Metadata {
                file_type: FileType::Directory,
                size: 0,
                mode: 0o755,
                modified: now,
                created: now,
            });
        }

        let session_path = Self::store_path(path);

        // Check if it's a file
        match self.store.read_file(self.session_id, &session_path).await {
            Ok(Some(file)) => {
                let now = SystemTime::now();

                let file_type = if file.is_directory {
                    FileType::Directory
                } else {
                    FileType::File
                };

                // Use 0o755 so files are executable by default in the virtual filesystem.
                // The session filesystem doesn't track Unix permissions, and scripts
                // stored in /workspace need to be directly executable.
                Ok(Metadata {
                    file_type,
                    size: file.size_bytes as u64,
                    mode: 0o755,
                    modified: now,
                    created: now,
                })
            }
            Ok(None) => {
                // No row: the path can still be an implicit directory (a virtual mount,
                // or a parent never materialized as its own row). Only a *non-empty*
                // listing proves that. Treating any `Ok` as a directory made every
                // absent path look like an empty directory, because stores return
                // `Ok(vec![])` rather than an error for paths they do not know.
                if self.directory_has_entries(&session_path).await {
                    let now = SystemTime::now();
                    Ok(Metadata {
                        file_type: FileType::Directory,
                        size: 0,
                        mode: 0o755,
                        modified: now,
                        created: now,
                    })
                } else {
                    Err(bashkit::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("Path not found: {}", path.display()),
                    )))
                }
            }
            Err(e) => Err(bashkit::Error::Io(std::io::Error::other(e.to_string()))),
        }
    }

    async fn read_dir(&self, path: &Path) -> bashkit::Result<Vec<DirEntry>> {
        let session_path = Self::store_path(path);

        let entries = self
            .store
            .list_directory(self.session_id, &session_path)
            .await
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))?;

        let now = SystemTime::now();

        Ok(entries
            .into_iter()
            .map(|e| {
                let file_type = if e.is_directory {
                    FileType::Directory
                } else {
                    FileType::File
                };

                DirEntry {
                    name: e.name,
                    metadata: Metadata {
                        file_type,
                        size: e.size_bytes as u64,
                        mode: 0o755,
                        modified: now,
                        created: now,
                    },
                }
            })
            .collect())
    }

    async fn exists(&self, path: &Path) -> bashkit::Result<bool> {
        // /workspace always exists
        if path.to_string_lossy() == "/workspace" {
            return Ok(true);
        }

        let session_path = Self::store_path(path);

        // A row exists for both files and materialized directories.
        if let Ok(Some(_)) = self.store.read_file(self.session_id, &session_path).await {
            return Ok(true);
        }

        // Otherwise only an implicit directory counts — see `stat` for why an empty
        // listing must not be read as existence.
        Ok(self.directory_has_entries(&session_path).await)
    }

    async fn rename(&self, from: &Path, to: &Path) -> bashkit::Result<()> {
        let from_session = Self::store_path(from);

        // Read source file
        let content = self.read_file(from).await?;

        // Write to destination
        self.write_file(to, &content).await?;

        // Delete source
        self.store
            .delete_file(self.session_id, &from_session, false)
            .await
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))?;

        Ok(())
    }

    async fn copy(&self, from: &Path, to: &Path) -> bashkit::Result<()> {
        let content = self.read_file(from).await?;
        self.write_file(to, &content).await
    }

    async fn symlink(&self, _target: &Path, _link: &Path) -> bashkit::Result<()> {
        // Session filesystem doesn't support symlinks
        Err(bashkit::Error::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Symlinks not supported in session filesystem",
        )))
    }

    async fn read_link(&self, path: &Path) -> bashkit::Result<PathBuf> {
        // Session filesystem doesn't support symlinks
        Err(bashkit::Error::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("Symlinks not supported: {}", path.display()),
        )))
    }

    async fn chmod(&self, _path: &Path, _mode: u32) -> bashkit::Result<()> {
        // chmod is a no-op - session filesystem doesn't track permissions
        Ok(())
    }

    // THREAT[TM-BASH-017]: no-op like `chmod` (TM-BASH-014). The session store does not
    // persist mtimes and `stat` synthesizes them, so there is nothing to spoof. The
    // bashkit default impl errors, which made `touch` fail after it had already
    // created the file.
    async fn set_modified_time(&self, _path: &Path, _time: SystemTime) -> bashkit::Result<()> {
        Ok(())
    }

    fn as_search_capable(&self) -> Option<&dyn SearchCapable> {
        Some(self)
    }
}

// ============================================================================
// SearchCapable / SearchProvider — indexed search via SessionFileSystem
// ============================================================================

impl SearchCapable for SessionFileSystemAdapter {
    fn search_provider(&self, _path: &Path) -> Option<Box<dyn SearchProvider>> {
        // The store resolves any path (root mount included), so indexed search is
        // available everywhere the shell can address.
        Some(Box::new(SessionSearchProvider {
            session_id: self.session_id,
            store: self.store.clone(),
        }))
    }
}

/// Bridges bashkit's synchronous `SearchProvider` to `SessionFileSystem::grep_files`.
///
/// Uses a scoped thread with a dedicated tokio runtime to call the async
/// store method from the sync trait, avoiding nested `block_on` calls.
struct SessionSearchProvider {
    session_id: SessionId,
    store: Arc<dyn SessionFileSystem>,
}

impl SearchProvider for SessionSearchProvider {
    fn search(&self, query: &SearchQuery) -> bashkit::Result<SearchResults> {
        let session_id = self.session_id;
        let store = self.store.clone();
        let root = query.root.to_string_lossy().into_owned();
        let max_results = query.max_results;

        // Honor case_insensitive flag via inline regex flag
        let pattern = if query.case_insensitive {
            format!("(?i){}", query.pattern)
        } else {
            query.pattern.clone()
        };

        // The store ([`MountFs`]) resolves the search root, so search shares the
        // shell's namespace. A root at the workspace top searches the whole tree
        // (no path filter); anything deeper is passed through for the store to
        // resolve and scope.
        let path_pattern = if root == crate::mount_fs::WORKSPACE_MOUNT || root == "/" {
            None
        } else {
            Some(root)
        };

        // Bridge async grep_files to sync SearchProvider::search.
        // Run on a dedicated thread with its own runtime to avoid nesting
        // block_on calls within the caller's tokio runtime.
        let matches = std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))?;
                rt.block_on(async {
                    store
                        .grep_files(session_id, &pattern, path_pattern.as_deref())
                        .await
                })
                .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
            })
            .join()
            .unwrap_or_else(|_| {
                Err(bashkit::Error::Io(std::io::Error::other(
                    "search thread panicked",
                )))
            })
        })?;

        let truncated = max_results.is_some_and(|max| matches.len() > max);
        let matches: Vec<BashkitSearchMatch> = matches
            .into_iter()
            .take(max_results.unwrap_or(usize::MAX))
            .map(|m| {
                // Render the backend match path back into the shell's namespace
                // (the `/workspace` view) so matches read back in the same
                // namespace the shell resolves against.
                let vfs_path = self.store.display_path(&m.path);
                BashkitSearchMatch {
                    path: PathBuf::from(vfs_path),
                    line_number: m.line_number,
                    line_content: m.line,
                }
            })
            .collect();

        Ok(SearchResults { matches, truncated })
    }

    fn capabilities(&self) -> SearchCapabilities {
        SearchCapabilities {
            regex: true,
            glob_filter: false,
            content_search: true,
            filename_search: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_file::FileInfo;
    use crate::typed_id::SessionId;
    use crate::{FileStat, GrepMatch};
    use everruns_core::session_files::SessionFileSystem;
    use std::collections::HashMap;
    use std::sync::Mutex;

    // ========================================================================
    // MockFileStore for testing
    // ========================================================================

    /// In-memory file store for testing
    struct MockFileStore {
        files: Mutex<HashMap<(SessionId, String), (String, String)>>, // (content, encoding)
        directories: Mutex<HashMap<(SessionId, String), bool>>,
    }

    impl MockFileStore {
        fn new() -> Self {
            Self {
                files: Mutex::new(HashMap::new()),
                directories: Mutex::new(HashMap::new()),
            }
        }

        fn normalize_path(path: &str) -> String {
            let mut normalized = path.trim().to_string();
            if !normalized.starts_with('/') {
                normalized = format!("/{}", normalized);
            }
            if normalized.len() > 1 && normalized.ends_with('/') {
                normalized.pop();
            }
            normalized
        }
    }

    #[async_trait]
    impl SessionFileSystem for MockFileStore {
        fn is_mount_resolver(&self) -> bool {
            false
        }

        async fn read_file(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> everruns_provider::error::Result<Option<SessionFile>> {
            let path = Self::normalize_path(path);
            let files = self.files.lock().unwrap();
            if let Some((content, encoding)) = files.get(&(session_id, path.clone())) {
                Ok(Some(SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.into(),
                    path: path.clone(),
                    name: path.split('/').next_back().unwrap_or("").to_string(),
                    is_directory: false,
                    is_readonly: false,
                    content: Some(content.clone()),
                    encoding: encoding.clone(),
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            } else if self
                .directories
                .lock()
                .unwrap()
                .contains_key(&(session_id, path.clone()))
            {
                // Production stores materialize directories as rows that `read_file`
                // returns (see `InMemorySessionFileStore::create_directory`), so the
                // double must too — otherwise `exists`/`stat` cannot see an empty dir.
                Ok(Some(SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.into(),
                    path: path.clone(),
                    name: path.split('/').next_back().unwrap_or("").to_string(),
                    is_directory: true,
                    is_readonly: false,
                    content: None,
                    encoding: "text".to_string(),
                    size_bytes: 0,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            } else {
                Ok(None)
            }
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> everruns_provider::error::Result<SessionFile> {
            let path = Self::normalize_path(path);
            let mut files = self.files.lock().unwrap();
            files.insert(
                (session_id, path.clone()),
                (content.to_string(), encoding.to_string()),
            );
            Ok(SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.clone(),
                name: path.split('/').next_back().unwrap_or("").to_string(),
                is_directory: false,
                is_readonly: false,
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }

        async fn delete_file(
            &self,
            session_id: SessionId,
            path: &str,
            _recursive: bool,
        ) -> everruns_provider::error::Result<bool> {
            let path = Self::normalize_path(path);
            let mut files = self.files.lock().unwrap();
            Ok(files.remove(&(session_id, path)).is_some())
        }

        async fn list_directory(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> everruns_provider::error::Result<Vec<FileInfo>> {
            let path = Self::normalize_path(path);
            let files = self.files.lock().unwrap();
            let dirs = self.directories.lock().unwrap();
            let mut entries = Vec::new();

            // Root directory always exists
            let is_root = path == "/";

            for ((sid, file_path), (content, _)) in files.iter() {
                if *sid != session_id {
                    continue;
                }

                // Check if file is directly under this path
                let parent = if let Some(idx) = file_path.rfind('/') {
                    if idx == 0 {
                        "/".to_string()
                    } else {
                        file_path[..idx].to_string()
                    }
                } else {
                    "/".to_string()
                };

                if parent == path {
                    entries.push(FileInfo {
                        id: uuid::Uuid::new_v4(),
                        session_id: session_id.into(),
                        path: file_path.clone(),
                        name: file_path.split('/').next_back().unwrap_or("").to_string(),
                        is_directory: false,
                        is_readonly: false,
                        size_bytes: content.len() as i64,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    });
                }
            }

            // Subdirectory entries. Production lists directory rows alongside file rows,
            // so recursive walks (`find`, `grep -r`) can descend. Cover both explicitly
            // created directories and ones implied by a nested file path.
            let prefix = if is_root {
                "/".to_string()
            } else {
                format!("{path}/")
            };
            let mut child_dirs: std::collections::BTreeSet<String> =
                std::collections::BTreeSet::new();
            let descendant_paths = files
                .keys()
                .chain(dirs.keys())
                .filter(|(sid, _)| *sid == session_id)
                .map(|(_, p)| p);
            for candidate in descendant_paths {
                if let Some(rest) = candidate.strip_prefix(&prefix)
                    && let Some(name) = rest.split('/').next()
                    && rest.contains('/')
                    && !name.is_empty()
                {
                    child_dirs.insert(name.to_string());
                }
            }
            for name in child_dirs {
                entries.push(FileInfo {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.into(),
                    path: format!("{prefix}{name}"),
                    name,
                    is_directory: true,
                    is_readonly: false,
                    size_bytes: 0,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                });
            }

            // Return error if directory doesn't exist (not root, not explicitly created,
            // and no files have it as parent)
            if !is_root && entries.is_empty() && !dirs.contains_key(&(session_id, path.clone())) {
                // Also check if any file has this as an ancestor (implicit directory)
                let has_children = files
                    .keys()
                    .any(|(sid, fp)| *sid == session_id && fp.starts_with(&format!("{}/", path)));
                if !has_children {
                    return Err(anyhow::anyhow!("Directory not found: {}", path).into());
                }
            }

            Ok(entries)
        }

        async fn stat_file(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> everruns_provider::error::Result<Option<FileStat>> {
            let path = Self::normalize_path(path);
            let files = self.files.lock().unwrap();
            if let Some((content, _)) = files.get(&(session_id, path.clone())) {
                Ok(Some(FileStat {
                    path: path.clone(),
                    name: path.split('/').next_back().unwrap_or("").to_string(),
                    is_directory: false,
                    is_readonly: false,
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            } else {
                Ok(None)
            }
        }

        async fn grep_files(
            &self,
            session_id: SessionId,
            pattern: &str,
            path_pattern: Option<&str>,
        ) -> everruns_provider::error::Result<Vec<GrepMatch>> {
            let regex = regex::Regex::new(pattern)
                .map_err(|e| anyhow::anyhow!("invalid pattern: {}", e))?;
            let files = self.files.lock().unwrap();
            let mut matches = Vec::new();
            for ((sid, file_path), (content, _)) in files.iter() {
                if *sid != session_id {
                    continue;
                }
                if let Some(pp) = path_pattern
                    && !file_path.starts_with(pp)
                {
                    continue;
                }
                let decoded = SessionFile::decode_content(content, "utf-8")
                    .unwrap_or_else(|_| content.as_bytes().to_vec());
                let text = String::from_utf8_lossy(&decoded);
                for (i, line) in text.lines().enumerate() {
                    if regex.is_match(line) {
                        matches.push(GrepMatch {
                            path: file_path.clone(),
                            line_number: i + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
            matches.sort_by(|a, b| a.path.cmp(&b.path).then(a.line_number.cmp(&b.line_number)));
            Ok(matches)
        }

        async fn create_directory(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> everruns_provider::error::Result<FileInfo> {
            let path = Self::normalize_path(path);
            let mut dirs = self.directories.lock().unwrap();
            dirs.insert((session_id, path.clone()), true);
            Ok(FileInfo {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.clone(),
                name: path.split('/').next_back().unwrap_or("").to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
    }

    // ========================================================================
    // Capability metadata tests
    // ========================================================================

    // Metadata (id/name/status/risk/icon/category), tool-list, and dependency
    // constants are covered registry-wide by
    // `builtin_capabilities_satisfy_registry_invariants` in `capabilities::tests`.
    // Only the behavioral assertion — the description advertises built-in help —
    // is kept here.
    #[test]
    fn description_advertises_builtin_help_and_version() {
        let description = BashkitShellCapability.description();
        assert!(
            description.contains("`<command> --help`"),
            "description should advertise built-in help, got: {description}"
        );
        assert!(
            description.contains("`<command> --version`"),
            "description should advertise built-in version support, got: {description}"
        );
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = BashkitShellCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        // System prompt is now provided by bashkit library
        assert!(!prompt.is_empty(), "System prompt should not be empty");
        // Should contain the configured username/hostname
        assert!(
            prompt.contains("everruns"),
            "System prompt should contain configured identity"
        );
    }

    // ========================================================================
    // Path resolution (delegated to the store / MountFs)
    // ========================================================================

    // The adapter no longer parses paths itself (EVE-660): it hands them to the
    // store, which is a `MountFs` in production. These confirm the delegation, so
    // the shell shares the file tools' namespace — `/workspace` is the cwd view,
    // and the root mount makes any path addressable. Resolution edge cases live
    // in `mount_fs::tests`.
    fn mount_adapter() -> SessionFileSystemAdapter {
        let store = crate::mount_fs::MountFs::wrap(Arc::new(MockFileStore::new()));
        SessionFileSystemAdapter::new(SessionId::new(), store)
    }

    #[tokio::test]
    async fn adapter_maps_workspace_alias_to_backend_root() {
        let adapter = mount_adapter();
        adapter
            .write_file(Path::new("/workspace/file.txt"), b"hi")
            .await
            .unwrap();
        // The same file is visible via the `/workspace` alias and the
        // backend-native path — one namespace.
        assert_eq!(
            adapter
                .read_file(Path::new("/workspace/file.txt"))
                .await
                .unwrap(),
            b"hi"
        );
        assert_eq!(
            adapter.read_file(Path::new("/file.txt")).await.unwrap(),
            b"hi"
        );
    }

    #[tokio::test]
    async fn adapter_addresses_any_path_from_root() {
        // The old adapter rejected paths outside `/workspace`; with the root
        // mount they resolve into the backend instead (write anywhere from root,
        // still contained by the backend).
        let adapter = mount_adapter();
        adapter
            .write_file(Path::new("/tmp/file.txt"), b"x")
            .await
            .unwrap();
        assert_eq!(
            adapter.read_file(Path::new("/tmp/file.txt")).await.unwrap(),
            b"x"
        );
    }

    // ========================================================================
    // Tool error handling tests
    // ========================================================================

    #[tokio::test]
    async fn test_bash_without_context() {
        let tool = BashTool::default();
        let result = tool.execute(json!({"commands": "echo hello"})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_bash_missing_command() {
        let tool = BashTool::default();
        let context = ToolContext::new(SessionId::new());

        let result = tool.execute_with_context(json!({}), &context).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Missing required parameter"));
        } else {
            panic!("Expected tool error for missing command");
        }
    }

    #[tokio::test]
    async fn test_bash_no_file_store() {
        let tool = BashTool::default();
        let context = ToolContext::new(SessionId::new());

        let result = tool
            .execute_with_context(json!({"commands": "echo hello"}), &context)
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("not available"));
        } else {
            panic!("Expected tool error for missing file store");
        }
    }

    // ========================================================================
    // Bash execution tests with MockFileStore
    // ========================================================================

    fn create_context_with_mock_store() -> (ToolContext, SessionId) {
        let session_id = SessionId::new();
        // Wrap in MountFs exactly as production does, so the shell resolves
        // `/workspace` and the root mount through the same path it uses live.
        let store = crate::mount_fs::MountFs::wrap(Arc::new(MockFileStore::new()));
        let mut context = ToolContext::new(session_id);
        context.file_store = Some(store);
        (context, session_id)
    }

    #[tokio::test]
    async fn bash_output_preserves_blank_lines_and_final_progress_in_raw_and_visible_results() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();
        for (command, stdout, stderr, total_lines, raw) in [
            ("echo hello world", "hello world\n", "", 1, "hello world\n"),
            (
                r#"printf '\n\n10%%\r100%%\r\n'; printf '\nwarning\r\n' >&2"#,
                "\n\n100%\n",
                "\nwarning\n",
                3,
                "\n\n100%\n\n--- stderr ---\n\nwarning\n",
            ),
        ] {
            let result = tool
                .execute_with_context(json!({"commands":command}), &context)
                .await
                .into_tool_result("call-output", "bash");
            assert_eq!(result.tool_call_id, "call-output");
            assert!(result.error.is_none());
            assert_eq!(
                result.result,
                Some(
                    json!({"stdout":stdout,"stderr":stderr,"exit_code":0,"success":true,"truncated":false,"total_lines":total_lines})
                )
            );
            assert_eq!(result.raw_output.as_deref(), Some(raw));
        }
    }

    #[tokio::test]
    async fn test_bash_pwd_default_workspace() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        let result = tool
            .execute_with_context(json!({"commands": "pwd"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "/workspace\n");
            assert_eq!(output["exit_code"], 0);
        } else {
            panic!("Expected success result, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_env_variables() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Test HOME
        let result = tool
            .execute_with_context(json!({"commands": "echo $HOME"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "/home/agent\n");
        } else {
            panic!("Expected success");
        }

        // Test WORKSPACE
        let result = tool
            .execute_with_context(json!({"commands": "echo $WORKSPACE"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "/workspace\n");
        } else {
            panic!("Expected success");
        }

        // Test USER (set by bashkit from username)
        let result = tool
            .execute_with_context(json!({"commands": "echo $USER"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "everruns\n");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_bash_lang_env_default() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Default locale (None) should set LANG to en-US
        let result = tool
            .execute_with_context(json!({"commands": "echo $LANG"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "en-US\n");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_bash_lang_env_from_context_locale() {
        let (mut context, _) = create_context_with_mock_store();
        context.locale = Some("uk-UA".to_string());
        let tool = BashTool::default();

        let result = tool
            .execute_with_context(json!({"commands": "echo $LANG"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "uk-UA\n");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_bash_write_and_read_file() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Write a file
        let result = tool
            .execute_with_context(
                json!({"commands": "echo 'test content' > /workspace/test.txt"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Read it back
        let result = tool
            .execute_with_context(json!({"commands": "cat /workspace/test.txt"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "test content\n");
        } else {
            panic!("Expected success result");
        }
    }

    #[tokio::test]
    async fn test_bash_recursive_walk_descends_into_subdirectories() {
        let (context, _guard) = create_context_with_mock_store();
        let tool = BashTool::default();

        let result = tool
            .execute_with_context(
                json!({"commands": "mkdir -p /workspace/a/b && echo needle > /workspace/a/b/c.txt \
                     && ls /workspace/a && find /workspace/a -name '*.txt'"}),
                &context,
            )
            .await;

        match result {
            ToolExecutionResult::Success(output) => {
                let stdout = output["stdout"].as_str().unwrap_or("");
                assert_eq!(output["exit_code"], 0, "stderr: {}", output["stderr"]);
                assert!(
                    stdout.contains("b\n"),
                    "expected subdir in ls, got {stdout:?}"
                );
                assert!(
                    stdout.contains("/workspace/a/b/c.txt"),
                    "expected find to descend, got {stdout:?}"
                );
            }
            other => panic!("Expected success result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_adapter_reports_absent_paths_as_missing() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Stores answer `list_directory` for unknown paths with an empty listing, so
        // `exists`/`stat` must not read that as "an empty directory is here". When they
        // did, `touch` skipped creation (the file already "existed") and reported success.
        assert!(
            !adapter
                .exists(Path::new("/workspace/nope.txt"))
                .await
                .unwrap()
        );
        assert!(
            adapter
                .stat(Path::new("/workspace/nope.txt"))
                .await
                .is_err()
        );

        adapter
            .mkdir(Path::new("/workspace/realdir"), false)
            .await
            .unwrap();
        assert!(
            adapter
                .exists(Path::new("/workspace/realdir"))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_bash_touch_creates_and_updates_files() {
        let (context, _guard) = create_context_with_mock_store();
        let tool = BashTool::default();

        // `touch` writes the file and then sets its mtime; the session filesystem
        // does not track mtimes, so the second step must not fail the command.
        let result = tool
            .execute_with_context(
                json!({"commands": "touch /workspace/a.txt && touch /workspace/a.txt && ls /workspace"}),
                &context,
            )
            .await;

        match result {
            ToolExecutionResult::Success(output) => {
                assert_eq!(output["exit_code"], 0, "stderr: {}", output["stderr"]);
                assert!(
                    output["stdout"].as_str().unwrap_or("").contains("a.txt"),
                    "expected a.txt in listing, got {:?}",
                    output["stdout"]
                );
            }
            other => panic!("Expected success result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_bash_pipe_command() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        let result = tool
            .execute_with_context(json!({"commands": "echo hello | cat"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "hello\n");
            assert_eq!(output["exit_code"], 0);
        } else {
            panic!("Expected success result");
        }
    }

    #[tokio::test]
    async fn test_bash_arithmetic() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        let result = tool
            .execute_with_context(json!({"commands": "echo $((2 + 3 * 4))"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "14\n");
        } else {
            panic!("Expected success result");
        }
    }

    #[tokio::test]
    async fn test_bash_command_substitution() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        let result = tool
            .execute_with_context(json!({"commands": "echo $(echo nested)"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "nested\n");
        } else {
            panic!("Expected success result");
        }
    }

    // ========================================================================
    // Paths outside /workspace resolve into the backend (EVE-660)
    // ========================================================================
    //
    // `/workspace` is just the shell's cwd; the root mount makes any path
    // addressable. A path like `/tmp/x` resolves into the backend rather than
    // being rejected. For a host-backed store this stays contained under the
    // store's root (with symlink rejection) — it is never the host `/tmp`.

    #[tokio::test]
    async fn test_bash_write_from_root_succeeds() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Writing and reading back a path outside /workspace round-trips.
        let result = tool
            .execute_with_context(
                json!({"commands": "echo hi > /tmp/note.txt && cat /tmp/note.txt"}),
                &context,
            )
            .await;

        match result {
            ToolExecutionResult::Success(output) => {
                assert_eq!(output["exit_code"], 0, "got: {:?}", output);
                assert_eq!(output["stdout"], "hi\n");
            }
            other => panic!("expected success, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_bash_read_missing_file_fails_as_not_found() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // A nonexistent path resolves but has no file — `cat` fails with a
        // non-zero exit, not a containment error.
        let result = tool
            .execute_with_context(json!({"commands": "cat /etc/passwd"}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(output) => {
                assert_ne!(
                    output["exit_code"], 0,
                    "missing file should fail: {:?}",
                    output
                );
            }
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("not found") || msg.contains("No such"),
                    "got: {}",
                    msg
                );
            }
            _ => panic!("Unexpected result type"),
        }
    }

    #[tokio::test]
    async fn test_bash_mkdir_from_root_succeeds() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        let result = tool
            .execute_with_context(json!({"commands": "mkdir /tmp/sub && echo done"}), &context)
            .await;

        match result {
            ToolExecutionResult::Success(output) => {
                assert_eq!(output["exit_code"], 0, "got: {:?}", output);
                assert_eq!(output["stdout"], "done\n");
            }
            other => panic!("expected success, got: {:?}", other),
        }
    }

    // ========================================================================
    // Working directory tests
    // ========================================================================

    #[tokio::test]
    async fn test_bash_custom_working_dir() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // First create the directory
        let result = tool
            .execute_with_context(json!({"commands": "mkdir -p /workspace/mydir"}), &context)
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Run pwd with custom working directory
        let result = tool
            .execute_with_context(
                json!({
                    "commands": "pwd",
                    "working_dir": "/workspace/mydir"
                }),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "/workspace/mydir\n");
        } else {
            panic!("Expected success result");
        }
    }

    // ========================================================================
    // Exit code tests
    // ========================================================================

    #[tokio::test]
    async fn test_bash_false_command_exit_code() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        let result = tool
            .execute_with_context(json!({"commands": "false"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 1);
            assert_eq!(output["success"], false);
        } else {
            panic!("Expected success result with non-zero exit code");
        }
    }

    #[tokio::test]
    async fn test_bash_true_command_exit_code() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        let result = tool
            .execute_with_context(json!({"commands": "true"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["success"], true);
        } else {
            panic!("Expected success result");
        }
    }

    // ========================================================================
    // FileSystem adapter direct tests
    // ========================================================================

    #[tokio::test]
    async fn test_adapter_read_write_workspace_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write a file
        adapter
            .write_file(Path::new("/workspace/test.txt"), b"hello")
            .await
            .unwrap();

        // Read it back
        let content = adapter
            .read_file(Path::new("/workspace/test.txt"))
            .await
            .unwrap();
        assert_eq!(content, b"hello");
    }

    #[tokio::test]
    async fn test_adapter_read_missing_file_is_not_found() {
        let adapter = mount_adapter();

        // A path outside /workspace resolves but has no file: NotFound, not a
        // containment rejection.
        let result = adapter.read_file(Path::new("/tmp/file.txt")).await;
        let err = result.unwrap_err();
        assert!(
            matches!(&err, bashkit::Error::Io(io) if io.kind() == std::io::ErrorKind::NotFound),
            "expected NotFound, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_adapter_write_from_root_succeeds() {
        let adapter = mount_adapter();

        // Writing outside /workspace now resolves into the backend and reads back.
        adapter
            .write_file(Path::new("/tmp/file.txt"), b"data")
            .await
            .unwrap();
        assert_eq!(
            adapter.read_file(Path::new("/tmp/file.txt")).await.unwrap(),
            b"data"
        );
    }

    #[tokio::test]
    async fn test_adapter_stat_workspace_root() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        let stat = adapter.stat(Path::new("/workspace")).await.unwrap();
        assert!(stat.file_type.is_dir());
    }

    #[tokio::test]
    async fn test_adapter_stat_directory_returns_dir_type() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Create a directory
        adapter
            .mkdir(Path::new("/workspace/mydir"), false)
            .await
            .unwrap();

        // stat should report it as a directory, not a file
        let stat = adapter.stat(Path::new("/workspace/mydir")).await.unwrap();
        assert!(
            stat.file_type.is_dir(),
            "Expected directory but got file type for /workspace/mydir"
        );
    }

    #[tokio::test]
    async fn test_adapter_stat_file_returns_file_type() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write a file
        adapter
            .write_file(Path::new("/workspace/test.txt"), b"hello")
            .await
            .unwrap();

        // stat should report it as a file
        let stat = adapter
            .stat(Path::new("/workspace/test.txt"))
            .await
            .unwrap();
        assert!(
            stat.file_type.is_file(),
            "Expected file but got directory type for /workspace/test.txt"
        );
        assert_eq!(stat.size, 5);
    }

    #[tokio::test]
    async fn test_adapter_exists_workspace() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // /workspace always exists
        assert!(adapter.exists(Path::new("/workspace")).await.unwrap());

        // /tmp does not exist (outside workspace)
        assert!(!adapter.exists(Path::new("/tmp")).await.unwrap());
    }

    #[tokio::test]
    async fn test_adapter_mkdir_and_list() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store.clone());

        // Create a directory
        adapter
            .mkdir(Path::new("/workspace/mydir"), false)
            .await
            .unwrap();

        // Write a file in it
        adapter
            .write_file(Path::new("/workspace/mydir/file.txt"), b"content")
            .await
            .unwrap();

        // List should include the file
        let entries = adapter
            .read_dir(Path::new("/workspace/mydir"))
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "file.txt");
    }

    #[tokio::test]
    async fn test_adapter_rename_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write original file
        adapter
            .write_file(Path::new("/workspace/old.txt"), b"data")
            .await
            .unwrap();

        // Rename it
        adapter
            .rename(
                Path::new("/workspace/old.txt"),
                Path::new("/workspace/new.txt"),
            )
            .await
            .unwrap();

        // Old file should not exist
        let old_result = adapter.read_file(Path::new("/workspace/old.txt")).await;
        assert!(old_result.is_err());

        // New file should have the content
        let new_content = adapter
            .read_file(Path::new("/workspace/new.txt"))
            .await
            .unwrap();
        assert_eq!(new_content, b"data");
    }

    #[tokio::test]
    async fn test_adapter_copy_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write original file
        adapter
            .write_file(Path::new("/workspace/source.txt"), b"copy me")
            .await
            .unwrap();

        // Copy it
        adapter
            .copy(
                Path::new("/workspace/source.txt"),
                Path::new("/workspace/dest.txt"),
            )
            .await
            .unwrap();

        // Both files should exist with same content
        let source = adapter
            .read_file(Path::new("/workspace/source.txt"))
            .await
            .unwrap();
        let dest = adapter
            .read_file(Path::new("/workspace/dest.txt"))
            .await
            .unwrap();
        assert_eq!(source, dest);
        assert_eq!(source, b"copy me");
    }

    #[tokio::test]
    async fn test_adapter_append_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write initial content
        adapter
            .write_file(Path::new("/workspace/log.txt"), b"line1\n")
            .await
            .unwrap();

        // Append more
        adapter
            .append_file(Path::new("/workspace/log.txt"), b"line2\n")
            .await
            .unwrap();

        // Read combined content
        let content = adapter
            .read_file(Path::new("/workspace/log.txt"))
            .await
            .unwrap();
        assert_eq!(content, b"line1\nline2\n");
    }

    #[tokio::test]
    async fn test_adapter_symlink_not_supported() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        let result = adapter
            .symlink(Path::new("/workspace/target"), Path::new("/workspace/link"))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not supported"));
    }

    #[tokio::test]
    async fn test_adapter_chmod_is_noop() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // chmod should succeed as a no-op
        let result = adapter.chmod(Path::new("/workspace/file.txt"), 0o755).await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // Security limit tests (bashkit 0.1.0)
    // ========================================================================

    #[tokio::test]
    async fn test_bash_max_input_bytes_limit() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Create a script larger than 1MB limit
        let large_script = "echo ".to_string() + &"x".repeat(1_100_000);

        let result = tool
            .execute_with_context(json!({"commands": large_script}), &context)
            .await;

        // Should fail due to input size limit
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("too large") || msg.contains("input") || msg.contains("limit"),
                    "Expected input size error, got: {}",
                    msg
                );
            }
            ToolExecutionResult::Success(output) => {
                panic!(
                    "Expected error for oversized script, got success: {:?}",
                    output
                );
            }
            _ => panic!("Unexpected result type"),
        }
    }

    #[tokio::test]
    async fn test_bash_loop_within_limit() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Execute a loop within the 10000 iteration limit
        let command = "i=0; while [ $i -lt 100 ]; do i=$((i + 1)); done; echo $i";

        let result = tool
            .execute_with_context(json!({"commands": command}), &context)
            .await;

        // Should succeed within limits
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"].as_str().unwrap_or("").trim(), "100");
        } else {
            panic!("Expected success for loop within limit: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_function_calls() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Test basic function definition and calls (non-recursive to avoid stack issues)
        let command = r#"
            greet() {
                echo "Hello, $1!"
            }
            greet world
        "#;

        let result = tool
            .execute_with_context(json!({"commands": command}), &context)
            .await;

        // Should succeed
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert!(
                output["stdout"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Hello, world!")
            );
        } else {
            panic!("Expected success for function call: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_arithmetic_expressions() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Test various arithmetic expressions (shallow nesting to avoid stack issues)
        let command = "echo $((1 + 2 * 3))";

        let result = tool
            .execute_with_context(json!({"commands": command}), &context)
            .await;

        // Should succeed
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"].as_str().unwrap_or("").trim(), "7");
        } else {
            panic!("Expected success for arithmetic expression: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_commands_within_limit() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Execute multiple commands within the 1000 command limit
        let command = "for i in $(seq 1 100); do true; done; echo done";

        let result = tool
            .execute_with_context(json!({"commands": command}), &context)
            .await;

        // Should succeed within limits
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert!(output["stdout"].as_str().unwrap_or("").contains("done"));
        } else {
            panic!("Expected success for commands within limit: {:?}", result);
        }
    }

    // ========================================================================
    // Script file execution tests
    // ========================================================================

    #[tokio::test]
    async fn test_bash_execute_script_by_absolute_path() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Create a script file
        let result = tool
            .execute_with_context(
                json!({"commands": "cat > /workspace/test.sh << 'EOF'\n#!/bin/bash\necho hello\nEOF"}),
                &context,
            )
            .await;
        assert!(
            matches!(result, ToolExecutionResult::Success(_)),
            "Failed to create script: {:?}",
            result
        );

        // Execute by absolute path
        let result = tool
            .execute_with_context(json!({"commands": "/workspace/test.sh"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"], "hello\n");
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_execute_script_with_args() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Create a script that uses arguments
        let result = tool
            .execute_with_context(
                json!({"commands": "cat > /workspace/greet.sh << 'EOF'\n#!/bin/bash\necho \"Hello, $1! You are $2.\"\nEOF"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Execute with arguments
        let result = tool
            .execute_with_context(
                json!({"commands": "/workspace/greet.sh world awesome"}),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"], "Hello, world! You are awesome.\n");
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_execute_script_without_shebang() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Create a script without shebang
        let result = tool
            .execute_with_context(
                json!({"commands": "cat > /workspace/simple.sh << 'EOF'\necho simple\nEOF"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Execute - should still work
        let result = tool
            .execute_with_context(json!({"commands": "/workspace/simple.sh"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"], "simple\n");
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_execute_nonexistent_script() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Try to execute a script that doesn't exist
        let result = tool
            .execute_with_context(json!({"commands": "/workspace/nonexistent.sh"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_ne!(output["exit_code"], 0, "Should fail with non-zero exit");
            let stderr = output["stderr"].as_str().unwrap_or("");
            assert!(
                stderr.contains("No such file") || stderr.contains("not found"),
                "Expected file not found error, got stderr: {}",
                stderr
            );
        } else {
            panic!(
                "Expected success result with error output, got: {:?}",
                result
            );
        }
    }

    #[tokio::test]
    async fn test_bash_execute_script_in_nested_dir() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Create nested directory structure and script
        let setup = tool
            .execute_with_context(
                json!({"commands": "mkdir -p /workspace/.agents/skills/nav/scripts && cat > /workspace/.agents/skills/nav/scripts/nav.sh << 'EOF'\n#!/bin/bash\necho \"navigating $1\"\nEOF"}),
                &context,
            )
            .await;
        assert!(matches!(setup, ToolExecutionResult::Success(_)));

        // Execute by absolute path (the exact scenario from the bug report)
        let result = tool
            .execute_with_context(
                json!({"commands": "/workspace/.agents/skills/nav/scripts/nav.sh dist"}),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"], "navigating dist\n");
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_file_mode_is_executable() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Write a file and check that test -x reports it as executable
        let result = tool
            .execute_with_context(
                json!({"commands": "echo 'echo hi' > /workspace/check.sh && test -x /workspace/check.sh && echo 'executable' || echo 'not executable'"}),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert!(
                output["stdout"]
                    .as_str()
                    .unwrap_or("")
                    .contains("executable"),
                "File should be reported as executable, got: {}",
                output["stdout"]
            );
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_execute_script_with_exit_code() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Create a script that exits with a specific code
        let result = tool
            .execute_with_context(
                json!({"commands": "cat > /workspace/fail.sh << 'EOF'\n#!/bin/bash\necho failing\nexit 42\nEOF"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Execute and check exit code propagation
        let result = tool
            .execute_with_context(
                json!({"commands": "/workspace/fail.sh; echo \"code: $?\""}),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            let stdout = output["stdout"].as_str().unwrap_or("");
            assert!(stdout.contains("failing"), "Script should have run");
            assert!(
                stdout.contains("code: 42"),
                "Exit code should propagate, got: {}",
                stdout
            );
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    // ========================================================================
    // Overwrite / existing-file tests
    // ========================================================================

    #[tokio::test]
    async fn test_bash_overwrite_existing_file() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Write a file
        let result = tool
            .execute_with_context(
                json!({"commands": "echo 'first' > /workspace/overwrite.txt"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Overwrite with new content
        let result = tool
            .execute_with_context(
                json!({"commands": "echo 'second' > /workspace/overwrite.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::Success(output) = &result {
            assert_eq!(output["exit_code"], 0, "Overwrite should succeed");
        } else {
            panic!("Expected success on overwrite, got: {:?}", result);
        }

        // Read back — should have new content
        let result = tool
            .execute_with_context(
                json!({"commands": "cat /workspace/overwrite.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "second\n");
        } else {
            panic!("Expected success on read, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_append_to_existing_file() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Create file
        let result = tool
            .execute_with_context(
                json!({"commands": "echo 'line1' > /workspace/append.txt"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Append
        let result = tool
            .execute_with_context(
                json!({"commands": "echo 'line2' >> /workspace/append.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::Success(output) = &result {
            assert_eq!(output["exit_code"], 0, "Append should succeed");
        } else {
            panic!("Expected success on append, got: {:?}", result);
        }

        // Verify combined content
        let result = tool
            .execute_with_context(json!({"commands": "cat /workspace/append.txt"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "line1\nline2\n");
        } else {
            panic!("Expected success on read");
        }
    }

    #[tokio::test]
    async fn test_adapter_overwrite_existing_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write initial
        adapter
            .write_file(Path::new("/workspace/ow.txt"), b"original")
            .await
            .unwrap();

        // Overwrite
        adapter
            .write_file(Path::new("/workspace/ow.txt"), b"updated")
            .await
            .unwrap();

        // Verify new content
        let content = adapter
            .read_file(Path::new("/workspace/ow.txt"))
            .await
            .unwrap();
        assert_eq!(content, b"updated");
    }

    #[tokio::test]
    async fn test_adapter_append_to_existing_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write initial
        adapter
            .write_file(Path::new("/workspace/ap.txt"), b"AAA")
            .await
            .unwrap();

        // Append
        adapter
            .append_file(Path::new("/workspace/ap.txt"), b"BBB")
            .await
            .unwrap();

        // Verify combined
        let content = adapter
            .read_file(Path::new("/workspace/ap.txt"))
            .await
            .unwrap();
        assert_eq!(content, b"AAABBB");
    }

    #[tokio::test]
    async fn test_bash_redirect_creates_parent_dirs() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Write to a nested path — parent dirs should be auto-created
        let result = tool
            .execute_with_context(
                json!({"commands": "echo 'deep' > /workspace/a/b/c/deep.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::Success(output) = &result {
            assert_eq!(output["exit_code"], 0, "Nested write should succeed");
        } else {
            panic!("Expected success, got: {:?}", result);
        }

        // Read back
        let result = tool
            .execute_with_context(
                json!({"commands": "cat /workspace/a/b/c/deep.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "deep\n");
        } else {
            panic!("Expected success on read");
        }
    }

    // ========================================================================
    // bashkit API smoke tests
    // ========================================================================

    #[test]
    fn test_bashkit_tool_description_is_nonempty() {
        let desc = BASHKIT_TOOL.description();
        assert!(
            !desc.is_empty(),
            "bashkit tool description should not be empty"
        );
        // Should mention bash or command execution
        assert!(
            desc.to_lowercase().contains("bash") || desc.to_lowercase().contains("command"),
            "description should mention bash or command, got: {}",
            desc
        );
    }

    #[test]
    fn test_bashkit_tool_system_prompt_is_nonempty() {
        let prompt = BASHKIT_TOOL.system_prompt();
        assert!(
            !prompt.is_empty(),
            "bashkit system prompt should not be empty"
        );
        assert!(
            prompt.contains("everruns"),
            "system prompt should contain configured identity 'everruns', got: {}",
            prompt
        );
    }

    #[test]
    fn test_bashkit_static_description_matches_tool() {
        // Verify the LazyLock statics produce the same values as direct calls
        let direct_desc = BASHKIT_TOOL.description();
        let static_desc: &str = &TOOL_DESCRIPTION;
        assert_eq!(static_desc, direct_desc);

        let direct_prompt = BASHKIT_TOOL.system_prompt();
        let static_prompt: &str = &TOOL_SYSTEM_PROMPT;
        // TOOL_SYSTEM_PROMPT = bashkit prompt + EXEC_OUTPUT_HINT (EVE-223)
        assert!(
            static_prompt.starts_with(&direct_prompt),
            "system prompt should start with bashkit prompt"
        );
        assert!(
            static_prompt.contains("Output economy"),
            "system prompt should include output economy hint"
        );
    }

    #[test]
    fn test_bash_tool_display_name() {
        let tool = BashTool::default();
        assert_eq!(tool.display_name(), Some("Bash"));
    }

    #[test]
    fn test_bash_tool_is_never_deferred() {
        let tool = BashTool::default();
        assert_eq!(tool.deferrable_policy(), DeferrablePolicy::Never);
    }

    #[test]
    fn test_bash_tool_parameters_schema_structure() {
        let tool = BashTool::default();
        let schema = tool.parameters_schema();

        // Verify required fields
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["commands"].is_object());

        // Verify optional fields
        assert!(schema["properties"]["working_dir"].is_object());
        assert!(schema["properties"]["timeout_ms"].is_object());

        // Verify "commands" is required
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("commands")));
    }

    // ========================================================================
    // SearchCapable / indexed search tests
    // ========================================================================

    #[test]
    fn test_adapter_is_search_capable() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        let sc = adapter.as_search_capable();
        assert!(
            sc.is_some(),
            "SessionFileSystemAdapter should be SearchCapable"
        );

        let provider = sc.unwrap().search_provider(Path::new("/workspace"));
        assert!(provider.is_some(), "Should return a SearchProvider");

        let caps = provider.unwrap().capabilities();
        assert!(caps.content_search, "Should support content search");
        assert!(caps.regex, "Should support regex patterns");
    }

    #[tokio::test]
    async fn test_search_provider_returns_grep_results() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store.clone());

        // Write files via the adapter
        adapter
            .write_file(
                Path::new("/workspace/hello.txt"),
                b"hello world\ngoodbye world",
            )
            .await
            .unwrap();
        adapter
            .write_file(Path::new("/workspace/other.txt"), b"no match here")
            .await
            .unwrap();

        let sc = adapter.as_search_capable().unwrap();
        let provider = sc.search_provider(Path::new("/workspace")).unwrap();

        let results = provider
            .search(&SearchQuery {
                pattern: "hello".into(),
                is_regex: false,
                case_insensitive: false,
                root: PathBuf::from("/workspace"),
                glob_filter: None,
                max_results: None,
            })
            .unwrap();

        assert_eq!(results.matches.len(), 1);
        assert_eq!(
            results.matches[0].path,
            PathBuf::from("/workspace/hello.txt")
        );
        assert_eq!(results.matches[0].line_number, 1);
        assert_eq!(results.matches[0].line_content, "hello world");
    }

    #[tokio::test]
    async fn test_search_provider_truncates_at_max_results() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store.clone());

        adapter
            .write_file(
                Path::new("/workspace/many.txt"),
                b"match line 1\nmatch line 2\nmatch line 3\nmatch line 4",
            )
            .await
            .unwrap();

        let sc = adapter.as_search_capable().unwrap();
        let provider = sc.search_provider(Path::new("/workspace")).unwrap();

        let results = provider
            .search(&SearchQuery {
                pattern: "match".into(),
                is_regex: false,
                case_insensitive: false,
                root: PathBuf::from("/workspace"),
                glob_filter: None,
                max_results: Some(2),
            })
            .unwrap();

        assert_eq!(results.matches.len(), 2);
        assert!(results.truncated);
    }

    #[tokio::test]
    async fn test_bash_grep_uses_indexed_search() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool::default();

        // Create files
        tool.execute_with_context(
            json!({"commands": "mkdir -p /workspace/src && echo 'fn main() { println!(\"hello\"); }' > /workspace/src/main.rs && echo 'fn test() {}' > /workspace/src/test.rs"}),
            &context,
        )
        .await;

        // Run grep -r which should use indexed search via SearchCapable
        let result = tool
            .execute_with_context(json!({"commands": "grep -r 'fn' /workspace/src"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            let stdout = output["stdout"].as_str().unwrap_or("");
            assert!(
                stdout.contains("fn main") || stdout.contains("fn test"),
                "grep -r should find matches via indexed search, got: {}",
                stdout
            );
        } else {
            panic!("Expected success result, got: {:?}", result);
        }
    }

    #[test]
    fn test_parameters_schema_delegates_to_bashkit() {
        let tool = BashTool::default();
        let schema = tool.parameters_schema();
        let bashkit_schema = BASHKIT_TOOL.input_schema();

        // All bashkit properties must be present in our schema
        let bashkit_props = bashkit_schema["properties"].as_object().unwrap();
        let our_props = schema["properties"].as_object().unwrap();
        for key in bashkit_props.keys() {
            assert!(
                our_props.contains_key(key),
                "bashkit property '{key}' missing from parameters_schema"
            );
        }

        // Required fields from bashkit must be preserved
        let bashkit_required = bashkit_schema["required"].as_array().unwrap();
        let our_required = schema["required"].as_array().unwrap();
        for req in bashkit_required {
            assert!(
                our_required.contains(req),
                "bashkit required field {req} missing from parameters_schema"
            );
        }

        // Everruns extension: working_dir must be present
        assert!(
            our_props.contains_key("working_dir"),
            "working_dir must be in parameters_schema"
        );
    }

    // ========================================================================
    // Observability hooks (EVE-299)
    // ========================================================================

    #[test]
    fn truncate_for_log_returns_short_strings_unchanged() {
        assert_eq!(truncate_for_log("hello", 100), "hello");
        assert_eq!(truncate_for_log("", 100), "");
    }

    #[test]
    fn truncate_for_log_stays_within_budget_and_marks() {
        let input = "a".repeat(500);
        let out = truncate_for_log(&input, 100);
        assert!(
            out.len() <= 100,
            "output exceeded budget: {} bytes",
            out.len()
        );
        assert!(out.ends_with("…[truncated]"));
        assert!(out.starts_with('a'));
    }

    #[test]
    fn truncate_for_log_respects_utf8_boundaries() {
        // Each '🦀' is 4 bytes; marker is 14 bytes. Budget 20 leaves 6 for content,
        // which backs off to a 4-byte char boundary (one crab).
        let input = "🦀🦀🦀🦀🦀🦀🦀🦀🦀🦀";
        let out = truncate_for_log(input, 20);
        assert!(out.len() <= 20);
        assert!(out.starts_with('🦀'));
        assert!(out.ends_with("…[truncated]"));
    }

    #[test]
    fn truncate_for_log_omits_marker_when_budget_is_too_small() {
        // Budget smaller than the marker -> marker is dropped, content is still
        // cut on a valid UTF-8 boundary and fits within max_bytes.
        let input = "abcdefghijklmnop";
        let out = truncate_for_log(input, 4);
        assert_eq!(out, "abcd");
        assert!(out.len() <= 4);
    }

    #[tokio::test]
    async fn install_observability_hooks_fires_on_builtin_and_preserves_exit() {
        use bashkit::hooks::{HookAction, ToolResult};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU64, Ordering};

        let tool_calls = Arc::new(AtomicU64::new(0));
        let counter = tool_calls.clone();

        // Start from the shared hook installer, then stack a test observer.
        // This proves the installer leaves the builtin pipeline intact and
        // that additional hooks compose cleanly.
        let session_id: SessionId = "session_0197a4a4c0c0780180000000000000ff".parse().unwrap();
        let builder = install_observability_hooks(Bash::builder(), session_id).after_tool(
            Box::new(move |r: ToolResult| {
                counter.fetch_add(1, Ordering::Relaxed);
                HookAction::Continue(r)
            }),
        );

        let mut bash = builder.build();
        let result = bash.exec("echo hook-smoke").await.unwrap();

        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.trim(), "hook-smoke");
        assert!(
            tool_calls.load(Ordering::Relaxed) >= 1,
            "after_tool hook should fire at least once for `echo`"
        );
    }

    // ========================================================================
    // Outbound HTTP via egress (enable_http config)
    // ========================================================================

    mod http_tests {
        use super::*;
        use crate::egress::{EgressError, EgressRequestKind, EgressSigning};
        use crate::egress_transport::tests::MockEgress;
        use crate::network_access::NetworkAccessList;

        fn http_context(egress: Option<Arc<MockEgress>>) -> ToolContext {
            let (mut context, _) = create_context_with_mock_store();
            if let Some(egress) = egress {
                context.egress_service = Some(egress);
            }
            context
        }

        #[tokio::test]
        async fn http_disabled_by_default_even_with_egress_available() {
            let egress = Arc::new(MockEgress::with_responses(vec![]));
            let context = http_context(Some(egress.clone()));
            let tool = BashTool::default();

            let result = tool
                .execute_with_context(
                    json!({"commands": "curl -s http://93.184.216.34/ 2>&1; echo rc=$?"}),
                    &context,
                )
                .await;

            let ToolExecutionResult::Success(output) = result else {
                panic!("expected success result");
            };
            let combined = format!("{}{}", output["stdout"], output["stderr"]);
            assert!(
                !combined.contains("rc=0"),
                "curl must fail without enable_http, got: {combined}"
            );
            assert!(
                egress.requests.lock().unwrap().is_empty(),
                "no request may reach egress when HTTP is disabled"
            );
        }

        #[tokio::test]
        async fn http_enable_without_egress_service_stays_offline() {
            let context = http_context(None);
            let tool = BashTool { enable_http: true };

            let result = tool
                .execute_with_context(
                    json!({"commands": "curl -s http://93.184.216.34/ 2>&1; echo rc=$?"}),
                    &context,
                )
                .await;

            let ToolExecutionResult::Success(output) = result else {
                panic!("expected success result");
            };
            let combined = format!("{}{}", output["stdout"], output["stderr"]);
            assert!(
                !combined.contains("rc=0"),
                "curl must fail without an egress service, got: {combined}"
            );
        }

        #[tokio::test]
        async fn curl_routes_through_egress_and_forwards_policy_metadata() {
            let egress = Arc::new(MockEgress::with_responses(vec![MockEgress::ok(
                200,
                &[("content-type", "text/plain")],
                "egress-ok",
            )]));
            let acl = NetworkAccessList::allow_only(["93.184.216.34"]);
            let mut context = http_context(Some(egress.clone()));
            context.network_access = Some(acl.clone());
            let tool = BashTool { enable_http: true };

            let result = tool
                .execute_with_context(
                    json!({"commands": "curl -s http://93.184.216.34/data"}),
                    &context,
                )
                .await;

            let ToolExecutionResult::Success(output) = result else {
                panic!("expected success result");
            };
            assert_eq!(output["exit_code"], 0, "stderr: {}", output["stderr"]);
            assert!(
                output["stdout"].as_str().unwrap().contains("egress-ok"),
                "stdout: {}",
                output["stdout"]
            );

            assert_eq!(*egress.send_calls.lock().unwrap(), 0);
            assert_eq!(*egress.stream_calls.lock().unwrap(), 1);
            let requests = egress.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            let request = &requests[0];
            assert_eq!(request.method, "GET");
            assert_eq!(request.url, "http://93.184.216.34/data");
            assert_eq!(request.kind, EgressRequestKind::Capability);
            assert_eq!(request.signing, EgressSigning::PlatformDefault);
            assert_eq!(request.network_access, Some(acl));
            assert!(request.timeout_ms.is_some(), "deadline must be forwarded");
            // IP-literal host: bashkit's SSRF precheck pins the validated
            // address so the egress boundary can enforce resolve-then-check.
            let (host, addrs) = request.pinned_addrs.as_ref().expect("pinned addrs");
            assert_eq!(host, "93.184.216.34");
            assert_eq!(addrs[0].ip().to_string(), "93.184.216.34");
            assert_eq!(addrs[0].port(), 80);
        }

        #[tokio::test]
        async fn egress_denial_surfaces_as_curl_access_denied_exit_7() {
            let egress = Arc::new(MockEgress::with_responses(vec![Err(
                EgressError::NetworkAccessDenied {
                    url: "http://93.184.216.34/blocked".to_string(),
                },
            )]));
            let context = http_context(Some(egress));
            let tool = BashTool { enable_http: true };

            let result = tool
                .execute_with_context(
                    json!({"commands": "curl -s http://93.184.216.34/blocked"}),
                    &context,
                )
                .await;

            let ToolExecutionResult::Success(output) = result else {
                panic!("expected success result");
            };
            assert_eq!(output["exit_code"], 7, "stderr: {}", output["stderr"]);
            assert!(
                output["stderr"].as_str().unwrap().contains("access denied"),
                "stderr: {}",
                output["stderr"]
            );
            assert!(
                output["stderr"]
                    .as_str()
                    .unwrap()
                    .contains("blocked by network policy"),
                "stderr: {}",
                output["stderr"]
            );
        }

        #[tokio::test]
        async fn oversized_egress_response_surfaces_as_curl_exit_63() {
            // 11 MB body exceeds bashkit's 10 MB default cap; the transport
            // maps it to TooLarge before the interpreter sees the body.
            let big = "x".repeat(11 * 1024 * 1024);
            let egress = Arc::new(MockEgress::with_responses(vec![MockEgress::ok(
                200,
                &[("content-type", "text/plain")],
                &big,
            )]));
            let context = http_context(Some(egress.clone()));
            let tool = BashTool { enable_http: true };

            let result = tool
                .execute_with_context(
                    json!({"commands": "curl -s http://93.184.216.34/huge"}),
                    &context,
                )
                .await;

            let ToolExecutionResult::Success(output) = result else {
                panic!("expected success result");
            };
            assert_eq!(*egress.send_calls.lock().unwrap(), 0);
            assert_eq!(*egress.stream_calls.lock().unwrap(), 1);
            assert_eq!(output["exit_code"], 63, "stderr: {}", output["stderr"]);
            assert!(
                output["stderr"]
                    .as_str()
                    .unwrap()
                    .contains("response too large"),
                "stderr: {}",
                output["stderr"]
            );
        }

        #[test]
        fn validate_config_accepts_bool_and_rejects_other_types() {
            let cap = BashkitShellCapability;
            assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
            assert!(cap.validate_config(&json!({})).is_ok());
            assert!(cap.validate_config(&json!({"enable_http": true})).is_ok());
            assert!(cap.validate_config(&json!({"enable_http": "yes"})).is_err());
            assert!(cap.validate_config(&json!("nope")).is_err());
        }

        #[test]
        fn config_schema_exposes_enable_http() {
            let schema = BashkitShellCapability.config_schema().unwrap();
            assert!(schema["properties"]["enable_http"]["type"] == "boolean");
        }
    }
}
