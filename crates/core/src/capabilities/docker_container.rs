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
use crate::traits::{SessionFileStore, ToolContext};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, error, info, warn};

/// Default Docker image if none specified in config
const DEFAULT_IMAGE: &str = "mcr.microsoft.com/devcontainers/python:3.11";

/// Default working directory inside the container
const DEFAULT_WORKING_DIR: &str = "/workspace";

/// Container name prefix
const CONTAINER_PREFIX: &str = "everruns";

/// Default mount path inside container for session filesystem
const DEFAULT_SESSION_MOUNT_PATH: &str = "/session";

/// Default host base path for session file exports
const DEFAULT_HOST_BASE_PATH: &str = "/tmp/everruns-sessions";

/// Mount mode for session filesystem
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionMountMode {
    /// Disabled - no session filesystem mounting
    #[default]
    Disabled,
    /// Bind mount - export files to host directory and mount into container
    /// Requires shared filesystem between worker and Docker daemon
    BindMount,
    /// Copy - use `docker cp` to copy files into container after start
    /// Works in any Docker environment, no shared filesystem required
    Copy,
}

/// Configuration schema for the Docker Container capability
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DockerContainerConfig {
    /// Docker image to use (e.g., "mcr.microsoft.com/devcontainers/python:3.11")
    #[serde(default = "default_image")]
    pub image: String,

    /// Working directory inside the container
    #[serde(default = "default_working_dir")]
    pub working_dir: String,

    /// Session filesystem mount mode
    /// - "disabled": No mounting (default)
    /// - "bind_mount": Use Docker bind mount (requires shared filesystem)
    /// - "copy": Use docker exec to copy files (works anywhere, no shared filesystem needed)
    #[serde(default)]
    pub session_mount_mode: SessionMountMode,

    /// Path inside container where session filesystem is mounted/copied
    #[serde(default = "default_session_mount_path")]
    pub session_mount_path: String,

    /// Host base path for session file exports (session_id will be appended)
    /// Only used with bind_mount mode
    #[serde(default = "default_host_base_path")]
    pub host_base_path: String,
}

fn default_image() -> String {
    DEFAULT_IMAGE.to_string()
}

fn default_working_dir() -> String {
    DEFAULT_WORKING_DIR.to_string()
}

fn default_session_mount_path() -> String {
    DEFAULT_SESSION_MOUNT_PATH.to_string()
}

fn default_host_base_path() -> String {
    DEFAULT_HOST_BASE_PATH.to_string()
}

