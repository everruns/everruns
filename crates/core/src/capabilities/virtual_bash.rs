//! Virtual Bash Capability
//!
//! This capability provides a sandboxed bash interpreter using bashkit.
//! The bash environment uses an in-memory filesystem that syncs with
//! the session's virtual filesystem before/after execution.
//!
//! Design decisions:
//! - Uses bashkit's InMemoryFs for sandboxed bash execution
//! - Session files are loaded into InMemoryFs before execution
//! - Changes are synced back to session store after execution
//! - Resource limits prevent runaway scripts (max commands, loop iterations)
//! - Context-aware tool that requires session filesystem access

use super::{Capability, CapabilityStatus};
use crate::session_file::SessionFile;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{SessionFileStore, ToolContext};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use bashkit::{Bash, ExecutionLimits, FileSystem, InMemoryFs};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;

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

**Session Filesystem:**
- The session filesystem is mounted at `/`
- Files you create with `write_file` are accessible in bash
- Files created in bash are accessible via `read_file`
- Working directory starts at `/`

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
                    "default": "/",
                    "description": "Working directory for command execution (default: '/')"
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
            .unwrap_or("/");

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

        // Create in-memory filesystem and sync session files
        let mem_fs = Arc::new(InMemoryFs::new());

        // Load session files into in-memory fs
        if let Err(e) = sync_session_to_memfs(context.session_id, &file_store, &mem_fs, "/").await {
            return ToolExecutionResult::tool_error(format!("Failed to load session files: {}", e));
        }

        // Configure bash with resource limits
        let limits = ExecutionLimits::new()
            .max_commands(1000)
            .max_loop_iterations(10000)
            .max_function_depth(100);

        let mut bash = Bash::builder()
            .fs(mem_fs.clone())
            .cwd(working_dir)
            .env("HOME", "/")
            .env("USER", "agent")
            .env("SHELL", "/bin/bash")
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .limits(limits)
            .build();

        // Execute with timeout
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            bash.exec(command),
        )
        .await;

        // Sync changes back to session store (even on error, to capture partial writes)
        if let Err(e) = sync_memfs_to_session(context.session_id, &file_store, &mem_fs, "/").await {
            tracing::warn!("Failed to sync changes back to session: {}", e);
        }

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
// Filesystem Sync Functions
// ============================================================================

/// Load all session files into the in-memory filesystem
async fn sync_session_to_memfs(
    session_id: SessionId,
    store: &Arc<dyn SessionFileStore>,
    mem_fs: &Arc<InMemoryFs>,
    path: &str,
) -> Result<(), String> {
    // List directory contents
    let entries = store
        .list_directory(session_id, path)
        .await
        .map_err(|e| e.to_string())?;

    for entry in entries {
        let entry_path = if path == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", path, entry.name)
        };

        if entry.is_directory {
            // Create directory in memfs
            mem_fs
                .mkdir(Path::new(&entry_path), true)
                .await
                .map_err(|e| e.to_string())?;

            // Recursively sync directory contents
            Box::pin(sync_session_to_memfs(
                session_id,
                store,
                mem_fs,
                &entry_path,
            ))
            .await?;
        } else {
            // Read file from session store
            if let Some(file) = store
                .read_file(session_id, &entry_path)
                .await
                .map_err(|e| e.to_string())?
            {
                let content = file.content.unwrap_or_default();
                let bytes = SessionFile::decode_content(&content, &file.encoding)
                    .map_err(|e| e.to_string())?;

                // Ensure parent directory exists
                if let Some(parent) = Path::new(&entry_path).parent()
                    && parent.as_os_str() != "/"
                {
                    let _ = mem_fs.mkdir(parent, true).await;
                }

                // Write file to memfs
                mem_fs
                    .write_file(Path::new(&entry_path), &bytes)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

/// Sync changes from in-memory filesystem back to session store
async fn sync_memfs_to_session(
    session_id: SessionId,
    store: &Arc<dyn SessionFileStore>,
    mem_fs: &Arc<InMemoryFs>,
    path: &str,
) -> Result<(), String> {
    // Read directory contents from memfs
    let entries = mem_fs
        .read_dir(Path::new(path))
        .await
        .map_err(|e| e.to_string())?;

    for entry in entries {
        let entry_path = if path == "/" {
            format!("/{}", entry.name)
        } else {
            format!("{}/{}", path, entry.name)
        };

        // Check if it's a directory
        let stat = mem_fs
            .stat(Path::new(&entry_path))
            .await
            .map_err(|e| e.to_string())?;

        if stat.file_type.is_dir() {
            // Ensure directory exists in session store
            let _ = store.create_directory(session_id, &entry_path).await;

            // Recursively sync directory contents
            Box::pin(sync_memfs_to_session(
                session_id,
                store,
                mem_fs,
                &entry_path,
            ))
            .await?;
        } else {
            // Read file from memfs
            let bytes = mem_fs
                .read_file(Path::new(&entry_path))
                .await
                .map_err(|e| e.to_string())?;

            // Encode and write to session store
            let (content, encoding) = SessionFile::encode_content(&bytes);
            store
                .write_file(session_id, &entry_path, &content, &encoding)
                .await
                .map_err(|e| e.to_string())?;
        }
    }

    Ok(())
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
        assert!(prompt.contains("session filesystem"));
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
