//! Agent tools for Modal sandboxes.

use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::runtime::exec_tool_result::ExecToolResultPayload;
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::runtime::tool_narration::{
    ToolNarrationContext, ToolNarrationPhase, narrate_labeled_action, narrate_shell_exec,
};
use everruns_contracts::runtime::tool_output_sanitizer::{
    READ_FILE_DEFAULT_LIMIT, build_text_read_file_result, output_verbosity_schema,
    parse_read_file_window_args,
};
use everruns_contracts::runtime::tools::{Tool, ToolExecutionResult};
use everruns_contracts::tool_types::{ToolCall, ToolHints};
use serde_json::{Value, json};
use tracing::warn;

use super::client::{CreateSandboxParams, ModalClient, SandboxStatus};
use super::state::{
    ModalSandboxState, client_for, delete_sandbox_state, get_credentials, get_sandbox_state,
    list_sandbox_states, release_sandbox_lease, required_str, save_sandbox_state,
    touch_sandbox_lease,
};
use super::{
    MODAL_APP_NAME, MODAL_DEFAULT_EXEC_TIMEOUT_SECS, MODAL_DEFAULT_IMAGE, MODAL_DEFAULT_RUNTIME,
    MODAL_DEFAULT_SETUP_COMMANDS, MODAL_DEFAULT_TIMEOUT_SECS, MODAL_MAX_EXEC_TIMEOUT_SECS,
    MODAL_MAX_TIMEOUT_SECS, MODAL_WORKSPACE_PATH,
};

/// Turn a `Result<T, ToolExecutionResult>` into `T` or return the error result.
macro_rules! try_tool {
    ($expr:expr) => {
        match $expr {
            Ok(value) => value,
            Err(err) => return err,
        }
    };
}

fn context_required(name: &str) -> ToolExecutionResult {
    ToolExecutionResult::tool_error(format!(
        "{name} requires context. This tool must be executed with session context."
    ))
}

fn optional_u32(
    args: &Value,
    name: &str,
    min: u32,
    max: u32,
) -> Result<Option<u32>, ToolExecutionResult> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .filter(|v| (u64::from(min)..=u64::from(max)).contains(v))
            .map(|v| Some(v as u32))
            .ok_or_else(|| {
                ToolExecutionResult::tool_error(format!(
                    "Invalid '{name}': must be an integer between {min} and {max}"
                ))
            }),
    }
}

/// Registry tags: `[registry/]name[:tag][@digest]`, no whitespace or shell metacharacters.
pub(crate) fn validate_image_tag(tag: &str) -> Result<(), ToolExecutionResult> {
    let ok = !tag.is_empty()
        && tag.len() <= 256
        && tag
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"./:-_@".contains(&b));
    if ok {
        Ok(())
    } else {
        Err(ToolExecutionResult::tool_error(format!(
            "Invalid 'image': '{tag}' is not a container registry reference"
        )))
    }
}

fn validate_image_id(image_id: &str) -> Result<(), ToolExecutionResult> {
    let ok = image_id.len() <= 64
        && image_id.strip_prefix("im-").is_some_and(|rest| {
            !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric())
        });
    if ok {
        Ok(())
    } else {
        Err(ToolExecutionResult::tool_error(format!(
            "Invalid 'snapshot_image_id': '{image_id}' is not a Modal image ID (im-...)"
        )))
    }
}

/// When a router call fails, say whether the sandbox itself is gone, which is
/// the usual cause and the one the agent can act on.
async fn explain_failure(
    client: &ModalClient,
    state: &ModalSandboxState,
    error: String,
) -> ToolExecutionResult {
    match client.sandbox_status(&state.sandbox_id).await {
        Ok(SandboxStatus::Finished { status, exception }) => {
            ToolExecutionResult::tool_error(format!(
                "Modal sandbox {} is no longer running (status: {status}{}). Create a new one with modal_create_sandbox.",
                state.sandbox_id,
                exception.map(|e| format!(", {e}")).unwrap_or_default()
            ))
        }
        _ => ToolExecutionResult::tool_error(error),
    }
}

