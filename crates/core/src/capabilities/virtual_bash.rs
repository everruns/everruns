//! Virtual Bash Capability
//!
//! This capability provides a sandboxed bash interpreter using bashkit.
//! The bash environment uses a custom FileSystem adapter that bridges
//! directly to the session file store.
//!
//! Design decisions:
//! - SessionFileSystemAdapter implements bashkit's FileSystem trait
//! - Direct delegation to SessionFileStore - no sync overhead
//! - Live visibility: files written by other tools are immediately visible
//! - Resource limits prevent runaway scripts (max commands, loop iterations)
//! - Context-aware tool that requires session filesystem access

use super::{Capability, CapabilityStatus};
use crate::session_file::SessionFile;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{SessionFileStore, ToolContext};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use bashkit::{Bash, DirEntry, ExecutionLimits, FileSystem, FileType, Metadata};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Virtual Bash capability - execute bash commands in a sandboxed environment
pub struct VirtualBashCapability;

impl Capability for VirtualBashCapability {
    fn id(&self) -> &str {
        "virtual_bash"
    }

    fn name(&self) -> &str {
        "Virtual Bash"
    }

    fn description(&self) -> &str {
        r#"Execute bash commands in an isolated, sandboxed environment.

> [!NOTE]
> Commands run in a virtual environment with no access to the host system.
> The session filesystem is mounted at root, so you can read and write session files.

> [!TIP]
> Use standard Unix commands like `ls`, `cat`, `grep`, `echo`, and shell features
> like pipes, redirections, and command substitution."#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("terminal")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"You have access to a virtual bash shell that executes commands in an isolated environment.

## Bash Tool

Use the `bash` tool to execute shell commands. The environment includes:

**Built-in Commands:**
- File operations: `cat`, `ls`, `cp`, `mv`, `rm`, `mkdir`, `touch`
- Text processing: `echo`, `printf`, `grep`, `sed`, `awk`, `jq`
- Control flow: `if/else`, `for`, `while`, `case`
- Variables: `export`, `set`, `unset`, `local`
- Other: `cd`, `pwd`, `test`, `[`, `true`, `false`, `exit`, `source`

**Shell Features:**
- Pipes: `cmd1 | cmd2`
- Redirections: `>`, `>>`, `<`, `<<<`
- Command substitution: `$(cmd)`
- Arithmetic: `$((1 + 2))`
- Parameter expansion: `$VAR`, `${VAR:-default}`, `${#VAR}`
- Glob patterns: `*.txt`, `**/*.rs`
- Here documents: `<<EOF ... EOF`

**Workspace:**
- Session data is mounted at `/workspace`
- Files you create with `write_file` are accessible in bash at `/workspace`
- Files created in bash under `/workspace` are accessible via `read_file`
- Working directory starts at `/workspace`
- HOME is set to `/home/agent`

**Limits:**
- Maximum 1000 commands per execution
- Maximum 10000 loop iterations
- Maximum 100 function call depth

**Best Practices:**
- Use bash for complex text processing and file manipulation
- Combine multiple operations with pipes for efficiency
- Check exit codes for error handling: `cmd && echo "success" || echo "failed"`"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BashTool)]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        // Depends on session filesystem for file access
        vec!["session_file_system"]
    }
}

// ============================================================================
// BashTool
// ============================================================================

