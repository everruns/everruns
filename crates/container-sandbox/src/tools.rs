//! Tool implementations for container sandbox operations.
//!
//! Decision: One sandbox per session (container name derived from session ID).
//! Decision: All tools require context (requires_context = true).
//! Decision: Uses tool_output_sanitizer for exec output (same pipeline as Daytona).

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use everruns_core::ToolHints;
use everruns_core::tool_output_sanitizer::{
    clean_exec_output, output_verbosity_budget, priority_aware_truncate,
};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};
use tracing::{debug, error};

use crate::client::{ContainerCreateConfig, DockerClient, HostConfig};
use crate::config::ContainerSandboxConfig;
use crate::state::{
    SandboxState, container_name, delete_sandbox_state, get_sandbox_state, network_name,
    release_sandbox_lease, sandbox_labels, save_sandbox_state, touch_sandbox_lease,
};

/// Helper: extract a required string parameter.
fn required_str(args: &Value, name: &str) -> Result<String, ToolExecutionResult> {
    args.get(name)
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {name}"))
        })
}

/// Helper: create a DockerClient from config.
fn docker_client(config: &ContainerSandboxConfig) -> DockerClient {
    DockerClient::new(config.docker_host.as_deref())
}

// ============================================================================
// sandbox_create
// ============================================================================

pub struct SandboxCreateTool;

#[async_trait]
impl Tool for SandboxCreateTool {
    fn name(&self) -> &str {
        "sandbox_create"
    }