impl Default for DockerContainerConfig {
    fn default() -> Self {
        Self {
            image: default_image(),
            working_dir: default_working_dir(),
            session_mount_mode: SessionMountMode::default(),
            session_mount_path: default_session_mount_path(),
            host_base_path: default_host_base_path(),
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

Session Filesystem Mount (if enabled via config):
- The session's virtual filesystem is mounted at /session (configurable)
- Files from the session filesystem are available inside the container
- Modifications to files in /session are persisted on the host
- This allows agents to work with pre-populated files from capability mounts

Best practices:
- Use `docker_exec` with `bash -c "..."` for complex commands
- Check exit codes to verify command success
- The working directory defaults to /workspace
- Files written persist for the session duration
- If session filesystem is mounted, access files at /session
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

/// Volume mount specification for container
#[derive(Debug, Clone)]
struct VolumeMount {
    host_path: PathBuf,
    container_path: String,
    readonly: bool,
}

/// Ensure container is running, starting it if necessary
async fn ensure_container_running(
    name: &str,
    config: &DockerContainerConfig,
    mounts: Option<Vec<VolumeMount>>,
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

    // Build command arguments
    let mut args = vec![
        "run".to_string(),
        "-d".to_string(), // Detached mode
        "--name".to_string(),
        name.to_string(), // Container name
        "--network".to_string(),
        "host".to_string(), // Host networking
        "-w".to_string(),
        config.working_dir.clone(), // Working directory
        "--init".to_string(),       // Use init process
    ];

    // Add volume mounts if specified
    if let Some(mounts) = mounts {
        for mount in mounts {
            let mount_spec = if mount.readonly {
                format!("{}:{}:ro", mount.host_path.display(), mount.container_path)
            } else {
                format!("{}:{}", mount.host_path.display(), mount.container_path)
            };
            args.push("-v".to_string());
            args.push(mount_spec);
        }
    }

    // Add image and command
    args.push(config.image.clone());
    args.push("tail".to_string());
    args.push("-f".to_string());
    args.push("/dev/null".to_string());

    let output = Command::new("docker")
        .args(&args)
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

/// Ensure container is running with optional session filesystem mount
///
/// Supports two modes:
/// - BindMount: Export files to host directory and mount into container
/// - Copy: Start container first, then use `docker cp` to copy files in
async fn ensure_container_with_session_mount(
    name: &str,
    session_id: &uuid::Uuid,
    config: &DockerContainerConfig,
    file_store: Option<&Arc<dyn SessionFileStore>>,
) -> Result<(), String> {
    match &config.session_mount_mode {
        SessionMountMode::Disabled => {
            // No mounting, just ensure container is running
            ensure_container_running(name, config, None).await
        }
        SessionMountMode::BindMount => {
            // Bind mount mode: export files to host, then mount
            if let Some(store) = file_store {
                let host_path = session_host_path(config, session_id);

                // Export session files to host before container starts
                // Only export if container doesn't already exist (to avoid re-exporting on restart)
                if !container_exists(name).await {
                    export_session_files(store, *session_id, &host_path).await?;
                }

                let mounts = vec![VolumeMount {
                    host_path,
                    container_path: config.session_mount_path.clone(),
                    readonly: false,
                }];

                ensure_container_running(name, config, Some(mounts)).await
            } else {
                warn!("bind_mount mode enabled but no file_store available, skipping mount");
                ensure_container_running(name, config, None).await
            }
        }
        SessionMountMode::Copy => {
            // Copy mode: start container first, then copy files in
            let container_was_new = !container_exists(name).await;

            // Start container without mounts
            ensure_container_running(name, config, None).await?;

            // Copy files into container if it was just created
            if container_was_new {
                if let Some(store) = file_store {
                    copy_session_files_to_container(
                        store,
                        *session_id,
                        name,
                        &config.session_mount_path,
                    )
                    .await?;
                } else {
                    warn!("copy mode enabled but no file_store available, skipping copy");
                }
            }

            Ok(())
        }
    }
}

/// Get the host path for session files
fn session_host_path(config: &DockerContainerConfig, session_id: &uuid::Uuid) -> PathBuf {
    PathBuf::from(&config.host_base_path).join(session_id.to_string())
}

/// Copy session files directly into the container using docker exec
///
/// This approach doesn't require a shared filesystem between the worker and Docker daemon.
/// Files are written directly into the container via `docker exec` with base64-encoded content.
async fn copy_session_files_to_container(
    file_store: &Arc<dyn SessionFileStore>,
    session_id: uuid::Uuid,
    container_name: &str,
    mount_path: &str,
) -> Result<usize, String> {
    // Create the mount directory in the container
    let mkdir_output = Command::new("docker")
        .args(["exec", container_name, "mkdir", "-p", mount_path])
        .output()
        .await
        .map_err(|e| format!("Failed to create mount directory: {}", e))?;

    if !mkdir_output.status.success() {
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr);
        return Err(format!("Failed to create mount directory: {}", stderr));
    }

    // List all files from the session filesystem
    let files = list_all_files_iterative(file_store, session_id).await?;

    let mut copied = 0;
    for file_info in files {
        // Build the container path
        let container_file_path = format!(
            "{}/{}",
            mount_path.trim_end_matches('/'),
            file_info.path.trim_start_matches('/')
        );

        if file_info.is_directory {
            // Create directory in container
            let output = Command::new("docker")
                .args(["exec", container_name, "mkdir", "-p", &container_file_path])
                .output()
                .await
                .map_err(|e| {
                    format!("Failed to create directory {}: {}", container_file_path, e)
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(
                    "Failed to create directory {} in container: {}",
                    container_file_path, stderr
                );
            }
        } else {
            // Read file content
            if let Some(session_file) = file_store
                .read_file(session_id, &file_info.path)
                .await
                .map_err(|e| format!("Failed to read file {}: {}", file_info.path, e))?
            {
                let Some(content) = session_file.content else {
                    continue;
                };

                // Ensure parent directory exists
                if let Some(parent) = std::path::Path::new(&container_file_path).parent() {
                    let parent_str = parent.to_string_lossy();
                    let _ = Command::new("docker")
                        .args(["exec", container_name, "mkdir", "-p", &parent_str])
                        .output()
                        .await;
                }

                // Get content bytes (decode base64 if needed)
                let content_bytes: Vec<u8> = if session_file.encoding == "base64" {
                    base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        content.as_bytes(),
                    )
                    .map_err(|e| format!("Failed to decode base64 content: {}", e))?
                } else {
                    content.into_bytes()
                };

                // Encode as base64 for safe transport via shell
                let encoded = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &content_bytes,
                );

                // Write file using docker exec with base64 decode
                let output = Command::new("docker")
                    .args([
                        "exec",
                        container_name,
                        "sh",
                        "-c",
                        &format!("echo '{}' | base64 -d > '{}'", encoded, container_file_path),
                    ])
                    .output()
                    .await
                    .map_err(|e| format!("Failed to write file {}: {}", container_file_path, e))?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    warn!(
                        "Failed to write file {} in container: {}",
                        container_file_path, stderr
                    );
                } else {
                    // Set readonly permissions if applicable
                    if session_file.is_readonly {
                        let _ = Command::new("docker")
                            .args(["exec", container_name, "chmod", "444", &container_file_path])
                            .output()
                            .await;
                    }
                    copied += 1;
                }
            }
        }
    }

    info!(
        "Copied {} session files to container {} at {}",
        copied, container_name, mount_path
    );
    Ok(copied)
}

/// Export session files to the host filesystem for Docker mounting
///
/// This function reads all files from the session filesystem and writes them
/// to a host directory that can be mounted into the Docker container.
async fn export_session_files(
    file_store: &Arc<dyn SessionFileStore>,
    session_id: uuid::Uuid,
    host_path: &PathBuf,
) -> Result<usize, String> {
    // Create the host directory
    tokio::fs::create_dir_all(host_path)
        .await
        .map_err(|e| format!("Failed to create host directory: {}", e))?;

    // List all files recursively starting from root using iterative approach
    let files = list_all_files_iterative(file_store, session_id).await?;

    let mut exported = 0;
    for file_info in files {
        let file_path = host_path.join(file_info.path.trim_start_matches('/'));

        if file_info.is_directory {
            // Create directory
            tokio::fs::create_dir_all(&file_path).await.map_err(|e| {
                format!("Failed to create directory {}: {}", file_path.display(), e)
            })?;
        } else {
            // Read file content and write to host
            if let Some(session_file) = file_store
                .read_file(session_id, &file_info.path)
                .await
                .map_err(|e| format!("Failed to read file {}: {}", file_info.path, e))?
            {
                // Skip if no content (shouldn't happen for files, but be safe)
                let Some(content) = session_file.content else {
                    continue;
                };

                // Ensure parent directory exists
                if let Some(parent) = file_path.parent() {
                    tokio::fs::create_dir_all(parent)
                        .await
                        .map_err(|e| format!("Failed to create parent directory: {}", e))?;
                }

                // Decode content if base64 encoded
                let content_bytes: Vec<u8> = if session_file.encoding == "base64" {
                    base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        content.as_bytes(),
                    )
                    .map_err(|e| format!("Failed to decode base64 content: {}", e))?
                } else {
                    content.into_bytes()
                };

                tokio::fs::write(&file_path, &content_bytes)
                    .await
                    .map_err(|e| format!("Failed to write file {}: {}", file_path.display(), e))?;

                // Set readonly if applicable
                if session_file.is_readonly {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let perms = std::fs::Permissions::from_mode(0o444);
                        tokio::fs::set_permissions(&file_path, perms)
                            .await
                            .map_err(|e| {
                                format!(
                                    "Failed to set readonly permissions on {}: {}",
                                    file_path.display(),
                                    e
                                )
                            })?;
                    }
                }

                exported += 1;
            }
        }
    }

    info!(
        "Exported {} session files to {}",
        exported,
        host_path.display()
    );
    Ok(exported)
}

