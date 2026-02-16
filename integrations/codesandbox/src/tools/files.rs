//! File operation tools: read, write, and download workspace.

use crate::client::CodeSandboxClient;
use crate::state::*;

use async_trait::async_trait;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;
use serde_json::{Value, json};
use tracing::debug;

// ============================================================================
// CsbReadFileTool
// ============================================================================

pub struct CsbReadFileTool;

#[async_trait]
impl Tool for CsbReadFileTool {
    fn name(&self) -> &str {
        "csb_read_file"
    }

    fn description(&self) -> &str {
        "Read a file from a CodeSandbox VM filesystem."
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
                    "description": "Absolute path to file in sandbox (e.g., '/project/main.py')"
                }
            },
            "required": ["sandbox_id", "path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "csb_read_file requires context. This tool must be executed with session context.",
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
        let state = match get_sandbox_state(context, sandbox_id).await {
            Ok(s) => s,
            Err(e) => return e,
        };

        let client = CodeSandboxClient::new(api_key);

        match client.file_read(&state, path).await {
            Ok(content) => ToolExecutionResult::success(json!({
                "path": content.path,
                "content": content.content
            })),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// CsbWriteFileTool
// ============================================================================

pub struct CsbWriteFileTool;

#[async_trait]
impl Tool for CsbWriteFileTool {
    fn name(&self) -> &str {
        "csb_write_file"
    }

    fn description(&self) -> &str {
        "Write content to a file in a CodeSandbox VM filesystem."
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
                    "description": "Absolute path for file in sandbox (e.g., '/project/main.py')"
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
            "csb_write_file requires context. This tool must be executed with session context.",
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
            Ok(s) => s,
            Err(e) => return e,
        };

        let client = CodeSandboxClient::new(api_key);

        match client.file_write(&state, path, content).await {
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
// CsbDownloadWorkspaceTool
// ============================================================================

pub struct CsbDownloadWorkspaceTool;

#[async_trait]
impl Tool for CsbDownloadWorkspaceTool {
    fn name(&self) -> &str {
        "csb_download_workspace"
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
            "csb_download_workspace requires context. This tool must be executed with session context.",
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

        let client = CodeSandboxClient::new(api_key);

        // Recursively list and download files
        let mut downloaded = 0u64;
        let mut skipped = 0u64;
        let mut errors = Vec::new();

        let mut dirs_to_visit = vec![sandbox_root.to_string()];

        while let Some(dir_path) = dirs_to_visit.pop() {
            let entries = match client.dir_list(&state, &dir_path).await {
                Ok(e) => e,
                Err(e) => {
                    errors.push(format!("Failed to list {dir_path}: {e}"));
                    continue;
                }
            };

            for entry in entries {
                let full_path = if dir_path.ends_with('/') {
                    format!("{}{}", dir_path, entry.name)
                } else {
                    format!("{}/{}", dir_path, entry.name)
                };

                let is_dir = entry.is_dir;

                if is_dir {
                    dirs_to_visit.push(full_path);
                } else {
                    // Download file
                    match client.file_read(&state, &full_path).await {
                        Ok(content) => {
                            // Compute session destination path
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
                                .write_file(
                                    context.session_id,
                                    &session_dest,
                                    &content.content,
                                    "utf-8",
                                )
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

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::tools::Tool;

    #[test]
    fn test_read_file_schema() {
        let tool = CsbReadFileTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"path"));
    }

    #[test]
    fn test_write_file_schema() {
        let tool = CsbWriteFileTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"sandbox_id"));
        assert!(required_strs.contains(&"path"));
        assert!(required_strs.contains(&"content"));
    }

    #[test]
    fn test_download_workspace_schema() {
        let tool = CsbDownloadWorkspaceTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert_eq!(required.len(), 1);
        assert_eq!(required[0].as_str().unwrap(), "sandbox_id");
    }

    #[tokio::test]
    async fn test_read_file_without_context() {
        let tool = CsbReadFileTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "path": "/test.txt"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_write_file_without_context() {
        let tool = CsbWriteFileTool;
        let result = tool
            .execute(json!({"sandbox_id": "test", "path": "/test.txt", "content": "hello"}))
            .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            _ => panic!("Expected tool error"),
        }
    }

    #[tokio::test]
    async fn test_download_workspace_without_context() {
        let tool = CsbDownloadWorkspaceTool;
        let result = tool.execute(json!({"sandbox_id": "test"})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("requires context")),
            _ => panic!("Expected tool error"),
        }
    }

    #[test]
    fn test_all_file_tools_require_context() {
        assert!(CsbReadFileTool.requires_context());
        assert!(CsbWriteFileTool.requires_context());
        assert!(CsbDownloadWorkspaceTool.requires_context());
    }
}
