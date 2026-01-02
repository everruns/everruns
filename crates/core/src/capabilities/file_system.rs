//! Session File System Capability
//!
//! This capability provides tools for interacting with the session file system.
//! Each session has its own isolated filesystem stored in the database.
//!
//! Tools provided:
//! - `read_file`: Read file content
//! - `write_file`: Create or update a file
//! - `list_directory`: List files in a directory
//! - `grep_files`: Search files by regex pattern
//! - `delete_file`: Delete a file or directory
//! - `stat_file`: Get file metadata

use super::{Capability, CapabilityId, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{json, Value};

/// Session File System capability - provides file operations for session storage
pub struct FileSystemCapability;

impl Capability for FileSystemCapability {
    fn id(&self) -> &str {
        CapabilityId::FILE_SYSTEM
    }

    fn name(&self) -> &str {
        "File System"
    }

    fn description(&self) -> &str {
        "Tools to access and manipulate files in the session file system - read, write, list, grep, and more."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("hard-drive")
    }

    fn category(&self) -> Option<&str> {
        Some("File Operations")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to file system tools for working with the session file system. Each session has its own isolated filesystem stored in the database.

Available tools:
- `read_file`: Read the content of a file by path
- `write_file`: Create a new file or update existing file content
- `list_directory`: List files and directories at a given path
- `grep_files`: Search file contents using regex patterns
- `delete_file`: Delete a file or directory
- `stat_file`: Get metadata about a file (size, dates, etc.)

Best practices:
- Use `list_directory` first to explore the filesystem structure
- Use `stat_file` to check if a file exists before reading/writing
- Use `grep_files` to search across multiple files efficiently
- The root directory is `/` - all paths should be absolute
- Directories are created automatically when writing files"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(ReadFileTool),
            Box::new(WriteFileTool),
            Box::new(ListDirectoryTool),
            Box::new(GrepFilesTool),
            Box::new(DeleteFileTool),
            Box::new(StatFileTool),
        ]
    }
}

// ============================================================================
// ReadFileTool
// ============================================================================