/// Tool to execute bash commands in a sandboxed environment
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command in a sandboxed environment. The session filesystem is mounted at root."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "working_dir": {
                    "type": "string",
                    "default": "/workspace",
                    "description": "Working directory for command execution (default: '/workspace')"
                },
                "timeout_ms": {
                    "type": "integer",
                    "default": 30000,
                    "description": "Execution timeout in milliseconds (default: 30000, max: 60000)"
                }
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "bash requires context. This tool must be executed with session context.",
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

        let working_dir = arguments
            .get("working_dir")
            .and_then(|v| v.as_str())
            .unwrap_or("/workspace");

        let timeout_ms = arguments
            .get("timeout_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(30000)
            .min(60000);

        let file_store = match &context.file_store {
            Some(store) => store.clone(),
            None => {
                return ToolExecutionResult::tool_error(
                    "File system not available in this context",
                );
            }
        };

        // Create filesystem adapter that bridges to session file store
        let session_fs = Arc::new(SessionFileSystemAdapter::new(
            context.session_id,
            file_store,
        ));

        // Configure bash with resource limits
        let limits = ExecutionLimits::new()
            .max_commands(1000)
            .max_loop_iterations(10000)
            .max_function_depth(100);

        let mut bash = Bash::builder()
            .fs(session_fs)
            .cwd(working_dir)
            .env("HOME", "/home/agent")
            .env("USER", "agent")
            .env("SHELL", "/bin/bash")
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("WORKSPACE", "/workspace")
            .limits(limits)
            .build();

        // Execute with timeout
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            bash.exec(command),
        )
        .await;

        match result {
            Ok(Ok(output)) => ToolExecutionResult::success(json!({
                "stdout": output.stdout,
                "stderr": output.stderr,
                "exit_code": output.exit_code,
                "success": output.exit_code == 0
            })),
            Ok(Err(e)) => {
                // Execution error (syntax error, resource limit, etc.)
                ToolExecutionResult::tool_error(format!("Bash execution error: {}", e))
            }
            Err(_) => {
                // Timeout
                ToolExecutionResult::tool_error(format!("Command timed out after {}ms", timeout_ms))
            }
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// SessionFileSystemAdapter
// ============================================================================

/// Adapter that implements bashkit's FileSystem trait by delegating to SessionFileStore.
///
/// This provides live visibility of session files during bash execution - any files
/// written by other tools are immediately visible, and vice versa.
pub struct SessionFileSystemAdapter {
    session_id: SessionId,
    store: Arc<dyn SessionFileStore>,
}

impl SessionFileSystemAdapter {
    pub fn new(session_id: SessionId, store: Arc<dyn SessionFileStore>) -> Self {
        Self { session_id, store }
    }

    /// Workspace mount point in the virtual filesystem
    const WORKSPACE_PREFIX: &'static str = "/workspace";

    /// Convert bash path to session file store path.
    /// Paths under /workspace are mapped to session store.
    /// Returns None for paths outside /workspace.
    fn to_session_path(path: &Path) -> Option<String> {
        let path_str = path.to_string_lossy();

        // Normalize to absolute path
        let abs_path = if path_str.starts_with('/') {
            path_str.to_string()
        } else {
            format!("/{}", path_str)
        };

        // Check if path is under /workspace
        if abs_path == Self::WORKSPACE_PREFIX {
            // Root of workspace maps to root of session store
            Some("/".to_string())
        } else if let Some(stripped) = abs_path.strip_prefix(Self::WORKSPACE_PREFIX) {
            // /workspace/foo -> /foo
            if stripped.starts_with('/') {
                Some(stripped.to_string())
            } else {
                // /workspacefoo is not valid
                None
            }
        } else {
            // Path is outside /workspace
            None
        }
    }
}

#[async_trait]
impl FileSystem for SessionFileSystemAdapter {
    async fn read_file(&self, path: &Path) -> bashkit::Result<Vec<u8>> {
        let session_path = Self::to_session_path(path).ok_or_else(|| {
            bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Path not in workspace: {}", path.display()),
            ))
        })?;

        match self.store.read_file(self.session_id, &session_path).await {
            Ok(Some(file)) => {
                let content = file.content.unwrap_or_default();
                SessionFile::decode_content(&content, &file.encoding)
                    .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
            }
            Ok(None) => Err(bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("File not found: {}", path.display()),
            ))),
            Err(e) => Err(bashkit::Error::Io(std::io::Error::other(e.to_string()))),
        }
    }

    async fn write_file(&self, path: &Path, content: &[u8]) -> bashkit::Result<()> {
        let session_path = Self::to_session_path(path).ok_or_else(|| {
            bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Cannot write outside workspace: {}", path.display()),
            ))
        })?;

        let (encoded, encoding) = SessionFile::encode_content(content);

        self.store
            .write_file(self.session_id, &session_path, &encoded, &encoding)
            .await
            .map(|_| ())
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
    }

    async fn append_file(&self, path: &Path, content: &[u8]) -> bashkit::Result<()> {
        let session_path = Self::to_session_path(path).ok_or_else(|| {
            bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Cannot write outside workspace: {}", path.display()),
            ))
        })?;

        // Read existing content
        let mut existing = match self.store.read_file(self.session_id, &session_path).await {
            Ok(Some(file)) => {
                let content = file.content.unwrap_or_default();
                SessionFile::decode_content(&content, &file.encoding)
                    .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))?
            }
            Ok(None) => Vec::new(),
            Err(e) => return Err(bashkit::Error::Io(std::io::Error::other(e.to_string()))),
        };

        // Append new content
        existing.extend_from_slice(content);

        // Write back
        let (encoded, encoding) = SessionFile::encode_content(&existing);
        self.store
            .write_file(self.session_id, &session_path, &encoded, &encoding)
            .await
            .map(|_| ())
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
    }

    async fn mkdir(&self, path: &Path, _recursive: bool) -> bashkit::Result<()> {
        let session_path = Self::to_session_path(path).ok_or_else(|| {
            bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!(
                    "Cannot create directory outside workspace: {}",
                    path.display()
                ),
            ))
        })?;

        self.store
            .create_directory(self.session_id, &session_path)
            .await
            .map(|_| ())
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
    }

    async fn remove(&self, path: &Path, recursive: bool) -> bashkit::Result<()> {
        let session_path = Self::to_session_path(path).ok_or_else(|| {
            bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                format!("Cannot delete outside workspace: {}", path.display()),
            ))
        })?;

        self.store
            .delete_file(self.session_id, &session_path, recursive)
            .await
            .map(|_| ())
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
    }

    async fn stat(&self, path: &Path) -> bashkit::Result<Metadata> {
        // Handle /workspace itself
        if path.to_string_lossy() == "/workspace" {
            let now = SystemTime::now();
            return Ok(Metadata {
                file_type: FileType::Directory,
                size: 0,
                mode: 0o755,
                modified: now,
                created: now,
            });
        }

        let session_path = Self::to_session_path(path).ok_or_else(|| {
            bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Path not in workspace: {}", path.display()),
            ))
        })?;

        // Check if it's a file
        match self.store.read_file(self.session_id, &session_path).await {
            Ok(Some(file)) => {
                let content = file.content.unwrap_or_default();
                let size = content.len() as u64;
                let now = SystemTime::now();

                Ok(Metadata {
                    file_type: FileType::File,
                    size,
                    mode: 0o644,
                    modified: now,
                    created: now,
                })
            }
            Ok(None) => {
                // Check if it's a directory by listing it
                match self
                    .store
                    .list_directory(self.session_id, &session_path)
                    .await
                {
                    Ok(_entries) => {
                        let now = SystemTime::now();
                        Ok(Metadata {
                            file_type: FileType::Directory,
                            size: 0,
                            mode: 0o755,
                            modified: now,
                            created: now,
                        })
                    }
                    Err(_) => Err(bashkit::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        format!("Path not found: {}", path.display()),
                    ))),
                }
            }
            Err(e) => Err(bashkit::Error::Io(std::io::Error::other(e.to_string()))),
        }
    }

    async fn read_dir(&self, path: &Path) -> bashkit::Result<Vec<DirEntry>> {
        let session_path = Self::to_session_path(path).ok_or_else(|| {
            bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Path not in workspace: {}", path.display()),
            ))
        })?;

        let entries = self
            .store
            .list_directory(self.session_id, &session_path)
            .await
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))?;

        let now = SystemTime::now();

        Ok(entries
            .into_iter()
            .map(|e| {
                let file_type = if e.is_directory {
                    FileType::Directory
                } else {
                    FileType::File
                };

                DirEntry {
                    name: e.name,
                    metadata: Metadata {
                        file_type,
                        size: e.size_bytes as u64,
                        mode: if e.is_directory { 0o755 } else { 0o644 },
                        modified: now,
                        created: now,
                    },
                }
            })
            .collect())
    }

    async fn exists(&self, path: &Path) -> bashkit::Result<bool> {
        // /workspace always exists
        if path.to_string_lossy() == "/workspace" {
            return Ok(true);
        }

        let session_path = match Self::to_session_path(path) {
            Some(p) => p,
            None => return Ok(false), // Paths outside workspace don't exist
        };

        // Check file
        if let Ok(Some(_)) = self.store.read_file(self.session_id, &session_path).await {
            return Ok(true);
        }

        // Check directory
        if self
            .store
            .list_directory(self.session_id, &session_path)
            .await
            .is_ok()
        {
            return Ok(true);
        }

        Ok(false)
    }

    async fn rename(&self, from: &Path, to: &Path) -> bashkit::Result<()> {
        let from_session = Self::to_session_path(from).ok_or_else(|| {
            bashkit::Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Source not in workspace: {}", from.display()),
            ))
        })?;

        // Read source file
        let content = self.read_file(from).await?;

        // Write to destination
        self.write_file(to, &content).await?;

        // Delete source
        self.store
            .delete_file(self.session_id, &from_session, false)
            .await
            .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))?;

        Ok(())
    }

    async fn copy(&self, from: &Path, to: &Path) -> bashkit::Result<()> {
        let content = self.read_file(from).await?;
        self.write_file(to, &content).await
    }

    async fn symlink(&self, _target: &Path, _link: &Path) -> bashkit::Result<()> {
        // Session filesystem doesn't support symlinks
        Err(bashkit::Error::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Symlinks not supported in session filesystem",
        )))
    }

    async fn read_link(&self, path: &Path) -> bashkit::Result<PathBuf> {
        // Session filesystem doesn't support symlinks
        Err(bashkit::Error::Io(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            format!("Symlinks not supported: {}", path.display()),
        )))
    }

    async fn chmod(&self, _path: &Path, _mode: u32) -> bashkit::Result<()> {
        // chmod is a no-op - session filesystem doesn't track permissions
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_id::SessionId;

    #[test]
    fn test_capability_metadata() {
        let cap = VirtualBashCapability;
        assert_eq!(cap.id(), "virtual_bash");
        assert_eq!(cap.name(), "Virtual Bash");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("terminal"));
        assert_eq!(cap.category(), Some("Execution"));
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = VirtualBashCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "bash");
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = VirtualBashCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("bash"));
        assert!(prompt.contains("pipes"));
        assert!(prompt.contains("/workspace"));
    }

    #[test]
    fn test_capability_has_dependencies() {
        let cap = VirtualBashCapability;
        let deps = cap.dependencies();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0], "session_file_system");
    }

    #[test]
    fn test_tool_requires_context() {
        assert!(BashTool.requires_context());
    }

    #[tokio::test]
    async fn test_bash_without_context() {
        let tool = BashTool;
        let result = tool.execute(json!({"command": "echo hello"})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[tokio::test]
    async fn test_bash_missing_command() {
        let tool = BashTool;
        let context = ToolContext::new(SessionId::new());

        let result = tool.execute_with_context(json!({}), &context).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("Missing required parameter"));
        } else {
            panic!("Expected tool error for missing command");
        }
    }

    #[tokio::test]
    async fn test_bash_no_file_store() {
        let tool = BashTool;
        let context = ToolContext::new(SessionId::new());

        let result = tool
            .execute_with_context(json!({"command": "echo hello"}), &context)
            .await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("not available"));
        } else {
            panic!("Expected tool error for missing file store");
        }
    }
}
