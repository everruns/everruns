//! Docker Container Integration (Experimental)
//!
//! This capability provides tools for running and interacting with a Docker container
//! tied to the session lifecycle. The container is lazily started on first tool use
//! and persists for the duration of the session.
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: Experimental-only (gated behind DeploymentGrade::Dev)
//! Decision: Single container per session, named everruns-{session_id}
//! Decision: Lazy start on first tool use, host networking
//!
//! Configuration (via AgentCapabilityConfig.config):
//! ```json
//! {
//!   "image": "mcr.microsoft.com/devcontainers/python:3.11",
//!   "working_dir": "/workspace"  // optional, defaults to /workspace
//! }
//! ```

use everruns_core::ToolHints;
use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin, RiskLevel};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::process::Stdio;
use std::sync::LazyLock;
use tokio::process::Command;
use tracing::{debug, error, info, warn};

// ============================================================================
// Integration Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: true,
        factory: || Box::new(DockerContainerCapability),
    }
}

// ============================================================================
// Constants
// ============================================================================

/// Default Docker image if none specified in config
const DEFAULT_IMAGE: &str = "mcr.microsoft.com/devcontainers/python:3.11";

/// Default working directory inside the container
const DEFAULT_WORKING_DIR: &str = "/workspace";

/// Container name prefix
const CONTAINER_PREFIX: &str = "everruns";

// ============================================================================
// Configuration
// ============================================================================

/// Configuration schema for the Docker Container capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContainerConfig {
    /// Docker image to use (e.g., "mcr.microsoft.com/devcontainers/python:3.11")
    #[serde(default = "default_image")]
    pub image: String,

    /// Working directory inside the container
    #[serde(default = "default_working_dir")]
    pub working_dir: String,
}

fn default_image() -> String {
    DEFAULT_IMAGE.to_string()
}

fn default_working_dir() -> String {
    DEFAULT_WORKING_DIR.to_string()
}

impl Default for DockerContainerConfig {
    fn default() -> Self {
        Self {
            image: default_image(),
            working_dir: default_working_dir(),
        }
    }
}

// ============================================================================
// DockerContainerCapability
// ============================================================================

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        r#"## Docker Container (Experimental)

You have access to a Docker container for executing commands and managing files.
The container is tied to this session and persists across tool calls.

IMPORTANT: This is an EXPERIMENTAL capability. The container uses host networking.

Tools:
- `docker_exec` - Execute a shell command inside the container. Returns stdout, stderr, and exit code.
- `docker_read_file` - Read a file from the container filesystem.
- `docker_write_file` - Write content to a file in the container filesystem.
- `docker_logs` - Get logs from the container. Useful for debugging long-running processes.
- `docker_stop` - Stop and remove the container (for cleanup or to reset state).

The container is lazily started on first tool use. Subsequent calls reuse the same container.