    fn description(&self) -> &str {
        "Create and start a container sandbox for code execution. \
         One sandbox per session. Returns the sandbox container name."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "image": {
                    "type": "string",
                    "description": "Container image (default: ubuntu:24.04)"
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_long_running(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "Tool requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let config = ContainerSandboxConfig::default();
        let image = arguments
            .get("image")
            .and_then(|v| v.as_str())
            .unwrap_or(&config.image)
            .to_string();

        let session_id = context.session_id.to_string();
        let c_name = container_name(&session_id);
        let n_name = network_name(&session_id);
        let labels = sandbox_labels(&session_id);
        let client = docker_client(&config);

        // Create network (track whether we created it vs reused an existing one)
        let (network_id, network_created) = match client
            .create_network(&n_name, labels.clone())
            .await
        {
            Ok(id) => (id, true),
            Err(e) if e.contains("already exists") || e.contains("Conflict") => {
                // Network exists, that's fine — reuse
                debug!(name = %n_name, "Network already exists, reusing");
                (n_name.clone(), false)
            }
            Err(e) => {
                return ToolExecutionResult::tool_error(format!("Failed to create network: {e}"));
            }
        };

        // Create container
        let create_config = ContainerCreateConfig {
            image: image.clone(),
            cmd: vec![
                "tail".to_string(),
                "-f".to_string(),
                "/dev/null".to_string(),
            ],
            working_dir: config.working_dir.clone(),
            labels,
            host_config: HostConfig {
                runtime: config.runtime.clone(),
                network_mode: n_name.clone(),
                memory: config.memory_limit,
                nano_cpus: config.cpu_limit,
                pids_limit: config.pids_limit,
                init: true,
            },
        };

        let container = match client.create_container(&c_name, &create_config).await {
            Ok(c) => c,
            Err(e) => {
                // Only remove network if we just created it
                // TODO: idempotent create (409/name conflict) is a follow-up
                if network_created {
                    let _ = client.remove_network(&network_id).await;
                }
                return ToolExecutionResult::tool_error(format!("Failed to create container: {e}"));
            }
        };

        // Start container
        if let Err(e) = client.start_container(&container.id).await {
            let _ = client.remove_container(&container.id).await;
            if network_created {
                let _ = client.remove_network(&network_id).await;
            }
            return ToolExecutionResult::tool_error(format!("Failed to start container: {e}"));
        }

        let now = chrono::Utc::now().to_rfc3339();
        let state = SandboxState {
            container_id: container.id.clone(),
            network_id: network_id.clone(),
            container_name: c_name.clone(),
            network_name: n_name,
            image: image.clone(),
            working_dir: config.working_dir.clone(),
            started_at: now,
        };

        // Save state and register lease
        if let Err(e) = save_sandbox_state(context, &state).await {
            return e;
        }
        if let Err(e) = touch_sandbox_lease(context, &state).await {
            return e;
        }

        ToolExecutionResult::success(format!(
            "Sandbox created and running.\n\
             Container: {c_name}\n\
             Image: {image}\n\
             Working directory: {}",
            config.working_dir
        ))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// sandbox_exec
// ============================================================================

pub struct SandboxExecTool;

#[async_trait]
impl Tool for SandboxExecTool {
    fn name(&self) -> &str {
        "sandbox_exec"
    }

    fn description(&self) -> &str {
        "Execute a command in the container sandbox. Returns stdout/stderr and exit code."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory (default: /workspace)"
                },
                "output": everruns_core::tool_output_sanitizer::output_verbosity_schema()
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_long_running(true)
            .with_persist_output(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "Tool requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let command = match required_str(&arguments, "command") {
            Ok(c) => c,
            Err(e) => return e,
        };

        let output_mode = arguments
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("concise");

        let state = match get_sandbox_state(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let working_dir = arguments
            .get("working_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(&state.working_dir);

        let config = ContainerSandboxConfig::default();
        let client = docker_client(&config);

        let result = match client
            .exec(
                &state.container_id,
                &["sh", "-c", &command],
                Some(working_dir),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => return ToolExecutionResult::tool_error(format!("Exec failed: {e}")),
        };

        // Refresh lease on successful exec
        let _ = touch_sandbox_lease(context, &state).await;

        // Sanitize output
        let clean_output = clean_exec_output(&result.output);
        let output = if let Some(budget) = output_verbosity_budget(output_mode) {
            priority_aware_truncate(&clean_output, budget)
        } else {
            clean_output
        };

        let exit_code = result.exit_code;
        if exit_code == 0 {
            ToolExecutionResult::success(output)
        } else {
            ToolExecutionResult::success(format!("Exit code: {exit_code}\n\n{output}"))
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// sandbox_read_file
// ============================================================================

pub struct SandboxReadFileTool;

#[async_trait]
impl Tool for SandboxReadFileTool {
    fn name(&self) -> &str {
        "sandbox_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from the container sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file inside the container"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Tool requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = match required_str(&arguments, "path") {
            Ok(p) => p,
            Err(e) => return e,
        };

        let state = match get_sandbox_state(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let config = ContainerSandboxConfig::default();
        let client = docker_client(&config);

        match client.get_archive(&state.container_id, &path).await {
            Ok(bytes) => {
                // Try to return as text, fall back to base64
                match String::from_utf8(bytes.clone()) {
                    Ok(text) => ToolExecutionResult::success(text),
                    Err(_) => ToolExecutionResult::success(format!(
                        "[Binary file, {} bytes]",
                        bytes.len()
                    )),
                }
            }
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to read file: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// sandbox_write_file
// ============================================================================

pub struct SandboxWriteFileTool;

#[async_trait]
impl Tool for SandboxWriteFileTool {
    fn name(&self) -> &str {
        "sandbox_write_file"
    }

    fn description(&self) -> &str {
        "Write a file to the container sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path for the file inside the container"
                },
                "content": {
                    "type": "string",
                    "description": "File content to write"
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Tool requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = match required_str(&arguments, "path") {
            Ok(p) => p,
            Err(e) => return e,
        };
        let content = match required_str(&arguments, "content") {
            Ok(c) => c,
            Err(e) => return e,
        };

        let state = match get_sandbox_state(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let config = ContainerSandboxConfig::default();
        let client = docker_client(&config);

        // Split path into directory and filename
        let (dir, filename) = match path.rsplit_once('/') {
            Some((d, f)) if !d.is_empty() => (d.to_string(), f.to_string()),
            _ => ("/".to_string(), path.trim_start_matches('/').to_string()),
        };

        if filename.is_empty() {
            return ToolExecutionResult::tool_error(
                "Invalid path: filename is empty (trailing slash?)",
            );
        }

        match client
            .put_archive(&state.container_id, &dir, &filename, content.as_bytes())
            .await
        {
            Ok(()) => {
                ToolExecutionResult::success(format!("Written {path} ({} bytes)", content.len()))
            }
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to write file: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// sandbox_list
// ============================================================================

pub struct SandboxListTool;

#[async_trait]
impl Tool for SandboxListTool {
    fn name(&self) -> &str {
        "sandbox_list"
    }

    fn description(&self) -> &str {
        "List container sandboxes for this session."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Tool requires context.")
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let session_id = context.session_id.to_string();
        let config = ContainerSandboxConfig::default();
        let client = docker_client(&config);

        let filters = [("managed-by", "everruns"), ("session", &session_id)];

        match client.list_containers(&filters).await {
            Ok(containers) => {
                if containers.is_empty() {
                    return ToolExecutionResult::success("No sandboxes found for this session.");
                }
                let summary: Vec<String> = containers
                    .iter()
                    .map(|c| {
                        let name = c
                            .names
                            .first()
                            .map(|n| n.trim_start_matches('/'))
                            .unwrap_or("unknown");
                        format!("- {} ({}): {}", name, c.state, c.status)
                    })
                    .collect();
                ToolExecutionResult::success(summary.join("\n"))
            }
            Err(e) => ToolExecutionResult::tool_error(format!("Failed to list containers: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// sandbox_manage
// ============================================================================

pub struct SandboxManageTool;

#[async_trait]
impl Tool for SandboxManageTool {
    fn name(&self) -> &str {
        "sandbox_manage"
    }

    fn description(&self) -> &str {
        "Manage the container sandbox: stop, start, or remove it."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform",
                    "enum": ["stop", "start", "remove"]
                }
            },
            "required": ["action"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Tool requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let action = match required_str(&arguments, "action") {
            Ok(a) => a,
            Err(e) => return e,
        };

        let state = match get_sandbox_state(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let config = ContainerSandboxConfig::default();
        let client = docker_client(&config);

        match action.as_str() {
            "stop" => {
                if let Err(e) = client.stop_container(&state.container_id, 10).await {
                    return ToolExecutionResult::tool_error(format!("Failed to stop: {e}"));
                }
                ToolExecutionResult::success("Sandbox stopped.")
            }
            "start" => {
                if let Err(e) = client.start_container(&state.container_id).await {
                    return ToolExecutionResult::tool_error(format!("Failed to start: {e}"));
                }
                let _ = touch_sandbox_lease(context, &state).await;
                ToolExecutionResult::success("Sandbox started.")
            }
            "remove" => {
                // Stop, remove container, remove network, clean up state
                let _ = client.stop_container(&state.container_id, 5).await;
                if let Err(e) = client.remove_container(&state.container_id).await {
                    error!("Failed to remove container: {e}");
                }
                let _ = client.remove_network(&state.network_id).await;
                let _ = release_sandbox_lease(context, &state.container_id).await;
                let _ = delete_sandbox_state(context, &state.container_name).await;
                ToolExecutionResult::success("Sandbox removed and cleaned up.")
            }
            _ => ToolExecutionResult::tool_error("Invalid action. Use: stop, start, or remove"),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// sandbox_upload
// ============================================================================

pub struct SandboxUploadTool;

#[async_trait]
impl Tool for SandboxUploadTool {
    fn name(&self) -> &str {
        "sandbox_upload"
    }

    fn description(&self) -> &str {
        "Upload a file from session storage to the container sandbox."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "session_path": {
                    "type": "string",
                    "description": "Path of the file in session storage"
                },
                "container_path": {
                    "type": "string",
                    "description": "Absolute destination path inside the container"
                }
            },
            "required": ["session_path", "container_path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Tool requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let session_path = match required_str(&arguments, "session_path") {
            Ok(p) => p,
            Err(e) => return e,
        };
        let container_path = match required_str(&arguments, "container_path") {
            Ok(p) => p,
            Err(e) => return e,
        };

        let state = match get_sandbox_state(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let file_store = match context.file_store.as_ref() {
            Some(fs) => fs,
            None => return ToolExecutionResult::tool_error("File store not available"),
        };

        // Read file from session VFS
        let file = match file_store
            .read_file(context.session_id, &session_path)
            .await
        {
            Ok(Some(f)) => f,
            Ok(None) => {
                return ToolExecutionResult::tool_error(format!(
                    "File not found in session storage: {session_path}"
                ));
            }
            Err(e) => {
                return ToolExecutionResult::tool_error(format!(
                    "Failed to read from session storage: {e}"
                ));
            }
        };

        // Decode content (text or base64)
        let content_str = file.content.unwrap_or_default();
        let bytes = if file.encoding == "base64" {
            match BASE64.decode(&content_str) {
                Ok(b) => b,
                Err(e) => {
                    return ToolExecutionResult::tool_error(format!(
                        "Failed to decode base64 content from session file {session_path}: {e}"
                    ));
                }
            }
        } else {
            content_str.as_bytes().to_vec()
        };

        let config = ContainerSandboxConfig::default();
        let client = docker_client(&config);

        let (dir, filename) = match container_path.rsplit_once('/') {
            Some((d, f)) if !d.is_empty() => (d.to_string(), f.to_string()),
            _ => (
                "/".to_string(),
                container_path.trim_start_matches('/').to_string(),
            ),
        };

        if filename.is_empty() {
            return ToolExecutionResult::tool_error(
                "Invalid container_path: filename is empty (trailing slash?)",
            );
        }

        match client
            .put_archive(&state.container_id, &dir, &filename, &bytes)
            .await
        {
            Ok(()) => ToolExecutionResult::success(format!(
                "Uploaded {session_path} → {container_path} ({} bytes)",
                bytes.len()
            )),
            Err(e) => ToolExecutionResult::tool_error(format!("Upload failed: {e}")),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// sandbox_download
// ============================================================================

pub struct SandboxDownloadTool;

#[async_trait]
impl Tool for SandboxDownloadTool {
    fn name(&self) -> &str {
        "sandbox_download"
    }

    fn description(&self) -> &str {
        "Download a file from the container sandbox to session storage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "container_path": {
                    "type": "string",
                    "description": "Absolute path of the file inside the container"
                },
                "session_path": {
                    "type": "string",
                    "description": "Destination path in session storage"
                }
            },
            "required": ["container_path", "session_path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error("Tool requires context.")
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let container_path = match required_str(&arguments, "container_path") {
            Ok(p) => p,
            Err(e) => return e,
        };
        let session_path = match required_str(&arguments, "session_path") {
            Ok(p) => p,
            Err(e) => return e,
        };

        let state = match get_sandbox_state(context).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let file_store = match context.file_store.as_ref() {
            Some(fs) => fs,
            None => return ToolExecutionResult::tool_error("File store not available"),
        };

        let config = ContainerSandboxConfig::default();
        let client = docker_client(&config);

        let bytes = match client
            .get_archive(&state.container_id, &container_path)
            .await
        {
            Ok(b) => b,
            Err(e) => {
                return ToolExecutionResult::tool_error(format!("Failed to read file: {e}"));
            }
        };

        // Store as text if valid UTF-8, otherwise base64
        let (content, encoding) = match String::from_utf8(bytes.clone()) {
            Ok(text) => (text, "text"),
            Err(_) => (BASE64.encode(&bytes), "base64"),
        };

        match file_store
            .write_file(context.session_id, &session_path, &content, encoding)
            .await
        {
            Ok(_) => ToolExecutionResult::success(format!(
                "Downloaded {container_path} → {session_path} ({} bytes)",
                bytes.len()
            )),
            Err(e) => {
                ToolExecutionResult::tool_error(format!("Failed to save to session storage: {e}"))
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}