fn resolve_path(state: &ModalSandboxState, path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{}/{}", state.workspace_path.trim_end_matches('/'), path)
    }
}

// ============================================================================
// modal_create_sandbox
// ============================================================================

/// Create a Modal sandbox.
pub struct ModalCreateSandboxTool;

#[async_trait]
impl Tool for ModalCreateSandboxTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            (
                "Creating Modal sandbox",
                "Created Modal sandbox",
                "Could not create Modal sandbox",
            ),
            (
                "Створюю пісочницю Modal",
                "Створив пісочницю Modal",
                "Не вдалося створити пісочницю Modal",
            ),
            &["title", "image"],
        ))
    }

    fn name(&self) -> &str {
        "modal_create_sandbox"
    }

    fn description(&self) -> &str {
        "Create a Modal sandbox: a full Linux VM (runtime \"vm\", the default; supports Docker, \
         FUSE, systemd-style workloads) or a lighter gVisor container (runtime \"gvisor\"). \
         Boots from a registry image, optional Dockerfile setup commands, or a filesystem snapshot."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {"type": "string", "description": "Human-readable label"},
                "runtime": {"type": "string", "enum": ["vm", "gvisor"], "default": MODAL_DEFAULT_RUNTIME},
                "image": {
                    "type": "string",
                    "description": format!("Container registry image to boot (default {MODAL_DEFAULT_IMAGE} with git and curl)")
                },
                "setup_commands": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": 20,
                    "description": "Dockerfile lines applied on top of the image, e.g. \"RUN pip install numpy\". Modal caches the built image."
                },
                "snapshot_image_id": {
                    "type": "string",
                    "description": "Boot from a filesystem snapshot (im-...) taken with modal_snapshot_sandbox instead of an image"
                },
                "timeout_seconds": {
                    "type": "integer", "minimum": 60, "maximum": MODAL_MAX_TIMEOUT_SECS,
                    "default": MODAL_DEFAULT_TIMEOUT_SECS,
                    "description": "Maximum sandbox lifetime"
                },
                "idle_timeout_seconds": {
                    "type": "integer", "minimum": 60, "maximum": MODAL_MAX_TIMEOUT_SECS,
                    "description": "Terminate after this long without activity"
                },
                "cpu": {"type": "number", "minimum": 0.125, "maximum": 64, "description": "CPU cores to request"},
                "memory_mb": {"type": "integer", "minimum": 128, "maximum": 262144, "description": "Memory to request in MiB"},
                "expose_ports": {
                    "type": "array",
                    "items": {"type": "integer", "minimum": 1, "maximum": 65535},
                    "maxItems": 10,
                    "description": "Ports to publish on public HTTPS URLs (see modal_tunnel_urls)"
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let runtime = arguments
            .get("runtime")
            .and_then(Value::as_str)
            .unwrap_or(MODAL_DEFAULT_RUNTIME);
        if !matches!(runtime, "vm" | "gvisor") {
            return ToolExecutionResult::tool_error(
                "Invalid 'runtime': expected \"vm\" or \"gvisor\"",
            );
        }
        let timeout_secs = try_tool!(optional_u32(
            &arguments,
            "timeout_seconds",
            60,
            MODAL_MAX_TIMEOUT_SECS
        ))
        .unwrap_or(MODAL_DEFAULT_TIMEOUT_SECS);
        let idle_timeout_secs = try_tool!(optional_u32(
            &arguments,
            "idle_timeout_seconds",
            60,
            MODAL_MAX_TIMEOUT_SECS
        ));
        let memory_mb = try_tool!(optional_u32(&arguments, "memory_mb", 128, 262_144));
        let milli_cpu = match arguments.get("cpu") {
            None | Some(Value::Null) => None,
            Some(value) => match value.as_f64().filter(|c| (0.125..=64.0).contains(c)) {
                Some(cores) => Some((cores * 1000.0).round() as u32),
                None => {
                    return ToolExecutionResult::tool_error(
                        "Invalid 'cpu': must be a number between 0.125 and 64",
                    );
                }
            },
        };
        let mut ports = Vec::new();
        if let Some(value) = arguments.get("expose_ports").filter(|v| !v.is_null()) {
            let Some(items) = value.as_array().filter(|a| a.len() <= 10) else {
                return ToolExecutionResult::tool_error(
                    "Invalid 'expose_ports': expected at most 10 port numbers",
                );
            };
            for item in items {
                match item.as_u64().filter(|p| (1..=65_535).contains(p)) {
                    Some(port) => ports.push(port as u32),
                    None => {
                        return ToolExecutionResult::tool_error(
                            "Invalid 'expose_ports': ports must be 1-65535",
                        );
                    }
                }
            }
            ports.sort_unstable();
            ports.dedup();
        }
        let snapshot = arguments.get("snapshot_image_id").and_then(Value::as_str);
        let image = arguments.get("image").and_then(Value::as_str);
        let setup_commands: Vec<String> = match arguments
            .get("setup_commands")
            .filter(|v| !v.is_null())
        {
            None => Vec::new(),
            Some(value) => {
                let Some(items) = value.as_array().filter(|a| a.len() <= 20) else {
                    return ToolExecutionResult::tool_error(
                        "Invalid 'setup_commands': expected at most 20 strings",
                    );
                };
                let mut commands = Vec::new();
                for item in items {
                    match item.as_str() {
                        Some(line) if !line.contains('\n') && !line.trim().is_empty() => {
                            commands.push(line.trim().to_string());
                        }
                        _ => {
                            return ToolExecutionResult::tool_error(
                                "Invalid 'setup_commands': each entry must be one non-empty Dockerfile line",
                            );
                        }
                    }
                }
                commands
            }
        };
        if snapshot.is_some() && (image.is_some() || !setup_commands.is_empty()) {
            return ToolExecutionResult::tool_error(
                "'snapshot_image_id' cannot be combined with 'image' or 'setup_commands'",
            );
        }
        if let Some(snapshot) = snapshot {
            try_tool!(validate_image_id(snapshot));
        }
        if let Some(image) = image {
            try_tool!(validate_image_tag(image));
        }
        let title = arguments
            .get("title")
            .and_then(Value::as_str)
            .map(|t| t.chars().take(120).collect::<String>());

        let credentials = try_tool!(get_credentials(context).await);
        let client = try_tool!(client_for(credentials, context));

        let app_id = match client.get_or_create_app(MODAL_APP_NAME).await {
            Ok(id) => id,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };
        let (image_id, image_label) = match snapshot {
            Some(snapshot) => (snapshot.to_string(), snapshot.to_string()),
            None => {
                let tag = image.unwrap_or(MODAL_DEFAULT_IMAGE);
                let commands = if image.is_none() && setup_commands.is_empty() {
                    MODAL_DEFAULT_SETUP_COMMANDS
                        .iter()
                        .map(|c| (*c).to_string())
                        .collect()
                } else {
                    setup_commands
                };
                match client.build_image(&app_id, tag, &commands).await {
                    Ok(id) => (id, tag.to_string()),
                    Err(err) => return ToolExecutionResult::tool_error(err),
                }
            }
        };

        let mut tags = vec![
            ("everruns".to_string(), "true".to_string()),
            (
                "everruns.session_id".to_string(),
                context.session_id.to_string(),
            ),
        ];
        if let Some(title) = &title {
            tags.push(("everruns.title".to_string(), title.clone()));
        }
        let params = CreateSandboxParams {
            image_id,
            runtime: Some(runtime.to_string()),
            timeout_secs,
            idle_timeout_secs,
            workdir: None,
            milli_cpu,
            memory_mb,
            encrypted_ports: ports.clone(),
            tags,
        };
        let (sandbox_id, task_id) = match client.create_sandbox(&app_id, &params).await {
            Ok(ids) => ids,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };

        let state = ModalSandboxState {
            sandbox_id: sandbox_id.clone(),
            task_id,
            app_id,
            runtime: runtime.to_string(),
            image: image_label,
            workspace_path: MODAL_WORKSPACE_PATH.to_string(),
            started_at: chrono::Utc::now().to_rfc3339(),
            timeout_seconds: timeout_secs,
            title,
            exposed_ports: ports.clone(),
        };
        // A sandbox the session cannot record is one nothing would clean up.
        if let Err(err) = save_sandbox_state(context, &state).await {
            let _ = client.terminate(&sandbox_id).await;
            return err;
        }
        if let Err(err) = touch_sandbox_lease(context, &state).await {
            let _ = client.terminate(&sandbox_id).await;
            let _ = delete_sandbox_state(context, &sandbox_id).await;
            return err;
        }

        let mkdir = vec![
            "mkdir".to_string(),
            "-p".to_string(),
            MODAL_WORKSPACE_PATH.to_string(),
        ];
        if let Err(err) = client
            .exec(
                &state.task_id,
                &mkdir,
                None,
                &HashMap::new(),
                Duration::from_secs(60),
                None,
            )
            .await
        {
            warn!("Failed to create Modal workspace directory: {err}");
        }

        let mut result = json!({
            "sandbox_id": sandbox_id,
            "status": "running",
            "runtime": runtime,
            "image": state.image,
            "workspace_path": state.workspace_path,
            "timeout_seconds": timeout_secs,
        });
        if !ports.is_empty() {
            match client.tunnels(&sandbox_id).await {
                Ok(tunnels) => {
                    result["tunnels"] = json!(
                        tunnels
                            .iter()
                            .map(|t| json!({
                                "port": t.container_port,
                                "url": t.url,
                            }))
                            .collect::<Vec<_>>()
                    );
                }
                Err(err) => warn!("Failed to read Modal tunnels after create: {err}"),
            }
        }
        ToolExecutionResult::success(result)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// modal_exec
// ============================================================================

/// Run a shell command in a sandbox.
pub struct ModalExecTool;

#[async_trait]
impl Tool for ModalExecTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let fallback = self.display_name().unwrap_or("Modal");
        Some(narrate_shell_exec(&call.arguments, fallback, phase, locale))
    }

    fn name(&self) -> &str {
        "modal_exec"
    }

    fn description(&self) -> &str {
        "Run a shell command in a Modal sandbox and return stdout, stderr and the exit code."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "command": {"type": "string", "description": "Shell command, run with sh -c"},
                "cwd": {"type": "string", "description": format!("Working directory (default {MODAL_WORKSPACE_PATH})")},
                "env": {
                    "type": "object",
                    "additionalProperties": {"type": "string"},
                    "description": "Extra environment variables for this command"
                },
                "timeout_seconds": {
                    "type": "integer", "minimum": 1, "maximum": MODAL_MAX_EXEC_TIMEOUT_SECS,
                    "default": MODAL_DEFAULT_EXEC_TIMEOUT_SECS
                },
                "output": output_verbosity_schema()
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = try_tool!(required_str(&arguments, "sandbox_id"));
        let command = try_tool!(required_str(&arguments, "command"));
        let timeout_secs = try_tool!(optional_u32(
            &arguments,
            "timeout_seconds",
            1,
            MODAL_MAX_EXEC_TIMEOUT_SECS
        ))
        .unwrap_or(MODAL_DEFAULT_EXEC_TIMEOUT_SECS);
        let mut env = HashMap::new();
        if let Some(value) = arguments.get("env").filter(|v| !v.is_null()) {
            let Some(map) = value.as_object() else {
                return ToolExecutionResult::tool_error(
                    "Invalid 'env': expected an object of strings",
                );
            };
            for (key, value) in map {
                let Some(value) = value.as_str() else {
                    return ToolExecutionResult::tool_error(format!(
                        "Invalid 'env.{key}': expected a string"
                    ));
                };
                env.insert(key.clone(), value.to_string());
            }
        }
        let output_mode = arguments
            .get("output")
            .and_then(Value::as_str)
            .unwrap_or("auto");

        let credentials = try_tool!(get_credentials(context).await);
        let state = try_tool!(get_sandbox_state(context, sandbox_id).await);
        let client = try_tool!(client_for(credentials, context));
        let cwd = arguments
            .get("cwd")
            .and_then(Value::as_str)
            .map(|cwd| resolve_path(&state, cwd))
            .unwrap_or_else(|| state.workspace_path.clone());

        let argv = vec!["sh".to_string(), "-c".to_string(), command.to_string()];
        let output = match client
            .exec(
                &state.task_id,
                &argv,
                Some(&cwd),
                &env,
                Duration::from_secs(u64::from(timeout_secs)),
                None,
            )
            .await
        {
            Ok(output) => output,
            Err(err) => return explain_failure(&client, &state, err).await,
        };
        if let Err(err) = touch_sandbox_lease(context, &state).await {
            return err;
        }

        let ExecToolResultPayload {
            stdout,
            stderr,
            exit_code,
            success,
            truncated,
            total_lines,
            raw_output,
        } = ExecToolResultPayload::new(
            &output.stdout,
            &output.stderr,
            output.exit_code,
            output_mode,
        );
        ToolExecutionResult::success_with_raw_output(
            json!({
                "sandbox_id": sandbox_id,
                "cwd": cwd,
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": exit_code,
                "success": success,
                "truncated": truncated || output.truncated,
                "total_lines": total_lines,
            }),
            raw_output,
        )
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// modal_read_file / modal_write_file
// ============================================================================

/// Read a text file from a sandbox.
pub struct ModalReadFileTool;

#[async_trait]
impl Tool for ModalReadFileTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            (
                "Reading Modal file",
                "Read Modal file",
                "Could not read Modal file",
            ),
            (
                "Читаю файл Modal",
                "Прочитав файл Modal",
                "Не вдалося прочитати файл Modal",
            ),
            &["path"],
        ))
    }

    fn name(&self) -> &str {
        "modal_read_file"
    }

    fn description(&self) -> &str {
        "Read a text file from a Modal sandbox filesystem (NOT the session /workspace). Relative paths resolve against the sandbox workspace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "path": {"type": "string"},
                "offset": {"type": "integer", "minimum": 0, "default": 0, "description": "Zero-based line offset"},
                "limit": {"type": "integer", "minimum": 1, "default": READ_FILE_DEFAULT_LIMIT, "description": "Maximum lines to return"}
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = try_tool!(required_str(&arguments, "sandbox_id"));
        let path = try_tool!(required_str(&arguments, "path"));
        let (offset, limit) = match parse_read_file_window_args(&arguments) {
            Ok(window) => window,
            Err(err) => return ToolExecutionResult::tool_error(err),
        };
        let credentials = try_tool!(get_credentials(context).await);
        let state = try_tool!(get_sandbox_state(context, sandbox_id).await);
        let client = try_tool!(client_for(credentials, context));
        let full_path = resolve_path(&state, path);
        let argv = vec!["cat".to_string(), "--".to_string(), full_path.clone()];
        let output = match client
            .exec(
                &state.task_id,
                &argv,
                None,
                &HashMap::new(),
                Duration::from_secs(120),
                None,
            )
            .await
        {
            Ok(output) => output,
            Err(err) => return explain_failure(&client, &state, err).await,
        };
        if output.exit_code != 0 {
            return ToolExecutionResult::tool_error(format!(
                "Failed to read {full_path}: {}",
                output.stderr.trim()
            ));
        }
        if let Err(err) = touch_sandbox_lease(context, &state).await {
            return err;
        }
        let mut result = build_text_read_file_result(
            "modal_read_file",
            &full_path,
            &output.stdout,
            "text",
            offset,
            limit,
        );
        result["sandbox_id"] = json!(sandbox_id);
        ToolExecutionResult::success(result)
    }

    fn requires_context(&self) -> bool {
        true
    }
}

