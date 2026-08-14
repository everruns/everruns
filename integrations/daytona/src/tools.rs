//! Tool implementations for Daytona sandbox operations.
//!
//! Trust boundary (GitHub clone-auth host allowlist): when the user has a
//! GitHub token, `daytona_git_clone` and `daytona_git_credentials` embed it
//! in HTTPS URLs. To prevent the token from leaking to arbitrary hosts that
//! happen to contain the substring `github`, host injection is gated by a
//! strict allowlist.
//!
//! 1. The default allowlist is `["github.com"]`, matching public SaaS.
//! 2. Operators running GitHub Enterprise can extend the allowlist via the
//!    `EVERRUNS_DAYTONA_GITHUB_TRUSTED_HOSTS` env var (comma-separated).
//!    Entries are matched case-insensitively against the URL host (with
//!    optional `:port` stripped). No suffix or pattern matching — to allow
//!    `github.acme.com`, the operator must list it explicitly.
//! 3. `daytona_git_credentials` writes one credential entry per trusted
//!    host so authentication works against every configured server.
//!
//! See `integrations/daytona/SPEC.md` (GitHub clone-auth host allowlist) and
//! `TM-DAYTONA-008` in `knowledge/security/threat-model.md`.

use everruns_core::SessionFile;
use everruns_core::exec_tool_result::ExecToolResultPayload;
use everruns_core::resource_ownership::{
    list_owned_external_resource_ids, ownership_tracking_unavailable_error,
    require_owned_external_resource,
};
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_narration::{
    arg_str, generic_phrase, labeled_phrase, narrate_read_file, narrate_write_file, safe_arg_str,
    truncate, url_display,
};
use everruns_core::tool_output_sanitizer::{
    READ_FILE_DEFAULT_LIMIT, build_binary_read_file_result, build_text_read_file_result,
    parse_read_file_window_args,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_provider::tool_types::ToolHints;

use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashSet;
use tracing::{debug, warn};

use crate::client::DaytonaClient;
use crate::naming::create_sandbox_with_unique_name;
use crate::state::{
    SandboxState, delete_sandbox_state, get_api_key, get_sandbox_state, list_sandbox_states,
    release_sandbox_lease, required_str, save_sandbox_state, touch_sandbox_lease,
};
use crate::{
    AUTO_ARCHIVE_INTERVAL_MINUTES, AUTO_DELETE_INTERVAL_MINUTES, AUTO_STOP_INTERVAL_MINUTES,
    EXEC_TIMEOUT_MS, LEASE_HEARTBEAT_INTERVAL,
};

// Daytona REST API does not support per-sandbox resource overrides for snapshot-based creation.
// Instead, select a pre-built snapshot that matches the desired resource tier.
// See: https://github.com/daytonaio/daytona/issues/2296

/// Map a size tier to the corresponding Daytona snapshot name.
fn snapshot_for_size(size: &str) -> Result<&'static str, String> {
    match size {
        "small" => Ok("daytona-small"),
        "medium" => Ok("daytona-medium"),
        "large" => Ok("daytona-large"),
        _ => Err(
            "Invalid 'size': must be one of: small (1 vCPU, 1 GiB RAM, 3 GiB disk), \
             medium (2 vCPU, 4 GiB RAM, 8 GiB disk), \
             large (4 vCPU, 8 GiB RAM, 10 GiB disk)"
                .to_string(),
        ),
    }
}

/// Return a diagnostic hint for signal-based or notable exit codes.
///
/// Exit codes 128+N indicate the process was killed by signal N.
/// Common cases: 137 = SIGKILL (often OOM), 141 = SIGPIPE (pipe closed early),
/// 139 = SIGSEGV, 143 = SIGTERM. Providing these hints lets agents adapt
/// their retry strategy instead of blindly retrying. See EVE-252.
fn exit_code_hint(exit_code: i32) -> Option<&'static str> {
    match exit_code {
        0 => None,
        137 => Some(
            "Process was killed (SIGKILL). This usually means the process ran out of memory (OOM). Reduce memory usage or request a larger sandbox.",
        ),
        139 => Some("Process crashed with a segmentation fault (SIGSEGV)."),
        141 => Some(
            "Broken pipe (SIGPIPE). A downstream command (e.g. head, sed, tail) closed the pipe before the upstream command finished writing. This is usually harmless — check if the partial output is sufficient.",
        ),
        143 => Some(
            "Process was terminated (SIGTERM). It may have been stopped by the system or another process.",
        ),
        126 => Some("Command found but not executable. Check file permissions."),
        127 => Some("Command not found. Check that the tool is installed and in PATH."),
        _ if exit_code > 128 && exit_code <= 192 => Some("Process was killed by a signal."),
        _ => None,
    }
}

