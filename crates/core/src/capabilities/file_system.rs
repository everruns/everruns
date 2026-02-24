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

use super::{Capability, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult, ToolResultImage};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};

/// Image MIME types recognized by LLM vision APIs (OpenAI, Anthropic)
const IMAGE_EXTENSIONS: &[(&str, &str)] = &[
    (".png", "image/png"),
    (".jpg", "image/jpeg"),
    (".jpeg", "image/jpeg"),
    (".gif", "image/gif"),
    (".webp", "image/webp"),
];

/// Get the image MIME type if the path has a known image extension
fn image_media_type(path: &str) -> Option<&'static str> {
    let lower = path.to_lowercase();
    IMAGE_EXTENSIONS
        .iter()
        .find(|(ext, _)| lower.ends_with(ext))
        .map(|(_, mime)| *mime)
}

/// Workspace prefix used in file paths
const WORKSPACE_PREFIX: &str = "/workspace";

/// Normalize a file path by stripping the /workspace prefix.
/// This ensures both file_system and virtual_bash capabilities use the same
/// path format in the session file store.
///
/// Examples:
/// - `/workspace/foo.txt` -> `/foo.txt`
/// - `/workspace` -> `/`
/// - `/foo.txt` -> `/foo.txt` (already normalized)
fn normalize_path(path: &str) -> String {
    if path == WORKSPACE_PREFIX {
        "/".to_string()
    } else if let Some(stripped) = path.strip_prefix(WORKSPACE_PREFIX) {
        if stripped.starts_with('/') {
            stripped.to_string()
        } else {
            // path like "/workspacefoo" - not a valid workspace path
            path.to_string()
        }
    } else {
        // Path doesn't start with /workspace - use as-is
        path.to_string()
    }
}

/// Add workspace prefix back to a path for display to the user
fn add_workspace_prefix(path: &str) -> String {
    if path == "/" {
        WORKSPACE_PREFIX.to_string()
    } else if path.starts_with('/') {
        format!("{}{}", WORKSPACE_PREFIX, path)
    } else {
        format!("{}/{}", WORKSPACE_PREFIX, path)
    }
}

/// Session File System capability - provides file operations for session storage
pub struct FileSystemCapability;

impl Capability for FileSystemCapability {
    fn id(&self) -> &str {
        "session_file_system"
    }

    fn name(&self) -> &str {
        "File System"
    }

    fn description(&self) -> &str {
        r#"Tools to access and manipulate files in the session workspace - read, write, list, grep, and more.

> [!NOTE]
> Each session has its own isolated workspace at `/workspace`. Files persist for the session duration.

> [!TIP]
> Use `list_directory` to explore the workspace structure before reading or writing files."#
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
            r#"You have access to file system tools for working with the session workspace. Each session has its own isolated workspace stored in the database.

**Workspace Location:** `/workspace`

All session files are stored under `/workspace`. This is the root of your persistent storage.

Available tools:
- `read_file`: Read the content of a file by path
- `write_file`: Create a new file or update existing file content
- `list_directory`: List files and directories at a given path
- `grep_files`: Search file contents using regex patterns
- `delete_file`: Delete a file or directory
- `stat_file`: Get metadata about a file (size, dates, etc.)

Best practices:
- All paths should start with `/workspace` (e.g., `/workspace/myfile.txt`)
- Use `list_directory` with path `/workspace` to explore the workspace
- Use `stat_file` to check if a file exists before reading/writing
- Use `grep_files` to search across multiple files efficiently
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

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

// ----------------------------------------------------------------------------
// ReadFileTool
// ----------------------------------------------------------------------------

/// Tool to read file content
pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Read File")
    }

