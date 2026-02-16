//! Tool implementations for Daytona sandbox operations.

use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;

use async_trait::async_trait;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::client::DaytonaClient;
use crate::state::{
    SandboxState, delete_sandbox_state, get_api_key, get_sandbox_state, list_sandbox_states,
    required_str, save_sandbox_state,
};
use crate::{AUTO_STOP_INTERVAL_MINUTES, EXEC_TIMEOUT_MS};

// ============================================================================
// DaytonaCreateSandboxTool
// ============================================================================

pub struct DaytonaCreateSandboxTool;

#[async_trait]
impl Tool for DaytonaCreateSandboxTool {
    fn name(&self) -> &str {
        "daytona_create_sandbox"
    }

    fn description(&self) -> &str {
        "Create a new Daytona cloud sandbox. Optionally upload files from session storage."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Sandbox name (optional)"
                },
                "image": {
                    "type": "string",
                    "description": "Container image (optional, uses Daytona default if omitted)"
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
        let title = arguments
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Everruns Sandbox");
        let mut create_body = json!({
            "name": title,
            "autoStopInterval": AUTO_STOP_INTERVAL_MINUTES,
        });
        if let Some(image) = arguments.get("image").and_then(|v| v.as_str()) {
            create_body["image"] = json!(image);
        }

        // Create sandbox
        debug!("Creating Daytona sandbox: {title}");
        let sandbox_info = match client.create_sandbox(create_body).await {
            Ok(info) => info,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };

        let sandbox_id = &sandbox_info.id;

        // Wait for sandbox to reach "started" state
        debug!("Waiting for sandbox to start: {sandbox_id}");
        if let Err(e) = client.wait_for_ready(sandbox_id).await {
            warn!("Sandbox readiness check failed: {e}");
            // Continue anyway — the sandbox was created, agent can retry later
        }

        let workspace_path = "/home/daytona".to_string();

        // Save state
        let state = SandboxState {
            sandbox_id: sandbox_id.clone(),
            workspace_path: workspace_path.clone(),
            started_at: chrono::Utc::now().to_rfc3339(),
        };
        if let Err(e) = save_sandbox_state(context, &state).await {
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
                            .file_upload(sandbox_id, sandbox_path, content.as_bytes())
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

        ToolExecutionResult::success(json!({
            "sandbox_id": sandbox_id,
            "status": "running",
            "workspace_path": workspace_path,
            "files_uploaded": uploaded_count
        }))
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
    fn name(&self) -> &str {
        "daytona_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command in a Daytona sandbox. Returns output synchronously."
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
                    "description": "Timeout in milliseconds (optional, default: 120000)"
                }
            },
            "required": ["sandbox_id", "command"],
            "additionalProperties": false
        })
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
        let timeout = arguments
            .get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(EXEC_TIMEOUT_MS);

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        // Verify sandbox exists in state
        if let Err(e) = get_sandbox_state(context, sandbox_id).await {
            return e;
        }

        let client = DaytonaClient::new(api_key);

        debug!("Executing in sandbox {sandbox_id}: {command}");
        match client.exec(sandbox_id, command, cwd, Some(timeout)).await {
            Ok(result) => ToolExecutionResult::success(json!({
                "exit_code": result.exit_code,
                "output": result.result
            })),
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
    fn name(&self) -> &str {
        "daytona_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from a Daytona sandbox filesystem."
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
                }
            },
            "required": ["sandbox_id", "path"],
            "additionalProperties": false
        })
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

        let api_key = match get_api_key(context).await {
            Ok(k) => k,
            Err(e) => return e,
        };
        if let Err(e) = get_sandbox_state(context, sandbox_id).await {
            return e;
        }

        let client = DaytonaClient::new(api_key);

        match client.file_download(sandbox_id, path).await {
            Ok(bytes) => {
                let content = String::from_utf8_lossy(&bytes).to_string();
                ToolExecutionResult::success(json!({
                    "path": path,
                    "content": content
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
// DaytonaWriteFileTool
// ============================================================================

pub struct DaytonaWriteFileTool;

#[async_trait]
impl Tool for DaytonaWriteFileTool {
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
        if let Err(e) = get_sandbox_state(context, sandbox_id).await {
            return e;
        }

        let client = DaytonaClient::new(api_key);

        match client
            .file_upload(sandbox_id, path, content.as_bytes())
            .await
        {
            Ok(()) => ToolExecutionResult::success(json!({
                "path": path,
                "success": true
            })),
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
    fn name(&self) -> &str {
        "daytona_download_workspace"
    }

    fn description(&self) -> &str {
        "Download the entire sandbox workspace to session file storage."
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
        let session_root = arguments
            .get("session_path")
            .and_then(|v| v.as_str())
            .unwrap_or("/workspace");

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
                            let content = String::from_utf8_lossy(&bytes).to_string();
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
                                .write_file(context.session_id, &session_dest, &content, "utf-8")
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

        ToolExecutionResult::success(result)
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
        // Verify sandbox exists in state
        if let Err(e) = get_sandbox_state(context, sandbox_id).await {
            return e;
        }

        let client = DaytonaClient::new(api_key);

        let result = match action {
            "delete" => {
                let r = client.delete_sandbox(sandbox_id).await;
                if r.is_ok() {
                    let _ = delete_sandbox_state(context, sandbox_id).await;
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
            Ok(()) => ToolExecutionResult::success(json!({
                "sandbox_id": sandbox_id,
                "action": action,
                "success": true
            })),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
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
    use serde_json::json;

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
}