fn build_exec_tool_result(
    sandbox_id: &str,
    cwd: Option<&str>,
    result: &crate::state::ExecResult,
    output_mode: &str,
) -> ToolExecutionResult {
    let payload = ExecToolResultPayload::new(
        &result.stdout,
        &result.stderr,
        result.exit_code,
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

    let mut response = json!({
        "sandbox_id": sandbox_id,
        "stdout": stdout,
        "stderr": stderr,
        "exit_code": exit_code,
        "success": success,
        "truncated": truncated,
        "total_lines": total_lines,
    });
    if let Some(cwd) = cwd {
        response["cwd"] = json!(cwd);
    }
    if let Some(hint) = exit_code_hint(exit_code) {
        response["hint"] = json!(hint);
    }

    ToolExecutionResult::success_with_raw_output(response, raw_output)
}

/// Parse an optional integer resource parameter, validating type and range.
fn parse_resource_param(
    arguments: &Value,
    name: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, String> {
    let Some(val) = arguments.get(name) else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let n = val
        .as_u64()
        .ok_or_else(|| format!("Invalid '{name}': must be a positive integer"))?;
    if n < min || n > max {
        return Err(format!("Invalid '{name}': must be between {min} and {max}"));
    }
    Ok(Some(n))
}

fn create_sandbox_result_payload(
    sandbox_id: &str,
    requested_name: &str,
    canonical_name: &str,
    workspace_path: &str,
    uploaded_count: usize,
) -> Value {
    json!({
        "sandbox_id": sandbox_id,
        "name": canonical_name,
        "requested_name": requested_name,
        "status": "running",
        "workspace_path": workspace_path,
        "files_uploaded": uploaded_count
    })
}

// ============================================================================
// DaytonaCreateSandboxTool
// ============================================================================

pub struct DaytonaCreateSandboxTool;

#[async_trait]
impl Tool for DaytonaCreateSandboxTool {
    fn narrate(
        &self,
        _tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(generic_phrase(
            "Creating Daytona sandbox",
            "Created Daytona sandbox",
            "Failed to create Daytona sandbox",
            None,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "daytona_create_sandbox"
    }

    fn description(&self) -> &str {
        "Create a new Daytona cloud sandbox. Optionally upload files from session storage. \
         Default size: small (1 vCPU, 1 GiB RAM, 3 GiB disk). \
         Auto-stops after 5 minutes of inactivity."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Sandbox name (optional)"
                },
                "size": {
                    "type": "string",
                    "description": "Sandbox size: small (1 vCPU, 1 GiB RAM, 3 GiB disk), medium (2 vCPU, 4 GiB RAM, 8 GiB disk), or large (4 vCPU, 8 GiB RAM, 10 GiB disk). Default: small.",
                    "enum": ["small", "medium", "large"],
                    "default": "small"
                },
                "snapshot": {
                    "type": "string",
                    "description": "Daytona snapshot name (advanced, overrides size). Use size parameter for standard tiers."
                },
                "auto_stop_minutes": {
                    "type": "integer",
                    "description": "Minutes of inactivity before auto-stop (1-60, default: 5). Increase for long builds.",
                    "minimum": 1,
                    "maximum": 60,
                    "default": 5
                },
                "upload_files": {
                    "type": "array",
                    "description": "Files to upload from session storage (optional)",
                    "items": {
                        "type": "object",
                        "properties": {
                            "session_path": { "type": "string", "description": "Source path in session storage" },
                            "sandbox_path": { "type": "string", "description": "Destination path in sandbox" }
                        },
                        "required": ["session_path", "sandbox_path"]
                    }
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_create_sandbox requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };

        let client = DaytonaClient::new(api_key);

        // Build create request
        let requested_name = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Everruns Sandbox");

        // Build ownership labels for audit/cleanup traceability
        let mut labels = serde_json::Map::new();
        labels.insert("everruns".to_string(), json!("true"));
        labels.insert(
            "everruns.session_id".to_string(),
            json!(context.session_id.to_string()),
        );
        if let Some(session_store) = &context.session_store
            && let Ok(Some(session)) = session_store.get_session(context.session_id).await
        {
            labels.insert(
                "everruns.harness_id".to_string(),
                json!(session.harness_id.to_string()),
            );
            labels.insert(
                "everruns.org_id".to_string(),
                json!(&session.organization_id),
            );
            if let Some(agent_id) = &session.agent_id {
                labels.insert("everruns.agent_id".to_string(), json!(agent_id.to_string()));
            }
        }

        let auto_stop = match parse_resource_param(&arguments, "auto_stop_minutes", 1, 60) {
            Ok(v) => v.unwrap_or(AUTO_STOP_INTERVAL_MINUTES),
            Err(e) => return ToolExecutionResult::tool_error(&e),
        };

        let mut create_body = json!({
            "name": requested_name,
            "autoStopInterval": auto_stop,
            "autoArchiveInterval": AUTO_ARCHIVE_INTERVAL_MINUTES,
            "autoDeleteInterval": AUTO_DELETE_INTERVAL_MINUTES,
            "labels": labels,
        });
        // Resolve snapshot: explicit snapshot > size tier > default (small)
        let explicit_snapshot = arguments
            .get("snapshot")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());
        let size = arguments.get("size").and_then(|v| v.as_str());

        let snapshot_name = if let Some(snap) = &explicit_snapshot {
            snap.clone()
        } else {
            let sz = size.unwrap_or("small");
            match snapshot_for_size(sz) {
                Ok(name) => name.to_string(),
                Err(e) => return ToolExecutionResult::tool_error(&e),
            }
        };

        create_body["snapshot"] = json!(snapshot_name);

        // Create sandbox
        let (sandbox_info, sandbox_name) =
            match create_sandbox_with_unique_name(&client, requested_name, create_body).await {
                Ok(result) => result,
                Err(e) => return ToolExecutionResult::tool_error(e),
            };
        let canonical_name = sandbox_name.canonical_name;
        let sandbox_id = sandbox_info.id.clone();
        let requested_name = sandbox_name.requested_name;
        debug!(
            "Created Daytona sandbox from requested name '{}' as canonical name '{}'",
            requested_name, canonical_name
        );

        // Wait for sandbox to reach "started" state
        debug!("Waiting for sandbox to start: {sandbox_id}");
        if let Err(e) = client.wait_for_ready(&sandbox_id).await {
            warn!("Sandbox readiness check failed: {e}");
            // Continue anyway — the sandbox was created, agent can retry later
        }

        let workspace_path = crate::DAYTONA_WORKSPACE_PATH.to_string();

        // Ensure workspace directory exists
        if let Err(e) = client
            .exec(
                &sandbox_id,
                &format!("mkdir -p {}", crate::DAYTONA_WORKSPACE_PATH),
                None,
                None,
                |_| {},
            )
            .await
        {
            warn!("Failed to create workspace directory: {e}");
        }

        // Save state
        let state = SandboxState {
            sandbox_id: sandbox_id.clone(),
            workspace_path: workspace_path.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(e) = save_sandbox_state(context, &state).await {
            return e;
        }
        if let Err(e) = touch_sandbox_lease(context, &state, Some(canonical_name.clone())).await {
            return e;
        }

        // Optionally upload files
        let mut uploaded_count = 0;
        if let Some(files) = arguments.get("upload_files").and_then(|v| v.as_array()) {
            let file_store = match context.file_store.as_ref() {
                Some(fs) => fs,
                None => {
                    return ToolExecutionResult::tool_error(
                        "File store not available for file upload",
                    );
                }
            };

            for file_spec in files {
                let session_path = match file_spec.get("session_path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => continue,
                };
                let sandbox_path = match file_spec.get("sandbox_path").and_then(|v| v.as_str()) {
                    Some(p) => p,
                    None => continue,
                };

                // Read from session storage
                match file_store.read_file(context.session_id, session_path).await {
                    Ok(Some(file)) => {
                        let content = file.content.unwrap_or_default();
                        if let Err(e) = client
                            .file_upload(&sandbox_id, sandbox_path, content.as_bytes())
                            .await
                        {
                            warn!("Failed to upload {session_path} to sandbox: {e}");
                        } else {
                            uploaded_count += 1;
                        }
                    }
                    Ok(None) => {
                        warn!("File not found in session storage: {session_path}");
                    }
                    Err(e) => {
                        warn!("Failed to read {session_path}: {e}");
                    }
                }
            }
        }

        ToolExecutionResult::success(create_sandbox_result_payload(
            &sandbox_id,
            &requested_name,
            &canonical_name,
            &workspace_path,
            uploaded_count,
        ))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaExecTool
// ============================================================================

pub struct DaytonaExecTool;

#[async_trait]
impl Tool for DaytonaExecTool {
    fn narrate(
        &self,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        let fallback = self.display_name().unwrap_or("Daytona");
        Some(everruns_core::tool_narration::narrate_shell_exec(
            &tool_call.arguments,
            fallback,
            phase,
            locale,
        ))
    }

    fn name(&self) -> &str {
        "daytona_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command in a Daytona sandbox. Streams output in real time. \
         Default timeout: 2 minutes. Working directory defaults to /home/daytona. \
         For long builds (cargo, npm), pass a larger timeout explicitly."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID to execute in"
                },
                "command": {
                    "type": "string",
                    "description": "Shell command to execute (e.g., 'python app.py')"
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory (optional, defaults to sandbox workspace)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in milliseconds (optional, default: 300000)"
                },
                "output": everruns_core::tool_output_sanitizer::output_verbosity_schema()
            },
            "required": ["sandbox_id", "command"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
            .with_persist_output(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_exec requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let command = match required_str(&arguments, "command") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let cwd = arguments.get("cwd").and_then(|v| v.as_str());
        let output_mode = arguments
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("auto");
        let timeout = arguments
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(EXEC_TIMEOUT_MS);

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = DaytonaClient::new(api_key);

        debug!("Executing in sandbox {sandbox_id}: {command}");

        // Spawn a heartbeat task that periodically renews the Everruns
        // leased-resource record while the exec call is in-flight. This
        // prevents leased-resource cleanup from reclaiming the sandbox
        // during long-running commands (e.g. Rust compilation).
        // Note: this does NOT ping Daytona — use `auto_stop_minutes` on
        // sandbox creation to extend Daytona's inactivity timer.
        let heartbeat_ctx = context.clone();
        let heartbeat_state = state.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                debug!("Heartbeat: renewing Everruns sandbox lease during exec");
                if let Err(e) = touch_sandbox_lease(&heartbeat_ctx, &heartbeat_state, None).await {
                    warn!("Heartbeat lease renewal failed: {e:?}");
                }
                tokio::time::sleep(LEASE_HEARTBEAT_INTERVAL).await;
            }
        });

        // Use streaming exec to emit real-time output via tool.output.delta events.
        // Send chunks over an mpsc channel to a single emitter task to preserve
        // ordering and avoid unbounded task spawning.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<crate::state::ExecOutputChunk>();
        let streaming_ctx = context.clone();
        let emitter = tokio::spawn(async move {
            while let Some(delta) = rx.recv().await {
                streaming_ctx
                    .emit_tool_output("daytona_exec", &delta.text, delta.stream.as_str())
                    .await;
            }
        });

        let result = client
            .exec(sandbox_id, command, cwd, Some(timeout), |chunk| {
                let _ = tx.send(chunk);
            })
            .await;
        drop(tx); // Close the channel so the emitter task finishes.
        let _ = emitter.await;
        heartbeat.abort();

        match result {
            Ok(result) => {
                if let Err(e) = touch_sandbox_lease(context, &state, None).await {
                    return e;
                }
                build_exec_tool_result(sandbox_id, cwd, &result, output_mode)
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaReadFileTool
// ============================================================================

pub struct DaytonaReadFileTool;

#[async_trait]
impl Tool for DaytonaReadFileTool {
    fn narrate(
        &self,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_read_file(&tool_call.arguments, phase, locale))
    }

    fn name(&self) -> &str {
        "daytona_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from a remote Daytona sandbox filesystem (NOT the session /workspace). Requires sandbox_id."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "path": {
                    "type": "string",
                    "description": "Path to file in sandbox (e.g., '/home/daytona/main.py')"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 0,
                    "description": "Zero-based line offset to start reading from"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "default": READ_FILE_DEFAULT_LIMIT,
                    "description": "Maximum number of lines to return"
                }
            },
            "required": ["sandbox_id", "path"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_read_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let path = match required_str(&arguments, "path") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let (offset, limit) = match parse_read_file_window_args(&arguments) {
            Ok(window) => window,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = DaytonaClient::new(api_key);

        match client.file_download(sandbox_id, path).await {
            Ok(bytes) => {
                if let Err(e) = touch_sandbox_lease(context, &state, None).await {
                    return e;
                }
                let (content, encoding) = SessionFile::encode_content(&bytes);
                let mut result = if encoding == "text" || encoding == "utf-8" {
                    build_text_read_file_result(
                        "daytona_read_file",
                        path,
                        &content,
                        &encoding,
                        offset,
                        limit,
                    )
                } else {
                    build_binary_read_file_result(path, bytes.len(), &encoding)
                };
                result["sandbox_id"] = json!(sandbox_id);
                ToolExecutionResult::success(result)
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaWriteFileTool
// ============================================================================

pub struct DaytonaWriteFileTool;

#[async_trait]
impl Tool for DaytonaWriteFileTool {
    fn narrate(
        &self,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_write_file(&tool_call.arguments, phase, locale))
    }

    fn name(&self) -> &str {
        "daytona_write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file in a Daytona sandbox filesystem."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "path": {
                    "type": "string",
                    "description": "Path for file in sandbox (e.g., '/home/daytona/main.py')"
                },
                "content": {
                    "type": "string",
                    "description": "File content to write"
                }
            },
            "required": ["sandbox_id", "path", "content"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_write_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let path = match required_str(&arguments, "path") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let content = match required_str(&arguments, "content") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = DaytonaClient::new(api_key);

        match client
            .file_upload(sandbox_id, path, content.as_bytes())
            .await
        {
            Ok(()) => {
                if let Err(e) = touch_sandbox_lease(context, &state, None).await {
                    return e;
                }
                ToolExecutionResult::success(json!({
                    "path": path,
                    "success": true
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaDownloadWorkspaceTool
// ============================================================================

pub struct DaytonaDownloadWorkspaceTool;

#[async_trait]
impl Tool for DaytonaDownloadWorkspaceTool {
    fn narrate(
        &self,
        _tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(generic_phrase(
            "Downloading workspace",
            "Downloaded workspace",
            "Failed to download workspace",
            None,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "daytona_download_workspace"
    }

    fn description(&self) -> &str {
        "Download the entire sandbox workspace to session file storage. \
         Default sandbox path: /home/daytona. Default session path: /workspace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "sandbox_path": {
                    "type": "string",
                    "description": "Root path in sandbox to download (default: workspace path)"
                },
                "session_path": {
                    "type": "string",
                    "description": "Destination path in session storage (default: /workspace)"
                }
            },
            "required": ["sandbox_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_download_workspace requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let file_store = match context.file_store.as_ref() {
            Some(fs) => fs,
            None => {
                return ToolExecutionResult::tool_error(
                    "File store not available for workspace download",
                );
            }
        };

        let sandbox_root = arguments
            .get("sandbox_path")
            .and_then(|v| v.as_str())
            .unwrap_or(&state.workspace_path);
        let session_root_raw = arguments
            .get("session_path")
            .and_then(|v| v.as_str())
            .unwrap_or("/workspace");
        // Normalize: strip /workspace prefix to match session filesystem conventions.
        // The session_file_system capability strips /workspace/ from all paths, so
        // files stored here must use the same normalized form to be discoverable.
        let session_root = normalize_session_path(session_root_raw);

        let client = DaytonaClient::new(api_key);

        // Recursively list and download files
        let mut downloaded = 0u64;
        let mut skipped = 0u64;
        let mut errors = Vec::new();

        let mut dirs_to_visit = vec![sandbox_root.to_string()];

        while let Some(dir_path) = dirs_to_visit.pop() {
            let entries = match client.file_list(sandbox_id, &dir_path).await {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("Failed to list {dir_path}: {e}"));
                    continue;
                }
            };

            for entry in entries {
                let name = entry
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let is_dir = entry
                    .get("isDir")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let full_path = if dir_path.ends_with('/') {
                    format!("{dir_path}{name}")
                } else {
                    format!("{dir_path}/{name}")
                };

                if is_dir {
                    dirs_to_visit.push(full_path);
                } else {
                    // Download file
                    match client.file_download(sandbox_id, &full_path).await {
                        Ok(bytes) => {
                            let (content, encoding) = SessionFile::encode_content(&bytes);
                            let relative =
                                full_path.strip_prefix(sandbox_root).unwrap_or(&full_path);
                            let session_dest = format!(
                                "{}{}",
                                session_root.trim_end_matches('/'),
                                if relative.starts_with('/') {
                                    relative.to_string()
                                } else {
                                    format!("/{relative}")
                                }
                            );

                            match file_store
                                .write_file(context.session_id, &session_dest, &content, &encoding)
                                .await
                            {
                                Ok(_) => downloaded += 1,
                                Err(e) => {
                                    errors.push(format!("Failed to write {session_dest}: {e}"));
                                    skipped += 1;
                                }
                            }
                        }
                        Err(e) => {
                            debug!("Skipping {full_path}: {e}");
                            skipped += 1;
                        }
                    }
                }
            }
        }

        let mut result = json!({
            "files_downloaded": downloaded,
            "files_skipped": skipped
        });
        if !errors.is_empty() {
            result["errors"] = json!(errors);
        }

        if let Err(e) = touch_sandbox_lease(context, &state, None).await {
            return e;
        }

        ToolExecutionResult::success(result)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaListSnapshotsTool
// ============================================================================

pub struct DaytonaListSnapshotsTool;

#[async_trait]
impl Tool for DaytonaListSnapshotsTool {
    fn narrate(
        &self,
        _tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(generic_phrase(
            "Listing snapshots",
            "Listed snapshots",
            "Failed to list snapshots",
            None,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "daytona_list_snapshots"
    }

    fn description(&self) -> &str {
        "List available Daytona snapshots that can be used when creating sandboxes. \
         Returns snapshot names, resource specs (CPU, memory, disk), and state. \
         Use the snapshot name in daytona_create_sandbox's `snapshot` parameter."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_list_snapshots requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };

        let client = DaytonaClient::new(api_key);
        let snapshots = match client.list_snapshots().await {
            Ok(s) => s,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let items: Vec<Value> = snapshots
            .iter()
            .filter(|s| s.state == "active")
            .map(|s| {
                let mut obj = json!({
                    "name": s.name,
                    "state": s.state,
                });
                if let Some(cpu) = s.cpu {
                    obj["cpu"] = json!(cpu);
                }
                if let Some(mem) = s.mem {
                    obj["memory_gb"] = json!(mem);
                }
                if let Some(disk) = s.disk {
                    obj["disk_gb"] = json!(disk);
                }
                if let Some(gpu) = s.gpu {
                    obj["gpu"] = json!(gpu);
                }
                if let Some(ref size) = s.size {
                    obj["size"] = json!(size);
                }
                if let Some(general) = s.general {
                    obj["general"] = json!(general);
                }
                obj
            })
            .collect();

        ToolExecutionResult::success(json!({
            "snapshots": items,
            "count": items.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaListSandboxesTool
// ============================================================================

pub struct DaytonaListSandboxesTool;

#[async_trait]
impl Tool for DaytonaListSandboxesTool {
    fn narrate(
        &self,
        _tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(generic_phrase(
            "Listing sandboxes",
            "Listed sandboxes",
            "Failed to list sandboxes",
            None,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "daytona_list_sandboxes"
    }

    fn description(&self) -> &str {
        "List all Daytona sandboxes created in this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_list_sandboxes requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let states = match list_sandbox_states(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let sandboxes: Vec<Value> = states
            .iter()
            .map(|s| {
                json!({
                    "sandbox_id": s.sandbox_id,
                    "started_at": s.started_at,
                    "workspace_path": s.workspace_path
                })
            })
            .collect();

        ToolExecutionResult::success(json!({
            "sandboxes": sandboxes,
            "count": sandboxes.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaManageSandboxTool
// ============================================================================

pub struct DaytonaManageSandboxTool;

#[async_trait]
impl Tool for DaytonaManageSandboxTool {
    fn narrate(
        &self,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(labeled_phrase(
            "Managing sandbox",
            "Managed sandbox",
            "Failed to manage sandbox",
            arg_str(&tool_call.arguments, &["action"]).map(str::to_string),
            phase,
        ))
    }

    fn name(&self) -> &str {
        "daytona_manage_sandbox"
    }

    fn description(&self) -> &str {
        "Manage sandbox lifecycle: stop or delete a Daytona sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID"
                },
                "action": {
                    "type": "string",
                    "enum": ["stop", "delete"],
                    "description": "Action to perform: stop (halt sandbox), delete (permanently remove)"
                }
            },
            "required": ["sandbox_id", "action"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_destructive(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_manage_sandbox requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let action = match required_str(&arguments, "action") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(state) => state,
            Err(e) => return e,
        };

        let client = DaytonaClient::new(api_key);

        let result = match action {
            "delete" => {
                let r = client.delete_sandbox(sandbox_id).await;
                if r.is_ok() {
                    let _ = delete_sandbox_state(context, sandbox_id).await;
                    let _ = release_sandbox_lease(context, sandbox_id).await;
                }
                r
            }
            "stop" => client.stop_sandbox(sandbox_id).await,
            _ => {
                return ToolExecutionResult::tool_error(format!(
                    "Invalid action: '{action}'. Must be one of: stop, delete"
                ));
            }
        };

        match result {
            Ok(()) => {
                if action == "stop"
                    && let Err(e) = touch_sandbox_lease(context, &state, None).await
                {
                    return e;
                }
                ToolExecutionResult::success(json!({
                    "sandbox_id": sandbox_id,
                    "action": action,
                    "success": true
                }))
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaGitCloneTool
// ============================================================================

pub struct DaytonaGitCloneTool;

#[async_trait]
impl Tool for DaytonaGitCloneTool {
    fn narrate(
        &self,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // Schema key is `repo_url`; also accept common aliases. Rendered via
        // url_display so any embedded credentials/userinfo are stripped.
        let target = safe_arg_str(
            &tool_call.arguments,
            &["repo_url", "url", "repo", "repository"],
        )
        .map(url_display)
        .filter(|value| !value.is_empty());
        Some(labeled_phrase(
            "Cloning repository",
            "Cloned repository",
            "Failed to clone repository",
            target,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "daytona_git_clone"
    }

    fn description(&self) -> &str {
        "Clone a git repository into a Daytona sandbox. \
         Default clone path: /home/daytona/<owner>/<repo>. \
         Automatically uses the user's connected GitHub credentials if available. \
         For private repos, the user must have connected their GitHub account in Settings > Connections."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID to clone into"
                },
                "repo_url": {
                    "type": "string",
                    "description": "Repository URL (e.g., 'https://github.com/user/repo' or 'user/repo' shorthand)"
                },
                "branch": {
                    "type": "string",
                    "description": "Branch to clone (optional, defaults to default branch)"
                },
                "path": {
                    "type": "string",
                    "description": "Clone destination path inside sandbox (optional, defaults to /home/daytona/<owner>/<repo>)"
                }
            },
            "required": ["sandbox_id", "repo_url"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_git_clone requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };
        let repo_url_raw = match required_str(&arguments, "repo_url") {
            Ok(s) if !s.trim().is_empty() => s,
            Ok(_) => {
                return ToolExecutionResult::tool_error("repo_url must not be empty".to_string());
            }
            Err(e) => return e,
        };
        let branch = arguments
            .get("branch")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let clone_path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        // Normalize repo URL: "user/repo" → "https://github.com/user/repo.git"
        let repo_url = normalize_repo_url(repo_url_raw);

        // Build default clone path: <workspace>/owner/repo (preserves user/repo structure)
        let default_path = if let Some(owner_repo) = extract_owner_repo(repo_url_raw) {
            format!("{}/{}", state.workspace_path, owner_repo)
        } else {
            let repo_name = repo_url
                .rsplit('/')
                .next()
                .unwrap_or("repo")
                .trim_end_matches(".git");
            format!("{}/{}", state.workspace_path, repo_name)
        };
        let target_path = clone_path.unwrap_or(&default_path);

        // Resolve GitHub token for authenticated cloning
        let github_token = get_github_token(context).await;

        let client = DaytonaClient::new(api_key);

        // Build the clone URL — embed token only for HTTPS clones whose host
        // is on the operator-configured trusted-host allowlist (defaults to
        // `[github.com]`). Never attach GitHub credentials to arbitrary hosts.
        // See `trusted_github_hosts` for the configuration surface.
        let trusted_hosts = trusted_github_hosts();
        let clone_url = if let Some(ref token) = github_token {
            if let Some(rest) = repo_url.strip_prefix("https://")
                && is_trusted_github_https_host(rest, &trusted_hosts)
            {
                // Inject token into HTTPS GitHub URL: https://oauth2:<token>@<host>/...
                format!("https://oauth2:{token}@{rest}")
            } else {
                repo_url.clone()
            }
        } else {
            repo_url.clone()
        };

        // Build git clone command (shell-escape args to handle special characters)
        let esc_url = shell_escape(&clone_url);
        let esc_path = shell_escape(target_path);
        let mut cmd = format!("git clone --depth 1 {esc_url} {esc_path}");
        if let Some(b) = branch {
            let esc_branch = shell_escape(b);
            cmd = format!("git clone --depth 1 --branch {esc_branch} {esc_url} {esc_path}");
        }

        debug!("Cloning repository via exec: {repo_url} → {target_path}");
        match client.exec(sandbox_id, &cmd, None, None, |_| {}).await {
            Ok(r) if r.exit_code != 0 => {
                let hint = if github_token.is_none()
                    && (r.result.contains("Authentication")
                        || r.result.contains("not found")
                        || r.result.contains("could not read Username")
                        || r.result.contains("403"))
                {
                    "\n\nThis may be a private repository. The user can connect their GitHub account in Settings > Connections to enable authenticated cloning."
                } else {
                    ""
                };
                return ToolExecutionResult::tool_error(format!(
                    "Git clone failed (exit {}): {}{hint}",
                    r.exit_code, r.result
                ));
            }
            Err(e) => {
                return ToolExecutionResult::tool_error(format!("Git clone failed: {e}"));
            }
            Ok(_) => {} // success
        }

        // Get the HEAD commit SHA
        let commit_sha = match client
            .exec(
                sandbox_id,
                &format!(
                    "cd {} && git rev-parse --short HEAD",
                    shell_escape(target_path)
                ),
                None,
                None,
                |_| {},
            )
            .await
        {
            Ok(r) if r.exit_code == 0 => r.result.trim().to_string(),
            _ => "unknown".to_string(),
        };

        if let Err(e) = touch_sandbox_lease(context, &state, None).await {
            return e;
        }

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "repo_url": repo_url,
            "path": target_path,
            "branch": branch.unwrap_or("default"),
            "commit": commit_sha,
            "authenticated": github_token.is_some()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DaytonaGitCredentialsTool
// ============================================================================

pub struct DaytonaGitCredentialsTool;

#[async_trait]
impl Tool for DaytonaGitCredentialsTool {
    fn narrate(
        &self,
        _tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // Never render arguments: this tool handles secret credentials.
        Some(generic_phrase(
            "Configuring git credentials",
            "Configured git credentials",
            "Failed to configure git credentials",
            None,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "daytona_git_credentials"
    }

    fn description(&self) -> &str {
        "Configure git credentials in a Daytona sandbox so that all git operations \
         (push, pull, fetch, rebase, etc.) work transparently via daytona_exec. \
         Uses the user's connected GitHub credentials. Call once after creating a \
         sandbox; call again to refresh if the token expires (~1 hour)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {
                    "type": "string",
                    "description": "Sandbox ID to configure git credentials in"
                }
            },
            "required": ["sandbox_id"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_git_credentials requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = match required_str(&arguments, "sandbox_id") {
            Ok(s) => s,
            Err(e) => return e,
        };

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        // Resolve GitHub token (same chain as daytona_git_clone)
        let github_token = match get_github_token(context).await {
            Some(token) => token,
            None => {
                return ToolExecutionResult::tool_error(
                    "No GitHub credentials found. The user must connect their GitHub account \
                     in Settings > Connections, or set a GITHUB_TOKEN session secret.",
                );
            }
        };

        let client = DaytonaClient::new(api_key);

        // THREAT[TM-DAYTONA-001]: Git token persisted on sandbox disk
        // Mitigation: Written to /tmp (cleared on stop), short-lived (~1h), same trust boundary as exec
        // THREAT[TM-DAYTONA-008]: One credential entry per trusted host so
        // operators running GitHub Enterprise authenticate against their
        // configured server. Hosts come from the same allowlist as
        // `daytona_git_clone` to keep the trust boundary consistent.
        let trusted_hosts = trusted_github_hosts();
        let credentials_content = trusted_hosts
            .iter()
            .map(|host| format!("https://oauth2:{github_token}@{host}\n"))
            .collect::<String>();
        if let Err(e) = client
            .file_upload(
                sandbox_id,
                "/tmp/.git-credentials",
                credentials_content.as_bytes(),
            )
            .await
        {
            return ToolExecutionResult::tool_error(format!(
                "Failed to write credentials file: {e}"
            ));
        }

        // Restrict credential file permissions to owner-only read/write.
        let chmod_cmd = "chmod 600 /tmp/.git-credentials";
        match client.exec(sandbox_id, chmod_cmd, None, None, |_| {}).await {
            Ok(r) if r.exit_code == 0 => {}
            Ok(r) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to secure credentials file permissions (exit {}): {}",
                    r.exit_code, r.result
                ));
            }
            Err(e) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to secure credentials file permissions: {e}"
                ));
            }
        }

        // Configure git to use the credential store
        let config_cmd =
            "git config --global credential.helper 'store --file=/tmp/.git-credentials'";
        match client
            .exec(sandbox_id, config_cmd, None, None, |_| {})
            .await
        {
            Ok(r) if r.exit_code == 0 => {}
            Ok(r) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to configure git credential helper (exit {}): {}",
                    r.exit_code, r.result
                ));
            }
            Err(e) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to configure git credential helper: {e}"
                ));
            }
        }

        debug!("Configured git credentials in sandbox {sandbox_id}");
        if let Err(e) = touch_sandbox_lease(context, &state, None).await {
            return e;
        }

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "authenticated": true,
            "provider": "github",
            "hint": "Git credentials configured. All git operations (push, pull, fetch, etc.) via daytona_exec will now authenticate automatically. Token expires in ~1 hour; call this tool again to refresh."
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Session Path Helpers
// ============================================================================

/// Normalize a session filesystem path by stripping the `/workspace` prefix.
/// The session_file_system capability stores paths without /workspace, so
/// daytona tools must match this convention for files to be discoverable.
fn normalize_session_path(path: &str) -> String {
    if path == "/workspace" {
        "/".to_string()
    } else if let Some(stripped) = path.strip_prefix("/workspace/") {
        format!("/{stripped}")
    } else {
        path.to_string()
    }
}

// ============================================================================
// Git Clone Helpers
// ============================================================================

/// Escape a string for safe use inside POSIX single quotes.
/// Replaces `'` with `'\''` (end quote, escaped quote, start quote).
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Normalize repository URL: "user/repo" → "https://github.com/user/repo.git"
fn normalize_repo_url(url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("git@") {
        url.to_string()
    } else if url.contains('/') && !url.contains(' ') {
        // Looks like "user/repo" shorthand
        format!("https://github.com/{url}.git")
    } else {
        url.to_string()
    }
}

/// Returns true when an HTTPS clone URL suffix's host matches a trusted host.
///
/// Expects the string after stripping `https://`, e.g.:
/// - `github.com/owner/repo.git`
/// - `github.com:443/owner/repo.git`
///
/// Matching is case-insensitive, exact (no suffix/wildcard). The optional
/// `:port` is stripped before comparison. `trusted_hosts` is the operator's
/// configured allowlist (see `trusted_github_hosts`).
fn is_trusted_github_https_host(https_url_without_scheme: &str, trusted_hosts: &[String]) -> bool {
    let host = https_url_without_scheme
        .split('/')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    if host.is_empty() {
        return false;
    }
    trusted_hosts
        .iter()
        .any(|trusted| trusted.eq_ignore_ascii_case(host))
}

/// Resolve the operator-configured trusted-host allowlist for GitHub
/// clone-auth injection. Defaults to `["github.com"]` so public SaaS behavior
/// is unchanged. Operators running GitHub Enterprise extend the list via
/// `EVERRUNS_DAYTONA_GITHUB_TRUSTED_HOSTS` (comma-separated). The default is
/// always included so a misconfigured env var cannot silently disable
/// public-GitHub auth.
fn trusted_github_hosts() -> Vec<String> {
    parse_trusted_github_hosts(std::env::var("EVERRUNS_DAYTONA_GITHUB_TRUSTED_HOSTS").ok())
}

/// Pure-function parser for the trusted-host env var. Always seeds with
/// `github.com`, then appends every well-formed extra entry. Malformed
/// entries (containing `/`, `@`, whitespace, or `..`) are dropped with a
/// warning. Duplicates of `github.com` are skipped.
fn parse_trusted_github_hosts(raw: Option<String>) -> Vec<String> {
    let mut hosts = vec!["github.com".to_string()];
    let Some(raw) = raw else { return hosts };
    for entry in raw.split(',') {
        let trimmed = entry.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Reject malformed entries: must be a bare host (no scheme, no path,
        // no `@`/userinfo, no `..`). Defense against operator misconfig
        // injecting a path-traversal host or credential into the env var.
        if trimmed.contains('/')
            || trimmed.contains('@')
            || trimmed.chars().any(char::is_whitespace)
            || trimmed.contains("..")
        {
            // Do NOT log the raw value: a misconfigured env entry could carry
            // a credential (e.g. `https://oauth2:<token>@host/...`) that we
            // must not write to logs. Surface only the rejection reason and
            // the byte length so operators can locate the problem entry.
            warn!(
                "Ignoring malformed entry in EVERRUNS_DAYTONA_GITHUB_TRUSTED_HOSTS \
                 (entry must be a bare hostname; contains one of `/`, `@`, whitespace, or `..`). \
                 Length: {} bytes",
                trimmed.len()
            );
            continue;
        }
        // Strip optional `:port` for normalization; the matcher already
        // tolerates ports but storing without them keeps the list tidy.
        // Use ASCII-only lowercasing — hostnames are ASCII, and this stays
        // consistent with `eq_ignore_ascii_case` used in matching.
        let normalized = trimmed
            .split(':')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if normalized.is_empty() || normalized == "github.com" {
            continue;
        }
        hosts.push(normalized);
    }
    hosts
}

/// Extract "owner/repo" from a git URL. Returns `Some("owner/repo")` for:
/// - `"owner/repo"` shorthand
/// - `"https://github.com/owner/repo"` or `"https://github.com/owner/repo.git"`
/// - `"git@github.com:owner/repo.git"`
///
/// Returns `None` if the URL doesn't match any recognized pattern.
fn extract_owner_repo(url: &str) -> Option<String> {
    // Shorthand: "owner/repo" (no protocol, no spaces, exactly one slash)
    if !url.contains("://") && !url.starts_with("git@") && url.contains('/') && !url.contains(' ') {
        let trimmed = url.trim_end_matches(".git");
        let parts: Vec<&str> = trimmed.splitn(3, '/').collect();
        if parts.len() == 2
            && is_valid_owner_repo_segment(parts[0])
            && is_valid_owner_repo_segment(parts[1])
        {
            return Some(trimmed.to_string());
        }
    }

    // HTTPS: "https://github.com/owner/repo" or "https://github.com/owner/repo.git"
    if let Some(rest) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        // Skip the host (e.g. "github.com/owner/repo.git" → "owner/repo.git")
        if let Some(path) = rest.split_once('/').map(|(_, p)| p) {
            let path = path.trim_end_matches(".git").trim_end_matches('/');
            let parts: Vec<&str> = path.splitn(3, '/').collect();
            if parts.len() == 2
                && is_valid_owner_repo_segment(parts[0])
                && is_valid_owner_repo_segment(parts[1])
            {
                return Some(path.to_string());
            }
        }
    }

    // SSH: "git@github.com:owner/repo.git"
    if let Some(rest) = url.strip_prefix("git@")
        && let Some(path) = rest.split_once(':').map(|(_, p)| p)
    {
        let path = path.trim_end_matches(".git").trim_end_matches('/');
        let parts: Vec<&str> = path.splitn(3, '/').collect();
        if parts.len() == 2
            && is_valid_owner_repo_segment(parts[0])
            && is_valid_owner_repo_segment(parts[1])
        {
            return Some(path.to_string());
        }
    }

    None
}

fn is_valid_owner_repo_segment(segment: &str) -> bool {
    !segment.is_empty() && segment != "." && segment != ".."
}

/// Resolve GitHub token lazily from user connections, with session secret fallback.
async fn get_github_token(context: &ToolContext) -> Option<String> {
    // Try lazy resolution from user connections (preferred: always fresh)
    if let Some(ref resolver) = context.connection_resolver {
        match resolver
            .get_connection_token(context.session_id, "github")
            .await
        {
            Ok(Some(token)) if !token.is_empty() => return Some(token),
            Ok(_) => {}
            Err(e) => debug!("Connection resolver failed: {e}"),
        }
    }

    // Fallback: session secret (for backward compat with pre-injected tokens)
    if let Some(ref storage) = context.storage_store {
        match storage.get_secret(context.session_id, "GITHUB_TOKEN").await {
            Ok(Some(token)) if !token.is_empty() => return Some(token),
            Ok(_) => {}
            Err(e) => debug!("No GITHUB_TOKEN session secret: {e}"),
        }
    }

    None
}

// ============================================================================
// DaytonaApiCallTool (opt-in via capability config: enable_api_calling)
// ============================================================================

/// Session filesystem path where the Daytona OpenAPI spec is mounted.
pub const DAYTONA_OPENAPI_MOUNT_PATH: &str = "/daytona/openapi.yaml";

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScopedSandboxPath<'a> {
    SandboxCollection,
    Sandbox(&'a str),
    Toolbox(&'a str),
}

pub struct DaytonaApiCallTool;

#[async_trait]
impl Tool for DaytonaApiCallTool {
    fn narrate(
        &self,
        tool_call: &everruns_provider::tool_types::ToolCall,
        phase: everruns_core::tool_narration::ToolNarrationPhase,
        _locale: Option<&str>,
        _ctx: everruns_core::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        // Show only the endpoint path; never render the request body.
        let endpoint = safe_arg_str(&tool_call.arguments, &["path", "endpoint"])
            .map(|value| truncate(value, 48));
        Some(labeled_phrase(
            "Calling Daytona API",
            "Called Daytona API",
            "Failed to call Daytona API",
            endpoint,
            phase,
        ))
    }

    fn name(&self) -> &str {
        "daytona_api_call"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Daytona API Call")
    }

    fn description(&self) -> &str {
        "Call any Daytona REST API endpoint directly. \
         Routes automatically: paths starting with /toolbox/{sandbox_id}/... go to the \
         Toolbox API; all other paths go to the Management API. \
         Authentication headers are injected automatically — do NOT specify headers. \
         The full Daytona OpenAPI spec is available at /daytona/openapi.yaml in the \
         session filesystem — read it to discover available endpoints, parameters, \
         and schemas. \
         For sandbox lifecycle (create/exec/files), prefer the dedicated daytona_* tools \
         which handle state tracking and streaming. Use this tool for endpoints not \
         covered by dedicated tools (e.g. labels, ports, git status, search, replace)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "method": {
                    "type": "string",
                    "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                    "description": "HTTP method"
                },
                "path": {
                    "type": "string",
                    "description": "API path. Management API: /sandbox, /sandbox/{id}, etc. Toolbox API: /toolbox/{sandbox_id}/files, /toolbox/{sandbox_id}/process/execute, etc."
                },
                "body": {
                    "description": "JSON request body (optional, for POST/PUT/PATCH). Usually an object, but arrays are also accepted for endpoints that require them."
                }
            },
            "required": ["method", "path"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_requires_secrets(true)
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "daytona_api_call requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };

        let method_str = match required_str(&arguments, "method") {
            Ok(m) => m.to_string(),
            Err(e) => return e,
        };
        let path = match required_str(&arguments, "path") {
            Ok(p) => p.to_string(),
            Err(e) => return e,
        };

        // Normalize path: ensure leading /, strip trailing /, strip query string
        let path = Self::normalize_api_path(&path);

        let sandbox_scope = match Self::enforce_session_scope(context, &method_str, &path).await {
            Ok(scope) => scope,
            Err(e) => return e,
        };

        if method_str == "GET"
            && path == "/sandbox"
            && sandbox_scope.as_ref().is_some_and(HashSet::is_empty)
        {
            return ToolExecutionResult::success(json!([]));
        }

        let method = match method_str.as_str() {
            "GET" => reqwest::Method::GET,
            "POST" => reqwest::Method::POST,
            "PUT" => reqwest::Method::PUT,
            "PATCH" => reqwest::Method::PATCH,
            "DELETE" => reqwest::Method::DELETE,
            _ => {
                return ToolExecutionResult::tool_error(format!(
                    "Unsupported method: {method_str}. Use GET, POST, PUT, PATCH, or DELETE."
                ));
            }
        };

        let mut body = arguments
            .get("body")
            .and_then(|v| if v.is_null() { None } else { Some(v.clone()) });

        // Inject ownership labels into POST /sandbox body for audit/cleanup traceability.
        // Without these labels, sandboxes created via raw API are invisible to
        // Daytona-side orphan cleanup that filters on the "everruns" label.
        let is_sandbox_create = method_str == "POST" && path == "/sandbox";
        if is_sandbox_create {
            body = Some(Self::inject_labels(context, body.unwrap_or_else(|| json!({}))).await);
        }

        let client = DaytonaClient::new(api_key);
        match client.api_call(method, &path, body).await {
            Ok(response) => {
                let response = if let Some(owned_ids) = sandbox_scope.as_ref() {
                    Self::filter_owned_sandboxes(response, owned_ids)
                } else {
                    response
                };
                // Track sandbox resources created/deleted via raw API calls
                self.track_resources(context, &method_str, &path, &response)
                    .await;
                ToolExecutionResult::success(response)
            }
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }
}

impl DaytonaApiCallTool {
    fn normalize_api_path(path: &str) -> String {
        let path = path.split('?').next().unwrap_or(path);
        let segments: Vec<&str> = path
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();
        if segments.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", segments.join("/"))
        }
    }

    fn scoped_sandbox_path(path: &str) -> Option<ScopedSandboxPath<'_>> {
        if path == "/sandbox" {
            return Some(ScopedSandboxPath::SandboxCollection);
        }

        if let Some(rest) = path.strip_prefix("/sandbox/")
            && let Some(sandbox_id) = rest.split('/').next()
            && !sandbox_id.is_empty()
        {
            return Some(ScopedSandboxPath::Sandbox(sandbox_id));
        }

        if let Some(rest) = path.strip_prefix("/toolbox/")
            && let Some(sandbox_id) = rest.split('/').next()
            && !sandbox_id.is_empty()
        {
            return Some(ScopedSandboxPath::Toolbox(sandbox_id));
        }

        None
    }

    async fn enforce_session_scope(
        context: &ToolContext,
        method: &str,
        path: &str,
    ) -> Result<Option<HashSet<String>>, ToolExecutionResult> {
        match Self::scoped_sandbox_path(path) {
            Some(ScopedSandboxPath::SandboxCollection) if method == "GET" => {
                let Some(owned_ids) =
                    list_owned_external_resource_ids(context, "daytona", "sandbox").await?
                else {
                    return Err(ownership_tracking_unavailable_error("daytona", "sandbox"));
                };
                Ok(Some(owned_ids))
            }
            Some(ScopedSandboxPath::Sandbox(sandbox_id))
            | Some(ScopedSandboxPath::Toolbox(sandbox_id)) => {
                require_owned_external_resource(context, "daytona", "sandbox", sandbox_id).await?;
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn filter_owned_sandboxes(response: Value, owned_ids: &HashSet<String>) -> Value {
        match response {
            Value::Array(items) => Value::Array(
                items
                    .into_iter()
                    .filter(|item| {
                        item.get("id")
                            .and_then(|value| value.as_str())
                            .is_some_and(|id| owned_ids.contains(id))
                    })
                    .collect(),
            ),
            other => other,
        }
    }

    /// Build ownership labels for Daytona sandbox audit/cleanup.
    ///
    /// Mirrors the labels added by `DaytonaCreateSandboxTool` so that sandboxes
    /// created via raw API are still discoverable by label-based cleanup queries.
    async fn build_labels(context: &ToolContext) -> serde_json::Map<String, Value> {
        let mut labels = serde_json::Map::new();
        labels.insert("everruns".to_string(), json!("true"));
        labels.insert(
            "everruns.session_id".to_string(),
            json!(context.session_id.to_string()),
        );
        if let Some(session_store) = &context.session_store
            && let Ok(Some(session)) = session_store.get_session(context.session_id).await
        {
            labels.insert(
                "everruns.harness_id".to_string(),
                json!(session.harness_id.to_string()),
            );
            labels.insert(
                "everruns.org_id".to_string(),
                json!(&session.organization_id),
            );
            if let Some(agent_id) = &session.agent_id {
                labels.insert("everruns.agent_id".to_string(), json!(agent_id.to_string()));
            }
        }
        labels
    }

    /// Inject everruns ownership labels into a POST /sandbox request body.
    ///
    /// Merges with any user-provided labels (user labels take precedence for
    /// non-everruns keys; everruns.* keys are always set to prevent tampering).
    async fn inject_labels(context: &ToolContext, body: Value) -> Value {
        let labels = Self::build_labels(context).await;

        let mut obj = match body {
            Value::Object(map) => map,
            _ => serde_json::Map::new(),
        };

        let mut existing_labels = match obj.remove("labels") {
            Some(Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        };

        for (k, v) in labels {
            existing_labels.insert(k, v);
        }

        obj.insert("labels".to_string(), Value::Object(existing_labels));
        Value::Object(obj)
    }

    /// Best-effort resource tracking for sandbox lifecycle operations via raw API.
    ///
    /// When the agent creates or deletes sandboxes through `daytona_api_call` instead
    /// of the dedicated tools, we still want leased-resource tracking to work.
    /// Labels are injected pre-request via `inject_labels()`; Daytona's PUT labels
    /// API does not reliably update labels post-creation, so we rely solely on
    /// injecting them into the POST /sandbox body.
    async fn track_resources(
        &self,
        context: &ToolContext,
        method: &str,
        path: &str,
        response: &Value,
    ) {
        // POST /sandbox → new sandbox created, register lease
        if method == "POST"
            && path == "/sandbox"
            && let Some(sandbox_id) = response.get("id").and_then(|v| v.as_str())
        {
            let display_name = response
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let state = SandboxState {
                sandbox_id: sandbox_id.to_string(),
                workspace_path: crate::DAYTONA_WORKSPACE_PATH.to_string(),
                started_at: chrono::Utc::now().to_rfc3339(),
            };
            if let Err(e) = save_sandbox_state(context, &state).await {
                warn!("Failed to save sandbox state from api_call: {e:?}");
            }
            if let Err(e) = touch_sandbox_lease(context, &state, display_name).await {
                warn!("Failed to register sandbox lease from api_call: {e:?}");
            }
            debug!(sandbox_id, "Tracked sandbox created via daytona_api_call");
        }

        // DELETE /sandbox/{id} → sandbox deleted, release lease
        if method == "DELETE"
            && let Some(sandbox_id) = path
                .strip_prefix("/sandbox/")
                .filter(|rest| !rest.contains('/'))
        {
            if let Err(e) = delete_sandbox_state(context, sandbox_id).await {
                warn!("Failed to delete sandbox state from api_call: {e:?}");
            }
            if let Err(e) = release_sandbox_lease(context, sandbox_id).await {
                warn!("Failed to release sandbox lease from api_call: {e:?}");
            }
            debug!(sandbox_id, "Tracked sandbox deletion via daytona_api_call");
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ========================================================================
    // Mock SessionStorageStore for integration tests
    // ========================================================================

    use everruns_core::{
        connection_services::UserConnectionResolver, session_services::KeyInfo,
        session_services::SecretInfo, session_services::SessionStorageStore,
    };
    use everruns_provider::error::Result;
    use everruns_provider::typed_id::SessionId;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// In-memory mock of SessionStorageStore for testing.
    struct MockStorageStore {
        secrets: Mutex<HashMap<String, String>>,
    }

    impl MockStorageStore {
        fn new() -> Self {
            Self {
                secrets: Mutex::new(HashMap::new()),
            }
        }

        /// Pre-seed a secret for a given session.
        async fn seed_secret(&self, session_id: SessionId, name: &str, value: &str) {
            let key = format!("{}:{}", session_id, name);
            self.secrets.lock().await.insert(key, value.to_string());
        }
    }

    #[async_trait]
    impl SessionStorageStore for MockStorageStore {
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
            let key = format!("{session_id}:{name}");
            self.secrets.lock().await.insert(key, value.to_string());
            Ok(())
        }
        async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
            let key = format!("{session_id}:{name}");
            Ok(self.secrets.lock().await.get(&key).cloned())
        }
        async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
            let key = format!("{session_id}:{name}");
            Ok(self.secrets.lock().await.remove(&key).is_some())
        }
        async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
            let prefix = format!("{session_id}:");
            let secrets = self.secrets.lock().await;
            Ok(secrets
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .map(|k| SecretInfo {
                    name: k.strip_prefix(&prefix).unwrap_or(k).to_string(),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
                .collect())
        }
    }

    /// Mock connection resolver that returns a fixed token for all providers.
    struct MockConnectionResolver {
        token: Option<String>,
    }

    #[async_trait]
    impl UserConnectionResolver for MockConnectionResolver {
        async fn get_connection_token(
            &self,
            _session_id: SessionId,
            _provider: &str,
        ) -> Result<Option<String>> {
            Ok(self.token.clone())
        }
    }

    /// Mock connection resolver that returns tokens only for specific providers.
    struct ProviderAwareResolver {
        tokens: std::collections::HashMap<String, String>,
    }

    impl ProviderAwareResolver {
        fn daytona_only(api_key: &str) -> Self {
            let mut tokens = std::collections::HashMap::new();
            tokens.insert("daytona".to_string(), api_key.to_string());
            Self { tokens }
        }
    }

    #[async_trait]
    impl UserConnectionResolver for ProviderAwareResolver {
        async fn get_connection_token(
            &self,
            _session_id: SessionId,
            provider: &str,
        ) -> Result<Option<String>> {
            Ok(self.tokens.get(provider).cloned())
        }
    }

    #[tokio::test]
    async fn test_create_sandbox_without_context() {
        let tool = DaytonaCreateSandboxTool;
        let result = tool.execute(json!({})).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn test_exec_without_context() {
        let tool = DaytonaExecTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "command": "ls"}))
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn test_read_file_without_context() {
        let tool = DaytonaReadFileTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "path": "/test.txt"}))
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn test_write_file_without_context() {
        let tool = DaytonaWriteFileTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "path": "/test.txt", "content": "hello"}))
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn test_download_workspace_without_context() {
        let tool = DaytonaDownloadWorkspaceTool;
        let result = tool.execute(json!({"sandbox_id": "test"})).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn test_list_snapshots_without_context() {
        let tool = DaytonaListSnapshotsTool;
        let result = tool.execute(json!({})).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn test_list_sandboxes_without_context() {
        let tool = DaytonaListSandboxesTool;
        let result = tool.execute(json!({})).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn test_manage_sandbox_without_context() {
        let tool = DaytonaManageSandboxTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "action": "stop"}))
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[test]
    fn test_create_sandbox_result_payload_includes_requested_and_canonical_name() {
        let payload = create_sandbox_result_payload(
            "sb_123",
            "Requested Sandbox",
            "Requested Sandbox-abc123",
            "/home/daytona",
            2,
        );

        assert_eq!(payload["sandbox_id"], "sb_123");
        assert_eq!(payload["requested_name"], "Requested Sandbox");
        assert_eq!(payload["name"], "Requested Sandbox-abc123");
        assert_eq!(payload["workspace_path"], "/home/daytona");
        assert_eq!(payload["files_uploaded"], 2);
    }

    #[test]
    fn test_exec_schema_has_required_fields() {
        let tool = DaytonaExecTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"command"));
    }

    #[test]
    fn test_create_sandbox_schema_no_required() {
        let tool = DaytonaCreateSandboxTool;
        let schema = tool.parameters_schema();
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn test_manage_sandbox_schema_has_enum() {
        let tool = DaytonaManageSandboxTool;
        let schema = tool.parameters_schema();
        let action_enum = &schema["properties"]["action"]["enum"];
        let values: Vec<&str> = action_enum
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(values.contains(&"stop"));
        assert!(values.contains(&"delete"));
    }

    #[test]
    fn test_read_file_schema() {
        let tool = DaytonaReadFileTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"path"));
        assert!(schema["properties"]["sandbox_id"].is_object());
        assert!(schema["properties"]["path"].is_object());
    }

    #[test]
    fn test_write_file_schema() {
        let tool = DaytonaWriteFileTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"path"));
        assert!(required_strs.contains(&"content"));
    }

    #[test]
    fn test_download_workspace_schema() {
        let tool = DaytonaDownloadWorkspaceTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        // sandbox_path and session_path are optional
        assert!(!required_strs.contains(&"sandbox_path"));
        assert!(!required_strs.contains(&"session_path"));
    }

    #[test]
    fn test_list_snapshots_schema_no_required() {
        let tool = DaytonaListSnapshotsTool;
        let schema = tool.parameters_schema();
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn test_list_sandboxes_schema_no_required() {
        let tool = DaytonaListSandboxesTool;
        let schema = tool.parameters_schema();
        assert!(schema.get("required").is_none());
    }

    #[test]
    fn test_manage_sandbox_schema_required() {
        let tool = DaytonaManageSandboxTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"action"));
    }

    #[test]
    fn test_all_tool_names_have_daytona_prefix() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(DaytonaCreateSandboxTool),
            Box::new(DaytonaExecTool),
            Box::new(DaytonaReadFileTool),
            Box::new(DaytonaWriteFileTool),
            Box::new(DaytonaDownloadWorkspaceTool),
            Box::new(DaytonaListSnapshotsTool),
            Box::new(DaytonaListSandboxesTool),
            Box::new(DaytonaManageSandboxTool),
            Box::new(DaytonaGitCloneTool),
            Box::new(DaytonaGitCredentialsTool),
        ];
        for tool in &tools {
            assert!(
                tool.name().starts_with("daytona_"),
                "Tool {} should start with 'daytona_'",
                tool.name()
            );
        }
    }

    #[test]
    fn test_encode_content_text_files() {
        let text_bytes = b"hello world\nline two\n";
        let (content, encoding) = SessionFile::encode_content(text_bytes);
        assert_eq!(encoding, "text");
        assert_eq!(content, "hello world\nline two\n");
    }

    #[test]
    fn test_encode_content_binary_files() {
        // PNG-like header with null bytes
        let binary_bytes: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00];
        let (content, encoding) = SessionFile::encode_content(binary_bytes);
        assert_eq!(encoding, "base64");
        // Verify round-trip
        let decoded = SessionFile::decode_content(&content, &encoding).unwrap();
        assert_eq!(decoded, binary_bytes);
    }

    #[test]
    fn test_all_tools_have_descriptions() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(DaytonaCreateSandboxTool),
            Box::new(DaytonaExecTool),
            Box::new(DaytonaReadFileTool),
            Box::new(DaytonaWriteFileTool),
            Box::new(DaytonaDownloadWorkspaceTool),
            Box::new(DaytonaListSnapshotsTool),
            Box::new(DaytonaListSandboxesTool),
            Box::new(DaytonaManageSandboxTool),
            Box::new(DaytonaGitCloneTool),
            Box::new(DaytonaGitCredentialsTool),
        ];
        for tool in &tools {
            assert!(
                !tool.description().is_empty(),
                "Tool {} should have a description",
                tool.name()
            );
        }
    }

    #[test]
    fn test_all_schemas_disallow_additional_properties() {
        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(DaytonaCreateSandboxTool),
            Box::new(DaytonaExecTool),
            Box::new(DaytonaReadFileTool),
            Box::new(DaytonaWriteFileTool),
            Box::new(DaytonaDownloadWorkspaceTool),
            Box::new(DaytonaListSnapshotsTool),
            Box::new(DaytonaListSandboxesTool),
            Box::new(DaytonaManageSandboxTool),
            Box::new(DaytonaGitCloneTool),
            Box::new(DaytonaGitCredentialsTool),
        ];
        for tool in &tools {
            let schema = tool.parameters_schema();
            assert_eq!(
                schema["additionalProperties"],
                json!(false),
                "Tool {} schema should disallow additional properties",
                tool.name()
            );
        }
    }

    // --- Git credentials tool tests ---

    #[tokio::test]
    async fn test_git_credentials_without_context() {
        let tool = DaytonaGitCredentialsTool;
        let result = tool.execute(json!({"sandbox_id": "test"})).await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[test]
    fn test_git_credentials_schema() {
        let tool = DaytonaGitCredentialsTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert_eq!(required_strs.len(), 1);
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    // --- Git clone tool tests ---

    #[tokio::test]
    async fn test_git_clone_without_context() {
        let tool = DaytonaGitCloneTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "repo_url": "user/repo"}))
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[test]
    fn test_git_clone_schema() {
        let tool = DaytonaGitCloneTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"repo_url"));
        // branch and path are optional
        assert!(!required_strs.contains(&"branch"));
        assert!(!required_strs.contains(&"path"));
        assert!(schema["properties"]["branch"].is_object());
        assert!(schema["properties"]["path"].is_object());
    }

    // --- Session path normalization tests ---

    #[test]
    fn test_normalize_session_path_workspace() {
        assert_eq!(normalize_session_path("/workspace"), "/");
    }

    #[test]
    fn test_normalize_session_path_workspace_subpath() {
        assert_eq!(
            normalize_session_path("/workspace/reports/chat"),
            "/reports/chat"
        );
    }

    #[test]
    fn test_normalize_session_path_no_prefix() {
        assert_eq!(normalize_session_path("/reports/chat"), "/reports/chat");
    }

    #[test]
    fn test_normalize_session_path_root() {
        assert_eq!(normalize_session_path("/"), "/");
    }

    // --- Git URL normalization tests ---

    #[test]
    fn test_normalize_repo_url_shorthand() {
        assert_eq!(
            normalize_repo_url("user/repo"),
            "https://github.com/user/repo.git"
        );
    }

    #[test]
    fn test_normalize_repo_url_https() {
        let url = "https://github.com/user/repo.git";
        assert_eq!(normalize_repo_url(url), url);
    }

    #[test]
    fn test_normalize_repo_url_ssh() {
        let url = "git@github.com:user/repo.git";
        assert_eq!(normalize_repo_url(url), url);
    }

    #[test]
    fn test_normalize_repo_url_http() {
        let url = "http://github.com/user/repo";
        assert_eq!(normalize_repo_url(url), url);
    }

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn test_shell_escape_with_single_quote() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_shell_escape_with_spaces() {
        assert_eq!(shell_escape("path with spaces"), "'path with spaces'");
    }

    #[test]
    fn test_normalize_repo_url_https_without_git_suffix() {
        // HTTPS URLs without .git are valid and returned as-is
        let url = "https://github.com/user/repo";
        assert_eq!(normalize_repo_url(url), url);
    }

    #[test]
    fn test_normalize_repo_url_bare_word() {
        // Single word without slash — returned as-is
        assert_eq!(normalize_repo_url("something"), "something");
    }

    #[test]
    fn test_is_trusted_github_https_host_accepts_github_com_by_default() {
        let hosts = vec!["github.com".to_string()];
        assert!(is_trusted_github_https_host(
            "github.com/user/repo.git",
            &hosts
        ));
        assert!(is_trusted_github_https_host(
            "github.com:443/user/repo.git",
            &hosts
        ));
        assert!(is_trusted_github_https_host(
            "GITHUB.COM/user/repo.git",
            &hosts
        ));
    }

    #[test]
    fn test_is_trusted_github_https_host_rejects_non_github_hosts() {
        let hosts = vec!["github.com".to_string()];
        assert!(!is_trusted_github_https_host(
            "evil.example.com/user/repo.git",
            &hosts
        ));
        assert!(!is_trusted_github_https_host(
            "github.com.evil.example/user/repo.git",
            &hosts
        ));
        // Lookalike that prior implementation would reject — verify this
        // remains rejected when only `github.com` is trusted.
        assert!(!is_trusted_github_https_host(
            "github.example.com/user/repo.git",
            &hosts
        ));
    }

    #[test]
    fn test_is_trusted_github_https_host_accepts_enterprise_hosts() {
        // Operator-configured allowlist with public + enterprise hosts.
        let hosts = vec![
            "github.com".to_string(),
            "github.acme.com".to_string(),
            "git.internal.corp".to_string(),
        ];
        assert!(is_trusted_github_https_host(
            "github.com/owner/repo.git",
            &hosts
        ));
        assert!(is_trusted_github_https_host(
            "github.acme.com/owner/repo.git",
            &hosts
        ));
        assert!(is_trusted_github_https_host(
            "git.internal.corp/owner/repo.git",
            &hosts
        ));
        // Case-insensitive match.
        assert!(is_trusted_github_https_host(
            "GITHUB.ACME.COM/owner/repo.git",
            &hosts
        ));
    }

    #[test]
    fn test_is_trusted_github_https_host_rejects_lookalikes_with_enterprise_allowlist() {
        // Even with multiple trusted hosts configured, lookalikes are rejected.
        let hosts = vec!["github.com".to_string(), "github.acme.com".to_string()];
        // Subdomain prefix attack — `evil-github.acme.com` is not exactly
        // `github.acme.com`.
        assert!(!is_trusted_github_https_host(
            "evil-github.acme.com/owner/repo.git",
            &hosts
        ));
        // Suffix-confusion attack — `github.acme.com.evil.example` is rejected.
        assert!(!is_trusted_github_https_host(
            "github.acme.com.evil.example/owner/repo.git",
            &hosts
        ));
        // Empty allowlist rejects everything.
        assert!(!is_trusted_github_https_host(
            "github.com/owner/repo.git",
            &[]
        ));
    }

    #[test]
    fn test_is_trusted_github_https_host_rejects_empty_url() {
        let hosts = vec!["github.com".to_string()];
        assert!(!is_trusted_github_https_host("", &hosts));
        assert!(!is_trusted_github_https_host("/", &hosts));
    }

    #[test]
    fn test_parse_trusted_github_hosts_default() {
        // No env var: only github.com.
        assert_eq!(parse_trusted_github_hosts(None), vec!["github.com"]);
        assert_eq!(
            parse_trusted_github_hosts(Some(String::new())),
            vec!["github.com"]
        );
        assert_eq!(
            parse_trusted_github_hosts(Some(",,".to_string())),
            vec!["github.com"]
        );
    }

    #[test]
    fn test_parse_trusted_github_hosts_appends_enterprise_hosts() {
        let hosts =
            parse_trusted_github_hosts(Some("github.acme.com, git.internal.corp ".to_string()));
        assert_eq!(
            hosts,
            vec![
                "github.com".to_string(),
                "github.acme.com".to_string(),
                "git.internal.corp".to_string(),
            ]
        );
    }

    #[test]
    fn test_parse_trusted_github_hosts_normalizes_case_and_port() {
        let hosts =
            parse_trusted_github_hosts(Some("GITHUB.ACME.COM:8443, Git.Internal.Corp".to_string()));
        assert_eq!(
            hosts,
            vec![
                "github.com".to_string(),
                "github.acme.com".to_string(),
                "git.internal.corp".to_string(),
            ]
        );
    }

    #[test]
    fn test_parse_trusted_github_hosts_rejects_malformed_entries() {
        // Path, scheme, userinfo, traversal, whitespace are all dropped; the
        // default `github.com` floor remains. Operator misconfig cannot
        // silently inject a credential or a redirect target.
        let hosts = parse_trusted_github_hosts(Some(
            "https://evil.example, user@github.com, ../traversal, host with space, github.acme.com"
                .to_string(),
        ));
        assert_eq!(
            hosts,
            vec!["github.com".to_string(), "github.acme.com".to_string(),]
        );
    }

    #[test]
    fn test_parse_trusted_github_hosts_dedupes_default() {
        // Operator listing github.com explicitly should not double-add it.
        let hosts =
            parse_trusted_github_hosts(Some("github.com, GitHub.com, github.acme.com".to_string()));
        assert_eq!(
            hosts,
            vec!["github.com".to_string(), "github.acme.com".to_string(),]
        );
    }

    // --- extract_owner_repo tests ---

    #[test]
    fn test_extract_owner_repo_shorthand() {
        assert_eq!(
            extract_owner_repo("user/repo"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_https() {
        assert_eq!(
            extract_owner_repo("https://github.com/user/repo"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_https_git_suffix() {
        assert_eq!(
            extract_owner_repo("https://github.com/user/repo.git"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_ssh() {
        assert_eq!(
            extract_owner_repo("git@github.com:user/repo.git"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_daytona_api_call_scoped_sandbox_path_collection() {
        assert_eq!(
            DaytonaApiCallTool::scoped_sandbox_path("/sandbox"),
            Some(ScopedSandboxPath::SandboxCollection)
        );
    }

    #[test]
    fn test_daytona_api_call_normalizes_paths_for_scope() {
        assert_eq!(
            DaytonaApiCallTool::normalize_api_path("//sandbox//sb_test//files/?raw=1"),
            "/sandbox/sb_test/files"
        );
    }

    #[test]
    fn test_daytona_api_call_scoped_sandbox_path_management() {
        assert_eq!(
            DaytonaApiCallTool::scoped_sandbox_path("/sandbox/sb_test/files"),
            Some(ScopedSandboxPath::Sandbox("sb_test"))
        );
    }

    #[test]
    fn test_daytona_api_call_scoped_sandbox_path_toolbox() {
        assert_eq!(
            DaytonaApiCallTool::scoped_sandbox_path("/toolbox/sb_test/process/execute"),
            Some(ScopedSandboxPath::Toolbox("sb_test"))
        );
    }

    #[test]
    fn test_daytona_api_call_filters_sandbox_list() {
        let filtered = DaytonaApiCallTool::filter_owned_sandboxes(
            json!([
                {"id": "sb_owned", "state": "started"},
                {"id": "sb_other", "state": "started"}
            ]),
            &HashSet::from(["sb_owned".to_string()]),
        );
        assert_eq!(
            filtered,
            json!([
                {"id": "sb_owned", "state": "started"}
            ])
        );
    }

    #[test]
    fn test_extract_owner_repo_bare_word() {
        assert_eq!(extract_owner_repo("something"), None);
    }

    #[test]
    fn test_extract_owner_repo_http() {
        assert_eq!(
            extract_owner_repo("http://github.com/user/repo"),
            Some("user/repo".to_string())
        );
    }

    #[test]
    fn test_extract_owner_repo_with_spaces() {
        assert_eq!(extract_owner_repo("user /repo"), None);
    }

    #[test]
    fn test_extract_owner_repo_rejects_traversal_segments() {
        assert_eq!(extract_owner_repo("../repo"), None);
        assert_eq!(extract_owner_repo("user/.."), None);
        assert_eq!(extract_owner_repo("https://github.com/../repo"), None);
        assert_eq!(extract_owner_repo("git@github.com:user/../.git"), None);
    }

    #[test]
    fn test_exec_schema_has_optional_fields() {
        let tool = DaytonaExecTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        // cwd and timeout are optional
        assert!(!required_strs.contains(&"cwd"));
        assert!(!required_strs.contains(&"timeout"));
        // but they are in properties
        assert!(schema["properties"]["cwd"].is_object());
        assert!(schema["properties"]["timeout"].is_object());
    }

    #[test]
    fn test_create_sandbox_schema_has_upload_files() {
        let tool = DaytonaCreateSandboxTool;
        let schema = tool.parameters_schema();
        let upload_files = &schema["properties"]["upload_files"];
        assert_eq!(upload_files["type"], "array");
        let item_required = upload_files["items"]["required"].as_array().unwrap();
        let strs: Vec<&str> = item_required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(strs.contains(&"session_path"));
        assert!(strs.contains(&"sandbox_path"));
    }

    // ========================================================================
    // Git credentials tool - context-aware integration tests
    // ========================================================================

    #[tokio::test]
    async fn test_git_credentials_missing_sandbox_id() {
        let tool = DaytonaGitCredentialsTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let context = ToolContext::with_storage_store(session_id, store);
        let result = tool.execute_with_context(json!({}), &context).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("Missing required parameter"),
                    "Expected missing param error, got: {msg}"
                );
            }
            _ => panic!("Expected ToolError for missing sandbox_id"),
        }
    }

    #[tokio::test]
    async fn test_git_credentials_no_api_key() {
        let tool = DaytonaGitCredentialsTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        // No connection resolver — API key cannot be resolved
        let context = ToolContext::with_storage_store(session_id, store);
        let result = tool
            .execute_with_context(json!({"sandbox_id": "sb_test"}), &context)
            .await;
        match result {
            ToolExecutionResult::ConnectionRequired { provider } => {
                assert_eq!(provider, "daytona");
            }
            _ => panic!("Expected ConnectionRequired for missing API key, got: {result:?}"),
        }
    }

    #[tokio::test]
    async fn test_git_credentials_no_sandbox_state() {
        let tool = DaytonaGitCredentialsTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        // Connection resolver provides Daytona API key but no sandbox state
        let resolver = Arc::new(ProviderAwareResolver::daytona_only("test_key"));
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let result = tool
            .execute_with_context(json!({"sandbox_id": "sb_test"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("not found"),
                    "Expected sandbox not found error, got: {msg}"
                );
            }
            _ => panic!("Expected ToolError for missing sandbox state"),
        }
    }

    #[tokio::test]
    async fn test_git_credentials_no_github_token() {
        let tool = DaytonaGitCredentialsTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        // Seed sandbox state
        let state = SandboxState {
            sandbox_id: "sb_test".to_string(),
            workspace_path: "/home/daytona".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
        };
        store
            .seed_secret(
                session_id,
                "daytona_sandbox:sb_test",
                &serde_json::to_string(&state).unwrap(),
            )
            .await;
        // Daytona API key via connection resolver, but no GitHub token
        let resolver = Arc::new(ProviderAwareResolver::daytona_only("test_key"));
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let result = tool
            .execute_with_context(json!({"sandbox_id": "sb_test"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("No GitHub credentials"),
                    "Expected no credentials error, got: {msg}"
                );
                // Should suggest Settings > Connections
                assert!(
                    msg.contains("Settings > Connections"),
                    "Expected connection hint, got: {msg}"
                );
            }
            _ => panic!("Expected ToolError for missing GitHub token"),
        }
    }

    #[tokio::test]
    async fn test_git_credentials_with_github_token_secret() {
        // When GITHUB_TOKEN session secret is set, the tool should attempt to
        // upload credentials. It will fail because the Daytona API is not
        // reachable, but the token resolution should succeed.
        let tool = DaytonaGitCredentialsTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let state = SandboxState {
            sandbox_id: "sb_test".to_string(),
            workspace_path: "/home/daytona".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
        };
        store
            .seed_secret(
                session_id,
                "daytona_sandbox:sb_test",
                &serde_json::to_string(&state).unwrap(),
            )
            .await;
        store
            .seed_secret(session_id, "GITHUB_TOKEN", "ghp_test_token_123")
            .await;
        // Daytona API key via connection resolver (but not GitHub — that comes from secret)
        let resolver = Arc::new(ProviderAwareResolver::daytona_only("test_key"));
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let result = tool
            .execute_with_context(json!({"sandbox_id": "sb_test"}), &context)
            .await;
        // Should fail at HTTP call (no real Daytona API), not at token resolution
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(
                !msg.contains("No GitHub credentials"),
                "Token should have been resolved from GITHUB_TOKEN secret, got: {msg}"
            );
            // Should fail with connection/upload error (not auth error)
            assert!(
                msg.contains("Failed to write credentials file"),
                "Expected HTTP failure, got: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn test_git_credentials_with_connection_resolver() {
        // When connection resolver provides a GitHub token, it should be used.
        let tool = DaytonaGitCredentialsTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let state = SandboxState {
            sandbox_id: "sb_test".to_string(),
            workspace_path: "/home/daytona".to_string(),
            started_at: "2026-01-01T00:00:00Z".to_string(),
        };
        store
            .seed_secret(
                session_id,
                "daytona_sandbox:sb_test",
                &serde_json::to_string(&state).unwrap(),
            )
            .await;
        // Connection resolver provides both Daytona key and GitHub token
        let resolver = Arc::new(MockConnectionResolver {
            token: Some("ghs_connection_token_456".to_string()),
        });
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let result = tool
            .execute_with_context(json!({"sandbox_id": "sb_test"}), &context)
            .await;
        // Should fail at HTTP call, not at token resolution
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(
                !msg.contains("No GitHub credentials"),
                "Token should have been resolved from connection resolver, got: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn test_git_credentials_no_storage_store() {
        let tool = DaytonaGitCredentialsTool;
        let session_id = SessionId::new();
        // No storage store at all — get_api_key() returns ConnectionRequired
        let context = ToolContext::new(session_id);
        let result = tool
            .execute_with_context(json!({"sandbox_id": "sb_test"}), &context)
            .await;
        match result {
            ToolExecutionResult::ConnectionRequired { provider } => {
                assert_eq!(provider, "daytona");
            }
            _ => panic!("Expected ConnectionRequired for missing API key, got: {result:?}"),
        }
    }

    // ========================================================================
    // Git clone tool - context-aware integration tests
    // ========================================================================

    #[tokio::test]
    async fn test_git_clone_missing_repo_url() {
        let tool = DaytonaGitCloneTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let context = ToolContext::with_storage_store(session_id, store);
        let result = tool
            .execute_with_context(json!({"sandbox_id": "sb_test"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("Missing required parameter"),
                    "Expected missing param error, got: {msg}"
                );
            }
            _ => panic!("Expected ToolError for missing repo_url"),
        }
    }

    #[tokio::test]
    async fn test_git_clone_no_api_key() {
        let tool = DaytonaGitCloneTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        // No connection resolver — API key cannot be resolved
        let context = ToolContext::with_storage_store(session_id, store);
        let result = tool
            .execute_with_context(
                json!({"sandbox_id": "sb_test", "repo_url": "user/repo"}),
                &context,
            )
            .await;
        match result {
            ToolExecutionResult::ConnectionRequired { provider } => {
                assert_eq!(provider, "daytona");
            }
            _ => panic!("Expected ConnectionRequired for missing API key, got: {result:?}"),
        }
    }

    // ========================================================================
    // get_github_token helper tests
    // ========================================================================

    #[tokio::test]
    async fn test_get_github_token_prefers_connection_resolver() {
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        store
            .seed_secret(session_id, "GITHUB_TOKEN", "fallback_token")
            .await;
        let resolver = Arc::new(MockConnectionResolver {
            token: Some("preferred_token".to_string()),
        });
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let token = get_github_token(&context).await;
        assert_eq!(token, Some("preferred_token".to_string()));
    }

    #[tokio::test]
    async fn test_get_github_token_falls_back_to_secret() {
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        store
            .seed_secret(session_id, "GITHUB_TOKEN", "secret_token")
            .await;
        // No connection resolver
        let context = ToolContext::with_storage_store(session_id, store);
        let token = get_github_token(&context).await;
        assert_eq!(token, Some("secret_token".to_string()));
    }

    #[tokio::test]
    async fn test_get_github_token_returns_none_when_empty() {
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let resolver = Arc::new(MockConnectionResolver { token: None });
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let token = get_github_token(&context).await;
        assert_eq!(token, None);
    }

    #[tokio::test]
    async fn test_get_github_token_skips_empty_connection_token() {
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        store
            .seed_secret(session_id, "GITHUB_TOKEN", "fallback")
            .await;
        // Resolver returns empty string — should fall through
        let resolver = Arc::new(MockConnectionResolver {
            token: Some("".to_string()),
        });
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let token = get_github_token(&context).await;
        assert_eq!(token, Some("fallback".to_string()));
    }

    #[test]
    fn test_exit_code_hint_signals() {
        assert!(exit_code_hint(0).is_none());
        assert!(exit_code_hint(1).is_none());
        assert!(exit_code_hint(137).unwrap().contains("SIGKILL"));
        assert!(exit_code_hint(139).unwrap().contains("SIGSEGV"));
        assert!(exit_code_hint(141).unwrap().contains("SIGPIPE"));
        assert!(exit_code_hint(143).unwrap().contains("SIGTERM"));
        assert!(exit_code_hint(126).unwrap().contains("not executable"));
        assert!(exit_code_hint(127).unwrap().contains("not found"));
        // Generic signal range
        assert!(exit_code_hint(130).unwrap().contains("signal"));
        // Outside signal range
        assert!(exit_code_hint(200).is_none());
    }

    #[test]
    fn test_build_exec_tool_result_omits_unknown_cwd_and_uses_total_lines() {
        let result = crate::state::ExecResult {
            stdout: "line one\nline two\n".to_string(),
            stderr: "warning\n".to_string(),
            result: "line one\nline two\nwarning\n".to_string(),
            exit_code: 0,
        };

        let output = build_exec_tool_result("sb_test", None, &result, "normal");
        match output {
            ToolExecutionResult::Success(value) => {
                assert_eq!(value["sandbox_id"], "sb_test");
                assert_eq!(value["stdout"], "line one\nline two\n");
                assert_eq!(value["stderr"], "warning\n");
                assert_eq!(value["total_lines"], 2);
                assert!(value.get("cwd").is_none(), "cwd should be omitted");
                assert!(value.get("stdout_lines").is_none());
                assert!(value.get("stderr_lines").is_none());
            }
            other => panic!("Expected Success, got {other:?}"),
        }
    }

    // ========================================================================
    // DaytonaApiCallTool tests
    // ========================================================================

    #[test]
    fn test_api_call_tool_name() {
        let tool = DaytonaApiCallTool;
        assert_eq!(tool.name(), "daytona_api_call");
        assert!(tool.name().starts_with("daytona_"));
    }

    #[test]
    fn test_api_call_tool_schema() {
        let tool = DaytonaApiCallTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"method"));
        assert!(required_strs.contains(&"path"));
        assert!(!required_strs.contains(&"body"));
        assert_eq!(schema["additionalProperties"], json!(false));

        // method must have enum constraint
        let method_enum = schema["properties"]["method"]["enum"].as_array().unwrap();
        let methods: Vec<&str> = method_enum.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(methods.contains(&"GET"));
        assert!(methods.contains(&"POST"));
        assert!(methods.contains(&"DELETE"));
    }

    #[test]
    fn test_api_call_tool_description_references_spec() {
        let tool = DaytonaApiCallTool;
        assert!(tool.description().contains("/daytona/openapi.yaml"));
    }

    #[test]
    fn test_api_call_tool_hints() {
        let tool = DaytonaApiCallTool;
        let hints = tool.hints();
        assert_eq!(hints.open_world, Some(true));
        assert_eq!(hints.requires_secrets, Some(true));
    }

    #[tokio::test]
    async fn test_api_call_without_context() {
        let tool = DaytonaApiCallTool;
        let result = tool
            .execute(json!({"method": "GET", "path": "/sandbox"}))
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("requires context"))
        );
    }

    #[tokio::test]
    async fn test_api_call_missing_method() {
        let tool = DaytonaApiCallTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let resolver = Arc::new(ProviderAwareResolver::daytona_only("test_key"));
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let result = tool
            .execute_with_context(json!({"path": "/sandbox"}), &context)
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("Missing required parameter"))
        );
    }

    #[tokio::test]
    async fn test_api_call_missing_path() {
        let tool = DaytonaApiCallTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let resolver = Arc::new(ProviderAwareResolver::daytona_only("test_key"));
        let context =
            ToolContext::with_storage_store(session_id, store).with_connection_resolver(resolver);
        let result = tool
            .execute_with_context(json!({"method": "GET"}), &context)
            .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(msg) if msg.contains("Missing required parameter"))
        );
    }

    #[tokio::test]
    async fn test_api_call_no_api_key() {
        let tool = DaytonaApiCallTool;
        let session_id = SessionId::new();
        let store = Arc::new(MockStorageStore::new());
        let context = ToolContext::with_storage_store(session_id, store);
        let result = tool
            .execute_with_context(json!({"method": "GET", "path": "/sandbox"}), &context)
            .await;
        match result {
            ToolExecutionResult::ConnectionRequired { provider } => {
                assert_eq!(provider, "daytona");
            }
            _ => panic!("Expected ConnectionRequired for missing API key, got: {result:?}"),
        }
    }

    #[test]
    fn test_openapi_mount_path_constant() {
        assert_eq!(DAYTONA_OPENAPI_MOUNT_PATH, "/daytona/openapi.yaml");
    }

    // ========================================================================
    // Tool narration
    // ========================================================================

    use everruns_core::tool_narration::{ToolNarrationContext, ToolNarrationPhase};
    use everruns_provider::tool_types::ToolCall;

    fn narrate(tool: &dyn Tool, arguments: Value, phase: ToolNarrationPhase) -> Option<String> {
        let call = ToolCall {
            id: "call-1".to_string(),
            name: tool.name().to_string(),
            arguments,
        };
        tool.narrate(&call, phase, None, ToolNarrationContext::default())
    }

    #[test]
    fn test_narrate_create_sandbox_all_phases() {
        let tool = DaytonaCreateSandboxTool;
        assert_eq!(
            narrate(&tool, json!({}), ToolNarrationPhase::Started).as_deref(),
            Some("Creating Daytona sandbox")
        );
        assert_eq!(
            narrate(&tool, json!({}), ToolNarrationPhase::Completed).as_deref(),
            Some("Created Daytona sandbox")
        );
        assert_eq!(
            narrate(&tool, json!({}), ToolNarrationPhase::Failed).as_deref(),
            Some("Failed to create Daytona sandbox")
        );
    }

    #[test]
    fn test_narrate_read_file_uses_basename() {
        let tool = DaytonaReadFileTool;
        assert_eq!(
            narrate(
                &tool,
                json!({"sandbox_id": "s", "path": "/home/daytona/main.py"}),
                ToolNarrationPhase::Completed
            )
            .as_deref(),
            Some("Read main.py")
        );
    }

    #[test]
    fn test_narrate_manage_sandbox_labels_action() {
        let tool = DaytonaManageSandboxTool;
        assert_eq!(
            narrate(
                &tool,
                json!({"sandbox_id": "s", "action": "stop"}),
                ToolNarrationPhase::Started
            )
            .as_deref(),
            Some("Managing sandbox: stop")
        );
        // No action arg -> bare verb fallback.
        assert_eq!(
            narrate(&tool, json!({}), ToolNarrationPhase::Failed).as_deref(),
            Some("Failed to manage sandbox")
        );
    }

    #[test]
    fn test_narrate_git_clone_strips_credentials() {
        let tool = DaytonaGitCloneTool;
        // url_display strips scheme + userinfo, keeping host/path.
        assert_eq!(
            narrate(
                &tool,
                json!({"sandbox_id": "s", "repo_url": "https://oauth2:secret@github.com/owner/repo.git"}),
                ToolNarrationPhase::Started
            )
            .as_deref(),
            Some("Cloning repository: github.com/owner/repo.git")
        );
        // No url arg -> bare verb fallback.
        assert_eq!(
            narrate(&tool, json!({}), ToolNarrationPhase::Completed).as_deref(),
            Some("Cloned repository")
        );
    }

    #[test]
    fn test_narrate_git_credentials_never_renders_args() {
        let tool = DaytonaGitCredentialsTool;
        assert_eq!(
            narrate(
                &tool,
                json!({"sandbox_id": "super-secret"}),
                ToolNarrationPhase::Started
            )
            .as_deref(),
            Some("Configuring git credentials")
        );
    }

    #[test]
    fn test_narrate_api_call_truncates_path() {
        let tool = DaytonaApiCallTool;
        let long_path = format!("/toolbox/{}/files", "x".repeat(80));
        let narration = narrate(
            &tool,
            json!({"method": "GET", "path": long_path}),
            ToolNarrationPhase::Started,
        )
        .unwrap();
        assert!(narration.starts_with("Calling Daytona API: /toolbox/"));
        assert!(narration.ends_with("..."));
    }
}