    fn description(&self) -> &str {
        "Read the content of a file. Returns text content directly. For image files (PNG, JPEG, GIF, WebP), the image is returned as a native image so you can see it visually."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file (e.g., '/workspace/docs/readme.txt')"
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
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

        match file_store
            .read_file(context.session_id, &normalized_path)
            .await
        {
            Ok(Some(file)) => {
                if file.is_directory {
                    return ToolExecutionResult::tool_error(format!(
                        "Path '{}' is a directory, not a file. Use list_directory instead.",
                        display_path
                    ));
                }

                // Check if this is an image file that should be returned as native image content
                if let Some(media_type) = image_media_type(&normalized_path) {
                    // For base64-encoded files, return as image
                    if file.encoding == "base64"
                        && let Some(ref content) = file.content
                    {
                        return ToolExecutionResult::success_with_images(
                            json!({
                                "path": display_path,
                                "media_type": media_type,
                                "size_bytes": file.size_bytes
                            }),
                            vec![ToolResultImage {
                                base64: content.clone(),
                                media_type: media_type.to_string(),
                            }],
                        );
                    }
                    // Text-encoded image paths still get returned as text (unusual case)
                }

                ToolExecutionResult::success(json!({
                    "path": display_path,
                    "content": file.content,
                    "encoding": file.encoding,
                    "size_bytes": file.size_bytes
                }))
            }
            Ok(None) => {
                ToolExecutionResult::tool_error(format!("File not found: {}", display_path))
            }
            Err(e) => ToolExecutionResult::internal_error(e),
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ----------------------------------------------------------------------------
// WriteFileTool
// ----------------------------------------------------------------------------

/// Tool to write/create a file
pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Write File")
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
                    "description": "Absolute path for the file (e.g., '/workspace/docs/notes.txt')"
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
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

        match file_store
            .write_file(context.session_id, &normalized_path, content, encoding)
            .await
        {
            Ok(file) => ToolExecutionResult::success(json!({
                "path": display_path,
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

// ----------------------------------------------------------------------------
// ListDirectoryTool
// ----------------------------------------------------------------------------

/// Tool to list directory contents
pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn display_name(&self) -> Option<&str> {
        Some("List Directory")
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
                    "default": "/workspace",
                    "description": "Directory path to list (default: '/workspace')"
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
            .unwrap_or("/workspace");

        let file_store = match &context.file_store {
            Some(store) => store,
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

        match file_store
            .list_directory(context.session_id, &normalized_path)
            .await
        {
            Ok(files) => {
                let entries: Vec<Value> = files
                    .iter()
                    .map(|f| {
                        json!({
                            "name": f.name,
                            "path": add_workspace_prefix(&f.path),
                            "is_directory": f.is_directory,
                            "size_bytes": f.size_bytes,
                            "is_readonly": f.is_readonly
                        })
                    })
                    .collect();

                ToolExecutionResult::success(json!({
                    "path": display_path,
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

// ----------------------------------------------------------------------------
// GrepFilesTool
// ----------------------------------------------------------------------------

/// Tool to search files by pattern
pub struct GrepFilesTool;

#[async_trait]
impl Tool for GrepFilesTool {
    fn name(&self) -> &str {
        "grep_files"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Grep Files")
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
                    "description": "Optional path pattern to filter files (e.g., '*.txt', '/workspace/docs/*')"
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
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
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
                            "path": add_workspace_prefix(&m.path),
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

// ----------------------------------------------------------------------------
// DeleteFileTool
// ----------------------------------------------------------------------------

/// Tool to delete a file or directory
pub struct DeleteFileTool;

#[async_trait]
impl Tool for DeleteFileTool {
    fn name(&self) -> &str {
        "delete_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Delete File")
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
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

        match file_store
            .delete_file(context.session_id, &normalized_path, recursive)
            .await
        {
            Ok(deleted) => {
                if deleted {
                    ToolExecutionResult::success(json!({
                        "path": display_path,
                        "deleted": true
                    }))
                } else {
                    ToolExecutionResult::tool_error(format!("File not found: {}", display_path))
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

// ----------------------------------------------------------------------------
// StatFileTool
// ----------------------------------------------------------------------------

/// Tool to get file metadata
pub struct StatFileTool;

#[async_trait]
impl Tool for StatFileTool {
    fn name(&self) -> &str {
        "stat_file"
    }

    fn display_name(&self) -> Option<&str> {
        Some("File Info")
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
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Normalize path to strip /workspace prefix for storage
        let normalized_path = normalize_path(path);
        let display_path = add_workspace_prefix(&normalized_path);

        match file_store
            .stat_file(context.session_id, &normalized_path)
            .await
        {
            Ok(Some(stat)) => ToolExecutionResult::success(json!({
                "path": add_workspace_prefix(&stat.path),
                "name": stat.name,
                "exists": true,
                "is_directory": stat.is_directory,
                "is_readonly": stat.is_readonly,
                "size_bytes": stat.size_bytes,
                "created_at": stat.created_at.to_rfc3339(),
                "updated_at": stat.updated_at.to_rfc3339()
            })),
            Ok(None) => ToolExecutionResult::success(json!({
                "path": display_path,
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
    use crate::typed_id::SessionId;

    // Path normalization tests
    #[test]
    fn test_normalize_path_workspace_root() {
        assert_eq!(normalize_path("/workspace"), "/");
    }

    #[test]
    fn test_normalize_path_workspace_file() {
        assert_eq!(normalize_path("/workspace/test.txt"), "/test.txt");
    }

    #[test]
    fn test_normalize_path_workspace_nested() {
        assert_eq!(
            normalize_path("/workspace/foo/bar/test.txt"),
            "/foo/bar/test.txt"
        );
    }

    #[test]
    fn test_normalize_path_already_normalized() {
        assert_eq!(normalize_path("/test.txt"), "/test.txt");
    }

    #[test]
    fn test_normalize_path_invalid_workspace_prefix() {
        // /workspacefoo is not a valid workspace path (no slash after workspace)
        assert_eq!(normalize_path("/workspacefoo"), "/workspacefoo");
    }

    #[test]
    fn test_add_workspace_prefix_root() {
        assert_eq!(add_workspace_prefix("/"), "/workspace");
    }

    #[test]
    fn test_add_workspace_prefix_file() {
        assert_eq!(add_workspace_prefix("/test.txt"), "/workspace/test.txt");
    }

    #[test]
    fn test_add_workspace_prefix_nested() {
        assert_eq!(
            add_workspace_prefix("/foo/bar.txt"),
            "/workspace/foo/bar.txt"
        );
    }

    #[test]
    fn test_add_workspace_prefix_no_leading_slash() {
        assert_eq!(add_workspace_prefix("test.txt"), "/workspace/test.txt");
    }

    #[test]
    fn test_capability_metadata() {
        let cap = FileSystemCapability;
        assert_eq!(cap.id(), "session_file_system");
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
        let context = ToolContext::new(SessionId::new());

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
        let context = ToolContext::new(SessionId::new());

        let result = tool
            .execute_with_context(json!({"path": "/test.txt"}), &context)
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("not available"));
        } else {
            panic!("Expected tool error for missing file store");
        }
    }

    // Image detection tests
    #[test]
    fn test_image_media_type_png() {
        assert_eq!(
            image_media_type("/workspace/screenshot.png"),
            Some("image/png")
        );
    }

    #[test]
    fn test_image_media_type_jpeg() {
        assert_eq!(image_media_type("/workspace/photo.jpg"), Some("image/jpeg"));
        assert_eq!(
            image_media_type("/workspace/photo.jpeg"),
            Some("image/jpeg")
        );
    }

    #[test]
    fn test_image_media_type_gif() {
        assert_eq!(image_media_type("/data/anim.gif"), Some("image/gif"));
    }

    #[test]
    fn test_image_media_type_webp() {
        assert_eq!(image_media_type("/images/art.webp"), Some("image/webp"));
    }

    #[test]
    fn test_image_media_type_case_insensitive() {
        assert_eq!(image_media_type("/workspace/PHOTO.PNG"), Some("image/png"));
        assert_eq!(image_media_type("/workspace/image.JPG"), Some("image/jpeg"));
    }

    #[test]
    fn test_image_media_type_not_image() {
        assert_eq!(image_media_type("/workspace/readme.txt"), None);
        assert_eq!(image_media_type("/workspace/data.json"), None);
        assert_eq!(image_media_type("/workspace/script.py"), None);
    }
}