/// Iteratively list all files in the session filesystem (avoids async recursion)
async fn list_all_files_iterative(
    file_store: &Arc<dyn SessionFileStore>,
    session_id: uuid::Uuid,
) -> Result<Vec<crate::session_file::FileInfo>, String> {
    let mut all_files = Vec::new();
    let mut dirs_to_process = vec!["/".to_string()];

    while let Some(dir_path) = dirs_to_process.pop() {
        let entries = file_store
            .list_directory(session_id, &dir_path)
            .await
            .map_err(|e| format!("Failed to list directory {}: {}", dir_path, e))?;

        for entry in entries {
            if entry.is_directory {
                // Queue subdirectory for processing
                dirs_to_process.push(entry.path.clone());
            }
            all_files.push(entry);
        }
    }

    Ok(all_files)
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

        // Ensure container is running with optional session filesystem mount
        if let Err(e) = ensure_container_with_session_mount(
            &name,
            &context.session_id,
            &config,
            context.file_store.as_ref(),
        )
        .await
        {
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

        // Ensure container is running with optional session filesystem mount
        if let Err(e) = ensure_container_with_session_mount(
            &name,
            &context.session_id,
            &config,
            context.file_store.as_ref(),
        )
        .await
        {
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

        // Ensure container is running with optional session filesystem mount
        if let Err(e) = ensure_container_with_session_mount(
            &name,
            &context.session_id,
            &config,
            context.file_store.as_ref(),
        )
        .await
        {
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
        assert_eq!(config.session_mount_mode, SessionMountMode::Disabled);
        assert_eq!(config.session_mount_path, DEFAULT_SESSION_MOUNT_PATH);
        assert_eq!(config.host_base_path, DEFAULT_HOST_BASE_PATH);
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
        assert_eq!(config.session_mount_mode, SessionMountMode::Disabled);
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
    fn test_config_parse_with_copy_mode() {
        let json = json!({
            "image": "python:3.11",
            "session_mount_mode": "copy",
            "session_mount_path": "/data"
        });
        let config = parse_config(&json);
        assert_eq!(config.image, "python:3.11");
        assert_eq!(config.session_mount_mode, SessionMountMode::Copy);
        assert_eq!(config.session_mount_path, "/data");
    }

    #[test]
    fn test_config_parse_with_bind_mount_mode() {
        let json = json!({
            "image": "python:3.11",
            "session_mount_mode": "bind_mount",
            "host_base_path": "/var/sessions"
        });
        let config = parse_config(&json);
        assert_eq!(config.session_mount_mode, SessionMountMode::BindMount);
        assert_eq!(config.host_base_path, "/var/sessions");
    }

    #[test]
    fn test_session_host_path() {
        let config = DockerContainerConfig {
            host_base_path: "/tmp/test-sessions".to_string(),
            ..Default::default()
        };
        let session_id = uuid::Uuid::parse_str("12345678-1234-1234-1234-123456789012").unwrap();
        let path = session_host_path(&config, &session_id);
        assert_eq!(
            path.to_string_lossy(),
            "/tmp/test-sessions/12345678-1234-1234-1234-123456789012"
        );
    }

    #[test]
    fn test_volume_mount_readonly() {
        let mount = VolumeMount {
            host_path: PathBuf::from("/host/path"),
            container_path: "/container/path".to_string(),
            readonly: true,
        };
        assert!(mount.readonly);
    }

    #[test]
    fn test_volume_mount_readwrite() {
        let mount = VolumeMount {
            host_path: PathBuf::from("/host/path"),
            container_path: "/container/path".to_string(),
            readonly: false,
        };
        assert!(!mount.readonly);
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