/// Tool to read file content
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read the content of a file. Returns the file content as text or base64-encoded binary."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file (e.g., '/docs/readme.txt')"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "read_file requires context. This tool must be executed with session context.",
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

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("File system not available in this context")
            }
        };

        match file_store.read_file(context.session_id, path).await {
            Ok(Some(file)) => {
                if file.is_directory {
                    ToolExecutionResult::tool_error(format!(
                        "Path '{}' is a directory, not a file. Use list_directory instead.",
                        path
                    ))
                } else {
                    ToolExecutionResult::success(json!({
                        "path": file.path,
                        "content": file.content,
                        "encoding": file.encoding,
                        "size_bytes": file.size_bytes
                    }))
                }
            }
            Ok(None) => ToolExecutionResult::tool_error(format!("File not found: {}", path)),
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// WriteFileTool
// ============================================================================

/// Tool to write/create a file
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Create a new file or update an existing file's content. Parent directories are created automatically."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path for the file (e.g., '/docs/notes.txt')"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                },
                "encoding": {
                    "type": "string",
                    "enum": ["text", "base64"],
                    "default": "text",
                    "description": "Content encoding: 'text' for plain text, 'base64' for binary data"
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "write_file requires context. This tool must be executed with session context.",
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

        let encoding = arguments
            .get("encoding")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("File system not available in this context")
            }
        };

        match file_store
            .write_file(context.session_id, path, content, encoding)
            .await
        {
            Ok(file) => ToolExecutionResult::success(json!({
                "path": file.path,
                "size_bytes": file.size_bytes,
                "created": true
            })),
            Err(e) => {
                // Check if it's a user-facing error (like readonly file)
                let msg = e.to_string();
                if msg.contains("readonly") || msg.contains("is a directory") {
                    ToolExecutionResult::tool_error(msg)
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// ListDirectoryTool
// ============================================================================

/// Tool to list directory contents
pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List files and directories at a given path. Returns file metadata including size and type."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "default": "/",
                    "description": "Directory path to list (default: root '/')"
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "list_directory requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let path = arguments
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("/");

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("File system not available in this context")
            }
        };

        match file_store.list_directory(context.session_id, path).await {
            Ok(files) => {
                let entries: Vec<Value> = files
                    .iter()
                    .map(|f| {
                        json!({
                            "name": f.name,
                            "path": f.path,
                            "is_directory": f.is_directory,
                            "size_bytes": f.size_bytes,
                            "is_readonly": f.is_readonly
                        })
                    })
                    .collect();

                ToolExecutionResult::success(json!({
                    "path": path,
                    "entries": entries,
                    "count": entries.len()
                }))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("not a directory") {
                    ToolExecutionResult::tool_error(msg)
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// GrepFilesTool
// ============================================================================

/// Tool to search files by pattern
pub struct GrepFilesTool;

#[async_trait]
impl Tool for GrepFilesTool {
    fn name(&self) -> &str {
        "grep_files"
    }

    fn description(&self) -> &str {
        "Search file contents using a regex pattern. Returns matching lines with file paths and line numbers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path_pattern": {
                    "type": "string",
                    "description": "Optional path pattern to filter files (e.g., '*.txt', '/docs/*')"
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "grep_files requires context. This tool must be executed with session context.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let pattern = match arguments.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("Missing required parameter: pattern"),
        };

        let path_pattern = arguments.get("path_pattern").and_then(|v| v.as_str());

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("File system not available in this context")
            }
        };

        match file_store
            .grep_files(context.session_id, pattern, path_pattern)
            .await
        {
            Ok(matches) => {
                let results: Vec<Value> = matches
                    .iter()
                    .map(|m| {
                        json!({
                            "path": m.path,
                            "line_number": m.line_number,
                            "line": m.line
                        })
                    })
                    .collect();

                ToolExecutionResult::success(json!({
                    "pattern": pattern,
                    "matches": results,
                    "match_count": results.len()
                }))
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("regex") || msg.contains("pattern") {
                    ToolExecutionResult::tool_error(format!("Invalid regex pattern: {}", msg))
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// DeleteFileTool
// ============================================================================

/// Tool to delete a file or directory
pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
    fn name(&self) -> &str {
        "delete_file"
    }

    fn description(&self) -> &str {
        "Delete a file or directory. Use recursive=true to delete non-empty directories."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file or directory to delete"
                },
                "recursive": {
                    "type": "boolean",
                    "default": false,
                    "description": "If true, delete directories and all contents recursively"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "delete_file requires context. This tool must be executed with session context.",
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

        let recursive = arguments
            .get("recursive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("File system not available in this context")
            }
        };

        match file_store
            .delete_file(context.session_id, path, recursive)
            .await
        {
            Ok(deleted) => {
                if deleted {
                    ToolExecutionResult::success(json!({
                        "path": path,
                        "deleted": true
                    }))
                } else {
                    ToolExecutionResult::tool_error(format!("File not found: {}", path))
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("not empty") || msg.contains("recursive") {
                    ToolExecutionResult::tool_error(msg)
                } else {
                    ToolExecutionResult::internal_error(e)
                }
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// StatFileTool
// ============================================================================

/// Tool to get file metadata
pub struct StatFileTool;

#[async_trait]
impl Tool for StatFileTool {
    fn name(&self) -> &str {
        "stat_file"
    }

    fn description(&self) -> &str {
        "Get metadata about a file or directory (exists, size, type, dates)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file or directory"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "stat_file requires context. This tool must be executed with session context.",
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

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error("File system not available in this context")
            }
        };

        match file_store.stat_file(context.session_id, path).await {
            Ok(Some(stat)) => ToolExecutionResult::success(json!({
                "path": stat.path,
                "name": stat.name,
                "exists": true,
                "is_directory": stat.is_directory,
                "is_readonly": stat.is_readonly,
                "size_bytes": stat.size_bytes,
                "created_at": stat.created_at.to_rfc3339(),
                "updated_at": stat.updated_at.to_rfc3339()
            })),
            Ok(None) => ToolExecutionResult::success(json!({
                "path": path,
                "exists": false
            })),
            Err(e) => ToolExecutionResult::internal_error(e),
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
    fn test_capability_metadata() {
        let cap = FileSystemCapability;
        assert_eq!(cap.id(), CapabilityId::FILE_SYSTEM);
        assert_eq!(cap.name(), "File System");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("hard-drive"));
        assert_eq!(cap.category(), Some("File Operations"));
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = FileSystemCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 6);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(tool_names.contains(&"read_file"));
        assert!(tool_names.contains(&"write_file"));
        assert!(tool_names.contains(&"list_directory"));
        assert!(tool_names.contains(&"grep_files"));
        assert!(tool_names.contains(&"delete_file"));
        assert!(tool_names.contains(&"stat_file"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = FileSystemCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("read_file"));
        assert!(prompt.contains("write_file"));
        assert!(prompt.contains("list_directory"));
    }

    #[test]
    fn test_tools_require_context() {
        assert!(ReadFileTool.requires_context());
        assert!(WriteFileTool.requires_context());
        assert!(ListDirectoryTool.requires_context());
        assert!(GrepFilesTool.requires_context());
        assert!(DeleteFileTool.requires_context());
        assert!(StatFileTool.requires_context());
    }

    #[tokio::test]
    async fn test_read_file_without_context() {
        let tool = ReadFileTool;
        let result = tool.execute(json!({"path": "/test.txt"})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_write_file_without_context() {
        let tool = WriteFileTool;
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
    async fn test_read_file_missing_path() {
        let tool = ReadFileTool;
        let context = ToolContext::new(uuid::Uuid::nil());

        let result = tool.execute_with_context(json!({}), &context).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Missing required parameter"));
        } else {
            panic!("Expected tool error for missing path");
        }
    }

    #[tokio::test]
    async fn test_read_file_no_file_store() {
        let tool = ReadFileTool;
        let context = ToolContext::new(uuid::Uuid::nil());

        let result = tool
            .execute_with_context(json!({"path": "/test.txt"}), &context)
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("not available"));
        } else {
            panic!("Expected tool error for missing file store");
        }
    }
}
