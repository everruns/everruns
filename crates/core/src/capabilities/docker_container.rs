//! Docker Container Capability (Experimental)
//!
//! This capability provides tools for running and interacting with a Docker container
//! tied to the session lifecycle. The container is lazily started on first tool use
//! and persists for the duration of the session.
//!
//! **EXPERIMENTAL**: This capability is experimental and may change significantly.
//!
//! Configuration (via AgentCapabilityConfig.config):
//! ```json
//! {
//!   "image": "mcr.microsoft.com/devcontainers/python:3.11",
//!   "working_dir": "/workspace"  // optional, defaults to /workspace
//! }
//! ```
//!
//! Tools provided:
//! - `docker_exec`: Execute a command inside the container
//! - `docker_read_file`: Read a file from the container
//! - `docker_write_file`: Write a file to the container
//! - `docker_stop`: Stop and remove the container

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, error, info, warn};

/// Default Docker image if none specified in config
const DEFAULT_IMAGE: &str = "mcr.microsoft.com/devcontainers/python:3.11";

/// Default working directory inside the container
const DEFAULT_WORKING_DIR: &str = "/workspace";

/// Container name prefix
const CONTAINER_PREFIX: &str = "everruns";

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

/// Docker Container capability - provides tools to interact with a Docker container
pub struct DockerContainerCapability;

impl Capability for DockerContainerCapability {
    fn id(&self) -> &str {
        CapabilityId::DOCKER_CONTAINER
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

    fn icon(&self) -> Option<&str> {
        Some("container")
    }

    fn category(&self) -> Option<&str> {
        Some("Development")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to a Docker container for executing commands and managing files.
The container is tied to this session and persists across tool calls.

IMPORTANT: This is an EXPERIMENTAL capability. The container uses host networking.

Available tools:
- `docker_exec`: Execute a shell command inside the container. Returns stdout, stderr, and exit code.
- `docker_read_file`: Read a file from the container filesystem.
- `docker_write_file`: Write content to a file in the container filesystem.
- `docker_stop`: Stop and remove the container (for cleanup or to reset state).

The container is lazily started on first tool use. Subsequent calls reuse the same container.

Best practices:
- Use `docker_exec` with `bash -c "..."` for complex commands
- Check exit codes to verify command success
- The working directory defaults to /workspace
- Files written persist for the session duration
- Use `docker_stop` to clean up when done or to reset container state"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(DockerExecTool),
            Box::new(DockerReadFileTool),
            Box::new(DockerWriteFileTool),
            Box::new(DockerStopTool),
        ]
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Generate container name from session ID
fn container_name(session_id: &uuid::Uuid) -> String {
    format!("{}-{}", CONTAINER_PREFIX, session_id)
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

/// Tool to execute commands inside the Docker container
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
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
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

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        ToolExecutionResult::success(json!({
            "stdout": stdout,
            "stderr": stderr,
            "exit_code": exit_code,
            "success": exit_code == 0
        }))
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DockerReadFileTool
// ============================================================================

/// Tool to read a file from the Docker container
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

/// Tool to write a file to the Docker container
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

        // Write content using docker exec with heredoc-style input via stdin
        // We use base64 encoding to handle special characters safely
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

/// Tool to stop and remove the Docker container
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
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = DockerContainerCapability;
        assert_eq!(cap.id(), CapabilityId::DOCKER_CONTAINER);
        assert_eq!(cap.name(), "[Experimental] Docker Container");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("container"));
        assert_eq!(cap.category(), Some("Development"));
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = DockerContainerCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 4);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"docker_exec"));
        assert!(tool_names.contains(&"docker_read_file"));
        assert!(tool_names.contains(&"docker_write_file"));
        assert!(tool_names.contains(&"docker_stop"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = DockerContainerCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("docker_exec"));
        assert!(prompt.contains("docker_read_file"));
        assert!(prompt.contains("docker_write_file"));
        assert!(prompt.contains("docker_stop"));
        assert!(prompt.contains("EXPERIMENTAL"));
    }

    #[test]
    fn test_tools_require_context() {
        assert!(DockerExecTool.requires_context());
        assert!(DockerReadFileTool.requires_context());
        assert!(DockerWriteFileTool.requires_context());
        assert!(DockerStopTool.requires_context());
    }

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
        let session_id = uuid::Uuid::parse_str("12345678-1234-1234-1234-123456789012").unwrap();
        let name = container_name(&session_id);
        assert_eq!(name, "everruns-12345678-1234-1234-1234-123456789012");
    }

    #[tokio::test]
    async fn test_docker_exec_without_context() {
        let tool = DockerExecTool;
        let result = tool.execute(json!({"command": "echo hello"})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_docker_read_file_without_context() {
        let tool = DockerReadFileTool;
        let result = tool.execute(json!({"path": "/test.txt"})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_docker_write_file_without_context() {
        let tool = DockerWriteFileTool;
        let result = tool
            .execute(json!({"path": "/test.txt", "content": "hello"}))
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_docker_exec_missing_command() {
        let tool = DockerExecTool;
        let context = ToolContext::new(uuid::Uuid::nil());

        let result = tool.execute_with_context(json!({}), &context).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Missing required parameter"));
        } else {
            panic!("Expected tool error for missing command");
        }
    }

    #[tokio::test]
    async fn test_docker_read_file_missing_path() {
        let tool = DockerReadFileTool;
        let context = ToolContext::new(uuid::Uuid::nil());

        let result = tool.execute_with_context(json!({}), &context).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Missing required parameter"));
        } else {
            panic!("Expected tool error for missing path");
        }
    }

    #[tokio::test]
    async fn test_docker_write_file_missing_params() {
        let tool = DockerWriteFileTool;
        let context = ToolContext::new(uuid::Uuid::nil());

        // Missing path
        let result = tool
            .execute_with_context(json!({"content": "hello"}), &context)
            .await;
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Missing required parameter"));
        } else {
            panic!("Expected tool error for missing path");
        }

        // Missing content
        let result = tool
            .execute_with_context(json!({"path": "/test.txt"}), &context)
            .await;
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Missing required parameter"));
        } else {
            panic!("Expected tool error for missing content");
        }
    }

    #[tokio::test]
    async fn test_docker_stop_without_context() {
        let tool = DockerStopTool;
        let result = tool.execute(json!({})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }
}