/// Write a text file into a sandbox.
pub struct ModalWriteFileTool;

/// Largest file `modal_write_file` accepts.
const MAX_WRITE_BYTES: usize = 10 * 1024 * 1024;

#[async_trait]
impl Tool for ModalWriteFileTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            (
                "Writing Modal file",
                "Wrote Modal file",
                "Could not write Modal file",
            ),
            (
                "Записую файл Modal",
                "Записав файл Modal",
                "Не вдалося записати файл Modal",
            ),
            &["path"],
        ))
    }

    fn name(&self) -> &str {
        "modal_write_file"
    }

    fn description(&self) -> &str {
        "Write a text file into a Modal sandbox, creating parent directories. Relative paths resolve against the sandbox workspace."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "path": {"type": "string"},
                "content": {"type": "string"}
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = try_tool!(required_str(&arguments, "sandbox_id"));
        let path = try_tool!(required_str(&arguments, "path"));
        let content = try_tool!(required_str(&arguments, "content"));
        if content.len() > MAX_WRITE_BYTES {
            return ToolExecutionResult::tool_error(format!(
                "Content is {} bytes; modal_write_file accepts at most {MAX_WRITE_BYTES}",
                content.len()
            ));
        }
        let credentials = try_tool!(get_credentials(context).await);
        let state = try_tool!(get_sandbox_state(context, sandbox_id).await);
        let client = try_tool!(client_for(credentials, context));
        let full_path = resolve_path(&state, path);
        // The path travels as a positional argument, never through the
        // script text, so no quoting of model-supplied input is needed.
        let argv = vec![
            "sh".to_string(),
            "-c".to_string(),
            "mkdir -p -- \"$(dirname -- \"$1\")\" && cat > \"$1\"".to_string(),
            "sh".to_string(),
            full_path.clone(),
        ];
        let output = match client
            .exec(
                &state.task_id,
                &argv,
                None,
                &HashMap::new(),
                Duration::from_secs(120),
                Some(content.as_bytes()),
            )
            .await
        {
            Ok(output) => output,
            Err(err) => return explain_failure(&client, &state, err).await,
        };
        if output.exit_code != 0 {
            return ToolExecutionResult::tool_error(format!(
                "Failed to write {full_path}: {}",
                output.stderr.trim()
            ));
        }
        if let Err(err) = touch_sandbox_lease(context, &state).await {
            return err;
        }
        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "path": full_path,
            "bytes_written": content.len(),
            "success": true,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// modal_list_sandboxes
// ============================================================================

/// List the session's sandboxes.
pub struct ModalListSandboxesTool;

#[async_trait]
impl Tool for ModalListSandboxesTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            (
                "Listing Modal sandboxes",
                "Listed Modal sandboxes",
                "Could not list Modal sandboxes",
            ),
            (
                "Перелічую пісочниці Modal",
                "Перелічив пісочниці Modal",
                "Не вдалося перелічити пісочниці Modal",
            ),
            &[],
        ))
    }

    fn name(&self) -> &str {
        "modal_list_sandboxes"
    }

    fn description(&self) -> &str {
        "List Modal sandboxes created in this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_requires_secrets(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let states = try_tool!(list_sandbox_states(context).await);
        ToolExecutionResult::success(json!({
            "sandboxes": states.iter().map(|s| json!({
                "sandbox_id": s.sandbox_id,
                "title": s.title,
                "runtime": s.runtime,
                "image": s.image,
                "workspace_path": s.workspace_path,
                "started_at": s.started_at,
                "timeout_seconds": s.timeout_seconds,
                "exposed_ports": s.exposed_ports,
            })).collect::<Vec<_>>(),
            "count": states.len(),
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// modal_manage_sandbox
// ============================================================================

/// Check on or terminate a sandbox.
pub struct ModalManageSandboxTool;

#[async_trait]
impl Tool for ModalManageSandboxTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        let (english, ukrainian) = match call.arguments.get("action").and_then(Value::as_str) {
            Some("terminate") => (
                (
                    "Terminating Modal sandbox",
                    "Terminated Modal sandbox",
                    "Could not terminate Modal sandbox",
                ),
                (
                    "Зупиняю пісочницю Modal",
                    "Зупинив пісочницю Modal",
                    "Не вдалося зупинити пісочницю Modal",
                ),
            ),
            _ => (
                (
                    "Checking Modal sandbox",
                    "Checked Modal sandbox",
                    "Could not check Modal sandbox",
                ),
                (
                    "Перевіряю пісочницю Modal",
                    "Перевірив пісочницю Modal",
                    "Не вдалося перевірити пісочницю Modal",
                ),
            ),
        };
        Some(narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            english,
            ukrainian,
            &["sandbox_id"],
        ))
    }

    fn name(&self) -> &str {
        "modal_manage_sandbox"
    }

    fn description(&self) -> &str {
        "Check a Modal sandbox's status, or terminate it. Terminate sandboxes when done to stop charges."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sandbox_id": {"type": "string"},
                "action": {"type": "string", "enum": ["status", "terminate"]}
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = try_tool!(required_str(&arguments, "sandbox_id"));
        let action = try_tool!(required_str(&arguments, "action"));
        if !matches!(action, "status" | "terminate") {
            return ToolExecutionResult::tool_error(
                "Invalid action. Expected one of: status, terminate.",
            );
        }
        let credentials = try_tool!(get_credentials(context).await);
        let state = try_tool!(get_sandbox_state(context, sandbox_id).await);
        let client = try_tool!(client_for(credentials, context));
        match action {
            "status" => match client.sandbox_status(sandbox_id).await {
                Ok(SandboxStatus::Running) => {
                    if let Err(err) = touch_sandbox_lease(context, &state).await {
                        return err;
                    }
                    ToolExecutionResult::success(json!({
                        "sandbox_id": sandbox_id,
                        "status": "running",
                        "runtime": state.runtime,
                        "started_at": state.started_at,
                        "timeout_seconds": state.timeout_seconds,
                    }))
                }
                Ok(SandboxStatus::Finished { status, exception }) => {
                    ToolExecutionResult::success(json!({
                        "sandbox_id": sandbox_id,
                        "status": "finished",
                        "result": status,
                        "exception": exception,
                    }))
                }
                Err(err) => ToolExecutionResult::tool_error(err),
            },
            _ => {
                if let Err(err) = client.terminate(sandbox_id).await {
                    return ToolExecutionResult::tool_error(err);
                }
                try_tool!(delete_sandbox_state(context, sandbox_id).await);
                try_tool!(release_sandbox_lease(context, sandbox_id).await);
                ToolExecutionResult::success(json!({
                    "sandbox_id": sandbox_id,
                    "action": "terminate",
                    "success": true,
                }))
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// modal_snapshot_sandbox
// ============================================================================

/// Snapshot a sandbox filesystem into a reusable image.
pub struct ModalSnapshotSandboxTool;

#[async_trait]
impl Tool for ModalSnapshotSandboxTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            (
                "Snapshotting Modal sandbox",
                "Snapshotted Modal sandbox",
                "Could not snapshot Modal sandbox",
            ),
            (
                "Роблю знімок пісочниці Modal",
                "Зробив знімок пісочниці Modal",
                "Не вдалося зробити знімок пісочниці Modal",
            ),
            &["sandbox_id"],
        ))
    }

    fn name(&self) -> &str {
        "modal_snapshot_sandbox"
    }

    fn description(&self) -> &str {
        "Snapshot a Modal sandbox's filesystem into an image (im-...). Pass it as snapshot_image_id to modal_create_sandbox to boot copies of this state. The sandbox keeps running."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"sandbox_id": {"type": "string"}},
            "required": ["sandbox_id"],
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = try_tool!(required_str(&arguments, "sandbox_id"));
        let credentials = try_tool!(get_credentials(context).await);
        let state = try_tool!(get_sandbox_state(context, sandbox_id).await);
        let client = try_tool!(client_for(credentials, context));
        let image_id = match client.snapshot_filesystem(sandbox_id).await {
            Ok(id) => id,
            Err(err) => return explain_failure(&client, &state, err).await,
        };
        if let Err(err) = touch_sandbox_lease(context, &state).await {
            return err;
        }
        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "snapshot_image_id": image_id,
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// modal_tunnel_urls
// ============================================================================

/// Public URLs for a sandbox's exposed ports.
pub struct ModalTunnelUrlsTool;

#[async_trait]
impl Tool for ModalTunnelUrlsTool {
    fn narrate(
        &self,
        call: &ToolCall,
        phase: ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(narrate_labeled_action(
            &call.arguments,
            phase,
            locale,
            (
                "Getting Modal tunnel URLs",
                "Got Modal tunnel URLs",
                "Could not get Modal tunnel URLs",
            ),
            (
                "Отримую адреси тунелів Modal",
                "Отримав адреси тунелів Modal",
                "Не вдалося отримати адреси тунелів Modal",
            ),
            &["sandbox_id"],
        ))
    }

    fn name(&self) -> &str {
        "modal_tunnel_urls"
    }

    fn description(&self) -> &str {
        "Get the public HTTPS URLs for ports exposed with expose_ports when the sandbox was created. Start a server listening on 0.0.0.0 at that port first."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {"sandbox_id": {"type": "string"}},
            "required": ["sandbox_id"],
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
        context_required(self.name())
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let sandbox_id = try_tool!(required_str(&arguments, "sandbox_id"));
        let credentials = try_tool!(get_credentials(context).await);
        let state = try_tool!(get_sandbox_state(context, sandbox_id).await);
        if state.exposed_ports.is_empty() {
            return ToolExecutionResult::tool_error(
                "This sandbox exposes no ports. Create a sandbox with expose_ports to get public URLs.",
            );
        }
        let client = try_tool!(client_for(credentials, context));
        match client.tunnels(sandbox_id).await {
            Ok(tunnels) => ToolExecutionResult::success(json!({
                "sandbox_id": sandbox_id,
                "tunnels": tunnels.iter().map(|t| json!({"port": t.container_port, "url": t.url})).collect::<Vec<_>>(),
            })),
            Err(err) => explain_failure(&client, &state, err).await,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_tags_are_validated() {
        for ok in [
            "python:3.13-slim",
            "ghcr.io/org/img:tag",
            "ubuntu@sha256:abc123",
        ] {
            assert!(validate_image_tag(ok).is_ok(), "{ok}");
        }
        for bad in ["", "python 3", "img;rm -rf /", "img\nRUN x", "$(id)"] {
            assert!(validate_image_tag(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn image_ids_are_validated() {
        assert!(validate_image_id("im-giF8mhSgJpgdTBrBkEzmOq").is_ok());
        for bad in ["", "im-", "sb-abc", "im-a/b"] {
            assert!(validate_image_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn relative_paths_resolve_against_the_workspace() {
        let state = ModalSandboxState {
            sandbox_id: "sb-1".into(),
            task_id: "ta-1".into(),
            app_id: "ap-1".into(),
            runtime: "vm".into(),
            image: "x".into(),
            workspace_path: "/workspace/".into(),
            started_at: String::new(),
            timeout_seconds: 60,
            title: None,
            exposed_ports: vec![],
        };
        assert_eq!(resolve_path(&state, "a/b.txt"), "/workspace/a/b.txt");
        assert_eq!(resolve_path(&state, "/etc/hosts"), "/etc/hosts");
    }

    #[test]
    fn optional_u32_enforces_bounds() {
        let args = json!({"t": 30, "u": "x", "n": null});
        assert!(optional_u32(&args, "t", 60, 100).is_err());
        assert!(optional_u32(&args, "u", 1, 100).is_err());
        assert_eq!(optional_u32(&args, "n", 1, 100).unwrap(), None);
        assert_eq!(optional_u32(&args, "missing", 1, 100).unwrap(), None);
        assert_eq!(optional_u32(&args, "t", 1, 100).unwrap(), Some(30));
    }
}