Best practices:
- Use `docker_exec` with `bash -c "..."` for complex commands
- Check exit codes to verify command success
- The working directory defaults to /workspace
- Files written persist for the session duration
- Use `docker_stop` to clean up when done or to reset container state"#,
    );
    prompt.push_str(everruns_core::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

pub struct DockerContainerCapability;

impl Capability for DockerContainerCapability {
    fn id(&self) -> &str {
        "docker_container"
    }

    fn name(&self) -> &str {
        "[Experimental] Docker Container"
    }

    fn description(&self) -> &str {
        "Run commands and manage files in a Docker container tied to the session. \
         Container is lazily started on first use and persists for the session duration. \
         EXPERIMENTAL: This capability may change significantly."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("container")
    }

    fn category(&self) -> Option<&str> {
        Some("Development")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(DockerExecTool),
            Box::new(DockerReadFileTool),
            Box::new(DockerWriteFileTool),
            Box::new(DockerLogsTool),
            Box::new(DockerStopTool),
        ]
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Generate container name from session ID
fn container_name(session_id: &everruns_core::typed_id::SessionId) -> String {
    format!("{}-{}", CONTAINER_PREFIX, session_id.uuid())
}

/// Check if Docker is available on the system
async fn is_docker_available() -> bool {
    match Command::new("docker")
        .arg("version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
    {
        Ok(status) => status.success(),
        Err(e) => {
            debug!("Docker not available: {}", e);
            false
        }
    }
}

/// Check if a container exists and is running
async fn is_container_running(name: &str) -> bool {
    match Command::new("docker")
        .args(["inspect", "-f", "{{.State.Running}}", name])
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.trim() == "true"
        }
        Err(_) => false,
    }
}

/// Check if a container exists (running or stopped)
async fn container_exists(name: &str) -> bool {
    Command::new("docker")
        .args(["inspect", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Ensure container is running, starting it if necessary
async fn ensure_container_running(
    name: &str,
    config: &DockerContainerConfig,
) -> Result<(), String> {
    // Check if Docker is available
    if !is_docker_available().await {
        return Err(
            "Docker is not available. Please ensure Docker is installed and running.".to_string(),
        );
    }

    // If container is already running, we're done
    if is_container_running(name).await {
        debug!("Container {} is already running", name);
        return Ok(());
    }

    // If container exists but is stopped, start it
    if container_exists(name).await {
        info!("Starting existing container: {}", name);
        let output = Command::new("docker")
            .args(["start", name])
            .output()
            .await
            .map_err(|e| format!("Failed to start container: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("Failed to start container: {}", stderr));
        }
        return Ok(());
    }

    // Create and start a new container
    info!(
        "Creating new container: {} with image: {}",
        name, config.image
    );

    let output = Command::new("docker")
        .args([
            "run",
            "-d", // Detached mode
            "--name",
            name, // Container name
            "--network",
            "host", // Host networking
            "-w",
            &config.working_dir, // Working directory
            "--init",            // Use init process
            &config.image,       // Image
            "tail",
            "-f",
            "/dev/null", // Keep container running
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to create container: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Failed to create container {}: {}", name, stderr);
        return Err(format!("Failed to create container: {}", stderr));
    }

    info!("Container {} created and running", name);
    Ok(())
}

/// Parse capability config from JSON value
fn parse_config(config: &Value) -> DockerContainerConfig {
    serde_json::from_value(config.clone()).unwrap_or_default()
}

// ============================================================================
// DockerExecTool
// ============================================================================

pub struct DockerExecTool;

#[async_trait]
impl Tool for DockerExecTool {
    fn name(&self) -> &str {
        "docker_exec"
    }

    fn description(&self) -> &str {
        "Execute a command inside the Docker container. Returns stdout, stderr, and exit code. \
         The container is automatically started if not already running."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute (e.g., 'ls -la' or 'python script.py')"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Working directory for the command (optional, defaults to container's working dir)"
                },
                "config": {
                    "type": "object",
                    "description": "Container configuration (image, working_dir). Usually provided by capability config.",
                    "properties": {
                        "image": { "type": "string" },
                        "working_dir": { "type": "string" }
                    }
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
            "docker_exec requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let command = match arguments.get("command").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolExecutionResult::tool_error("Missing required parameter: command"),
        };

        let config = arguments
            .get("config")
            .map(parse_config)
            .unwrap_or_default();

        let working_dir = arguments
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(String::from);
        let output_mode = arguments
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("concise");

        let name = container_name(&context.session_id);

        // Ensure container is running
        if let Err(e) = ensure_container_running(&name, &config).await {
            return ToolExecutionResult::tool_error(e);
        }

        // Build exec command
        let mut args = vec!["exec".to_string()];

        if let Some(ref wd) = working_dir {
            args.push("-w".to_string());
            args.push(wd.clone());
        }

        args.push(name.clone());
        args.push("sh".to_string());
        args.push("-c".to_string());
        args.push(command.to_string());

        debug!("Executing in container {}: {}", name, command);

        // Execute command
        let output = match Command::new("docker").args(&args).output().await {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to execute command in container: {}", e);
                return ToolExecutionResult::internal_error_msg(format!(
                    "Failed to execute command: {}",
                    e
                ));
            }
        };

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout_raw = String::from_utf8_lossy(&output.stdout);
        let stderr_raw = String::from_utf8_lossy(&output.stderr);

        use everruns_core::tool_output_sanitizer::{
            clean_exec_output, output_verbosity_budget, priority_aware_truncate,
        };
        let clean_stdout = clean_exec_output(&stdout_raw);
        let clean_stderr = clean_exec_output(&stderr_raw);
        let (stdout, stderr) = if let Some(budget) = output_verbosity_budget(output_mode) {
            (
                priority_aware_truncate(&clean_stdout, budget),
                priority_aware_truncate(&clean_stderr, budget.min(4096)),
            )
        } else {
            (clean_stdout.clone(), clean_stderr.clone())
        };
        let mut raw = clean_stdout;
        if !clean_stderr.is_empty() {
            raw.push_str("\n--- stderr ---\n");
            raw.push_str(&clean_stderr);
        }

        ToolExecutionResult::success_with_raw_output(
            json!({
                "stdout": stdout,
                "stderr": stderr,
                "exit_code": exit_code,
                "success": exit_code == 0
            }),
            raw,
        )
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DockerReadFileTool
// ============================================================================

pub struct DockerReadFileTool;

#[async_trait]
impl Tool for DockerReadFileTool {
    fn name(&self) -> &str {
        "docker_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from the Docker container filesystem. \
         The container is automatically started if not already running."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file inside the container (e.g., '/workspace/main.py')"
                },
                "config": {
                    "type": "object",
                    "description": "Container configuration (image, working_dir). Usually provided by capability config.",
                    "properties": {
                        "image": { "type": "string" },
                        "working_dir": { "type": "string" }
                    }
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "docker_read_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = match arguments.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: path"),
        };

        let config = arguments
            .get("config")
            .map(parse_config)
            .unwrap_or_default();

        let name = container_name(&context.session_id);

        // Ensure container is running
        if let Err(e) = ensure_container_running(&name, &config).await {
            return ToolExecutionResult::tool_error(e);
        }

        debug!("Reading file from container {}: {}", name, path);

        // Use docker exec to cat the file
        let output = match Command::new("docker")
            .args(["exec", &name, "cat", path])
            .output()
            .await
        {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to read file from container: {}", e);
                return ToolExecutionResult::internal_error_msg(format!(
                    "Failed to read file: {}",
                    e
                ));
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return ToolExecutionResult::tool_error(format!("Failed to read file: {}", stderr));
        }

        let content = String::from_utf8_lossy(&output.stdout).to_string();

        ToolExecutionResult::success(json!({
            "path": path,
            "content": content,
            "size_bytes": content.len()
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DockerWriteFileTool
// ============================================================================

pub struct DockerWriteFileTool;

#[async_trait]
impl Tool for DockerWriteFileTool {
    fn name(&self) -> &str {
        "docker_write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file in the Docker container filesystem. \
         Parent directories are created automatically. \
         The container is automatically started if not already running."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path for the file inside the container (e.g., '/workspace/main.py')"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                },
                "config": {
                    "type": "object",
                    "description": "Container configuration (image, working_dir). Usually provided by capability config.",
                    "properties": {
                        "image": { "type": "string" },
                        "working_dir": { "type": "string" }
                    }
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default().with_open_world(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "docker_write_file requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = match arguments.get("path").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: path"),
        };

        let content = match arguments.get("content").and_then(|v| v.as_str()) {
            Some(c) => c,
            None => return ToolExecutionResult::tool_error("Missing required parameter: content"),
        };

        let config = arguments
            .get("config")
            .map(parse_config)
            .unwrap_or_default();

        let name = container_name(&context.session_id);

        // Ensure container is running
        if let Err(e) = ensure_container_running(&name, &config).await {
            return ToolExecutionResult::tool_error(e);
        }

        debug!("Writing file to container {}: {}", name, path);

        // Get parent directory
        let parent_dir = std::path::Path::new(path)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "/".to_string());

        // Create parent directories if needed
        let mkdir_output = Command::new("docker")
            .args(["exec", &name, "mkdir", "-p", &parent_dir])
            .output()
            .await;

        if let Err(e) = mkdir_output {
            warn!("Failed to create parent directory: {}", e);
        }

        // Write content using docker exec with base64 encoding to handle special characters
        let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, content);

        let output = match Command::new("docker")
            .args([
                "exec",
                &name,
                "sh",
                "-c",
                &format!("echo '{}' | base64 -d > '{}'", encoded, path),
            ])
            .output()
            .await
        {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to write file to container: {}", e);
                return ToolExecutionResult::internal_error_msg(format!(
                    "Failed to write file: {}",
                    e
                ));
            }
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return ToolExecutionResult::tool_error(format!("Failed to write file: {}", stderr));
        }

        ToolExecutionResult::success(json!({
            "path": path,
            "size_bytes": content.len(),
            "success": true
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DockerStopTool
// ============================================================================

pub struct DockerStopTool;

#[async_trait]
impl Tool for DockerStopTool {
    fn name(&self) -> &str {
        "docker_stop"
    }

    fn description(&self) -> &str {
        "Stop and remove the Docker container associated with this session. \
         Use this to clean up resources or reset the container state. \
         A new container will be created on the next docker_exec/read/write call."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "force": {
                    "type": "boolean",
                    "description": "Force stop (kill) the container if it doesn't stop gracefully (default: false)"
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_open_world(true)
            .with_destructive(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "docker_stop requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let force = arguments
            .get("force")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let name = container_name(&context.session_id);

        // Check if container exists
        if !container_exists(&name).await {
            return ToolExecutionResult::success(json!({
                "stopped": false,
                "removed": false,
                "message": "Container does not exist",
                "container_name": name
            }));
        }

        debug!("Stopping container: {}", name);

        // Stop the container
        let stop_args = if force {
            vec!["kill", &name]
        } else {
            vec!["stop", &name]
        };

        let stop_output = match Command::new("docker").args(&stop_args).output().await {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to stop container: {}", e);
                return ToolExecutionResult::internal_error_msg(format!(
                    "Failed to stop container: {}",
                    e
                ));
            }
        };

        let stopped = stop_output.status.success();
        if !stopped {
            let stderr = String::from_utf8_lossy(&stop_output.stderr);
            warn!("Failed to stop container {}: {}", name, stderr);
        }

        // Remove the container
        debug!("Removing container: {}", name);

        let rm_output = match Command::new("docker")
            .args(["rm", "-f", &name])
            .output()
            .await
        {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to remove container: {}", e);
                return ToolExecutionResult::internal_error_msg(format!(
                    "Failed to remove container: {}",
                    e
                ));
            }
        };

        let removed = rm_output.status.success();
        if !removed {
            let stderr = String::from_utf8_lossy(&rm_output.stderr);
            warn!("Failed to remove container {}: {}", name, stderr);
        }

        info!("Container {} stopped and removed", name);

        ToolExecutionResult::success(json!({
            "stopped": stopped,
            "removed": removed,
            "container_name": name,
            "message": if stopped && removed {
                "Container stopped and removed successfully"
            } else if removed {
                "Container removed (was not running)"
            } else {
                "Failed to fully clean up container"
            }
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DockerLogsTool
// ============================================================================

pub struct DockerLogsTool;

#[async_trait]
impl Tool for DockerLogsTool {
    fn name(&self) -> &str {
        "docker_logs"
    }

    fn description(&self) -> &str {
        "Get logs from the Docker container. Returns stdout/stderr output from the container. \
         Useful for debugging long-running processes or checking application output."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tail": {
                    "type": "integer",
                    "description": "Number of lines to show from the end of the logs (default: 100)"
                },
                "since": {
                    "type": "string",
                    "description": "Show logs since timestamp (e.g., '2024-01-01T00:00:00Z') or relative time (e.g., '10m', '1h')"
                },
                "timestamps": {
                    "type": "boolean",
                    "description": "Show timestamps with each log line (default: false)"
                }
            },
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_open_world(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "docker_logs requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let tail = arguments
            .get("tail")
            .and_then(|v| v.as_i64())
            .unwrap_or(100);

        let since = arguments.get("since").and_then(|v| v.as_str());

        let timestamps = arguments
            .get("timestamps")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let name = container_name(&context.session_id);

        // Check if container exists
        if !container_exists(&name).await {
            return ToolExecutionResult::tool_error(format!(
                "Container '{}' does not exist. Use docker_exec to start it first.",
                name
            ));
        }

        debug!("Getting logs from container: {}", name);

        // Build docker logs command
        let mut args = vec!["logs".to_string()];

        args.push("--tail".to_string());
        args.push(tail.to_string());

        if let Some(since_val) = since {
            args.push("--since".to_string());
            args.push(since_val.to_string());
        }

        if timestamps {
            args.push("--timestamps".to_string());
        }

        args.push(name.clone());

        // Execute docker logs
        let output = match Command::new("docker").args(&args).output().await {
            Ok(o) => o,
            Err(e) => {
                error!("Failed to get logs from container: {}", e);
                return ToolExecutionResult::internal_error_msg(format!(
                    "Failed to get logs: {}",
                    e
                ));
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Docker logs command puts container stderr on command stderr,
        // so we combine them for the user
        let combined_logs = if stderr.is_empty() {
            stdout.clone()
        } else if stdout.is_empty() {
            stderr.clone()
        } else {
            format!("{}\n{}", stdout, stderr)
        };

        ToolExecutionResult::success(json!({
            "logs": combined_logs,
            "stdout": stdout,
            "stderr": stderr,
            "container_name": name,
            "lines_requested": tail
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Capability metadata tests ---

    #[test]
    fn test_capability_metadata() {
        let cap = DockerContainerCapability;
        assert_eq!(cap.id(), "docker_container");
        assert_eq!(cap.name(), "[Experimental] Docker Container");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("container"));
        assert_eq!(cap.category(), Some("Development"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = DockerContainerCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 5);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"docker_exec"));
        assert!(names.contains(&"docker_read_file"));
        assert!(names.contains(&"docker_write_file"));
        assert!(names.contains(&"docker_logs"));
        assert!(names.contains(&"docker_stop"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = DockerContainerCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("docker_exec"));
        assert!(prompt.contains("docker_read_file"));
        assert!(prompt.contains("docker_write_file"));
        assert!(prompt.contains("docker_logs"));
        assert!(prompt.contains("docker_stop"));
        assert!(prompt.contains("EXPERIMENTAL"));
    }

    #[test]
    fn test_all_tools_require_context() {
        let cap = DockerContainerCapability;
        for tool in cap.tools() {
            assert!(
                tool.requires_context(),
                "Tool {} should require context",
                tool.name()
            );
        }
    }

    // --- Config tests ---

    #[test]
    fn test_config_default() {
        let config = DockerContainerConfig::default();
        assert_eq!(config.image, DEFAULT_IMAGE);
        assert_eq!(config.working_dir, DEFAULT_WORKING_DIR);
    }

    #[test]
    fn test_config_parse() {
        let json = json!({
            "image": "ubuntu:22.04",
            "working_dir": "/app"
        });
        let config = parse_config(&json);
        assert_eq!(config.image, "ubuntu:22.04");
        assert_eq!(config.working_dir, "/app");
    }

    #[test]
    fn test_config_parse_partial() {
        let json = json!({
            "image": "node:18"
        });
        let config = parse_config(&json);
        assert_eq!(config.image, "node:18");
        assert_eq!(config.working_dir, DEFAULT_WORKING_DIR);
    }

    #[test]
    fn test_container_name() {
        let uuid = uuid::Uuid::parse_str("12345678-1234-1234-1234-123456789012").unwrap();
        let session_id = everruns_core::typed_id::SessionId::from_uuid(uuid);
        let name = container_name(&session_id);
        assert_eq!(name, "everruns-12345678-1234-1234-1234-123456789012");
    }

    // --- Error path tests ---

    #[tokio::test]
    async fn test_docker_exec_without_context() {
        let tool = DockerExecTool;
        let result = tool.execute(json!({"command": "echo hello"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_docker_read_file_without_context() {
        let tool = DockerReadFileTool;
        let result = tool.execute(json!({"path": "/test.txt"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_docker_write_file_without_context() {
        let tool = DockerWriteFileTool;
        let result = tool
            .execute(json!({"path": "/test.txt", "content": "hello"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_docker_stop_without_context() {
        let tool = DockerStopTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_docker_logs_without_context() {
        let tool = DockerLogsTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("requires context"));
            }
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_docker_exec_missing_command() {
        let tool = DockerExecTool;
        let context = ToolContext::new(everruns_core::typed_id::SessionId::from_uuid(
            uuid::Uuid::nil(),
        ));
        let result = tool.execute_with_context(json!({}), &context).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Missing required parameter"));
            }
            _ => panic!("Expected tool error for missing command"),
        }
    }

    #[tokio::test]
    async fn test_docker_read_file_missing_path() {
        let tool = DockerReadFileTool;
        let context = ToolContext::new(everruns_core::typed_id::SessionId::from_uuid(
            uuid::Uuid::nil(),
        ));
        let result = tool.execute_with_context(json!({}), &context).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Missing required parameter"));
            }
            _ => panic!("Expected tool error for missing path"),
        }
    }

    #[tokio::test]
    async fn test_docker_write_file_missing_params() {
        let tool = DockerWriteFileTool;
        let context = ToolContext::new(everruns_core::typed_id::SessionId::from_uuid(
            uuid::Uuid::nil(),
        ));

        // Missing path
        let result = tool
            .execute_with_context(json!({"content": "hello"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Missing required parameter"));
            }
            _ => panic!("Expected tool error for missing path"),
        }

        // Missing content
        let result = tool
            .execute_with_context(json!({"path": "/test.txt"}), &context)
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(msg.contains("Missing required parameter"));
            }
            _ => panic!("Expected tool error for missing content"),
        }
    }
}
