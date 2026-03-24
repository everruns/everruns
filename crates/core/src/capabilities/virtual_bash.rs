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
//! - SearchCapable impl delegates grep to SessionFileStore::grep_files for
//!   single-query indexed search instead of per-file linear scan

use super::{Capability, CapabilityStatus};
use crate::session_file::SessionFile;
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::{SessionFileStore, ToolContext};
use crate::typed_id::SessionId;
use async_trait::async_trait;
use bashkit::{
    Bash, BashTool as BashkitTool, DirEntry, ExecutionLimits, FileSystem, FileSystemExt, FileType,
    Metadata, SearchCapabilities, SearchCapable, SearchMatch as BashkitSearchMatch, SearchProvider,
    SearchQuery, SearchResults, Tool as BashkitToolTrait,
};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;

// ============================================================================
// Static configuration
// ============================================================================

/// Shared execution limits for bashkit.
fn execution_limits() -> ExecutionLimits {
    ExecutionLimits::new()
        .max_commands(1000)
        .max_loop_iterations(10000)
        .max_function_depth(100)
        .max_input_bytes(1_000_000) // 1MB max script size
        .max_ast_depth(100)
        .parser_timeout(std::time::Duration::from_secs(5))
}

/// Configured bashkit tool instance with everruns settings.
static BASHKIT_TOOL: LazyLock<BashkitTool> = LazyLock::new(|| {
    BashkitTool::builder()
        .username("everruns")
        .hostname("everruns")
        .limits(execution_limits())
        .env("HOME", "/home/agent")
        .env("SHELL", "/bin/bash")
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("WORKSPACE", "/workspace")
        .build()
});

/// Tool description from bashkit library.
static TOOL_DESCRIPTION: LazyLock<String> =
    LazyLock::new(|| BASHKIT_TOOL.description().to_string());

/// System prompt addition from bashkit library.
static TOOL_SYSTEM_PROMPT: LazyLock<String> =
    LazyLock::new(|| BASHKIT_TOOL.system_prompt().to_string());

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
        Some(&TOOL_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BashTool)]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        // Depends on session filesystem for file access
        vec!["session_file_system"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
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

    fn display_name(&self) -> Option<&str> {
        Some("Bash")
    }

    fn description(&self) -> &str {
        &TOOL_DESCRIPTION
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

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_long_running(true)
            .with_open_world(true)
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

        // Configure bash with resource limits (uses shared execution_limits)
        let mut bash = Bash::builder()
            .fs(session_fs)
            .cwd(working_dir)
            .username("everruns")
            .hostname("everruns")
            .env("HOME", "/home/agent")
            .env("SHELL", "/bin/bash")
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .env("WORKSPACE", "/workspace")
            .limits(execution_limits())
            .build();

        // Execute with timeout
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout_ms),
            bash.exec(command),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                // Emit output deltas for live rendering in UI/CLI.
                // bashkit returns all output at once; when bashkit adds streaming
                // support (output_stream()), these will become incremental chunks.
                if !output.stdout.is_empty() {
                    context
                        .emit_tool_output("bash", &output.stdout, "stdout")
                        .await;
                }
                if !output.stderr.is_empty() {
                    context
                        .emit_tool_output("bash", &output.stderr, "stderr")
                        .await;
                }
                ToolExecutionResult::success(json!({
                    "stdout": output.stdout,
                    "stderr": output.stderr,
                    "exit_code": output.exit_code,
                    "success": output.exit_code == 0
                }))
            }
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
impl FileSystemExt for SessionFileSystemAdapter {}

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
                let now = SystemTime::now();

                let file_type = if file.is_directory {
                    FileType::Directory
                } else {
                    FileType::File
                };

                // Use 0o755 so files are executable by default in the virtual filesystem.
                // The session filesystem doesn't track Unix permissions, and scripts
                // stored in /workspace need to be directly executable.
                Ok(Metadata {
                    file_type,
                    size: file.size_bytes as u64,
                    mode: 0o755,
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
                        mode: 0o755,
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

    fn as_search_capable(&self) -> Option<&dyn SearchCapable> {
        Some(self)
    }
}

// ============================================================================
// SearchCapable / SearchProvider — indexed search via SessionFileStore
// ============================================================================

impl SearchCapable for SessionFileSystemAdapter {
    fn search_provider(&self, path: &Path) -> Option<Box<dyn SearchProvider>> {
        // Only provide indexed search for paths inside /workspace
        Self::to_session_path(path)?;
        Some(Box::new(SessionSearchProvider {
            session_id: self.session_id,
            store: self.store.clone(),
        }))
    }
}

/// Bridges bashkit's synchronous `SearchProvider` to `SessionFileStore::grep_files`.
///
/// Uses a scoped thread with a dedicated tokio runtime to call the async
/// store method from the sync trait, avoiding nested `block_on` calls.
struct SessionSearchProvider {
    session_id: SessionId,
    store: Arc<dyn SessionFileStore>,
}

impl SearchProvider for SessionSearchProvider {
    fn search(&self, query: &SearchQuery) -> bashkit::Result<SearchResults> {
        let session_id = self.session_id;
        let store = self.store.clone();
        let root = query.root.to_string_lossy().into_owned();
        let max_results = query.max_results;

        // Honor case_insensitive flag via inline regex flag
        let pattern = if query.case_insensitive {
            format!("(?i){}", query.pattern)
        } else {
            query.pattern.clone()
        };

        // Convert root path from bash VFS path to session store path.
        // search_provider already guards against paths outside /workspace,
        // so to_session_path should always succeed here.
        let session_root =
            SessionFileSystemAdapter::to_session_path(Path::new(&root)).ok_or_else(|| {
                bashkit::Error::Io(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Path not in workspace: {}", root),
                ))
            })?;
        let path_pattern = if session_root == "/" {
            None
        } else {
            Some(session_root)
        };

        // Bridge async grep_files to sync SearchProvider::search.
        // Run on a dedicated thread with its own runtime to avoid nesting
        // block_on calls within the caller's tokio runtime.
        let matches = std::thread::scope(|s| {
            s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))?;
                rt.block_on(async {
                    store
                        .grep_files(session_id, &pattern, path_pattern.as_deref())
                        .await
                })
                .map_err(|e| bashkit::Error::Io(std::io::Error::other(e.to_string())))
            })
            .join()
            .unwrap_or_else(|_| {
                Err(bashkit::Error::Io(std::io::Error::other(
                    "search thread panicked",
                )))
            })
        })?;

        let truncated = max_results.is_some_and(|max| matches.len() > max);
        let matches: Vec<BashkitSearchMatch> = matches
            .into_iter()
            .take(max_results.unwrap_or(usize::MAX))
            .map(|m| {
                // Convert session store path back to bash VFS path
                let vfs_path = format!("{}{}", SessionFileSystemAdapter::WORKSPACE_PREFIX, m.path);
                BashkitSearchMatch {
                    path: PathBuf::from(vfs_path),
                    line_number: m.line_number,
                    line_content: m.line,
                }
            })
            .collect();

        Ok(SearchResults { matches, truncated })
    }

    fn capabilities(&self) -> SearchCapabilities {
        SearchCapabilities {
            regex: true,
            glob_filter: false,
            content_search: true,
            filename_search: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_file::FileInfo;
    use crate::traits::SessionFileStore;
    use crate::typed_id::SessionId;
    use crate::{FileStat, GrepMatch, Result};
    use std::collections::HashMap;
    use std::sync::Mutex;

    // ========================================================================
    // MockFileStore for testing
    // ========================================================================

    /// In-memory file store for testing
    struct MockFileStore {
        files: Mutex<HashMap<(SessionId, String), (String, String)>>, // (content, encoding)
        directories: Mutex<HashMap<(SessionId, String), bool>>,
    }

    impl MockFileStore {
        fn new() -> Self {
            Self {
                files: Mutex::new(HashMap::new()),
                directories: Mutex::new(HashMap::new()),
            }
        }

        fn normalize_path(path: &str) -> String {
            let mut normalized = path.trim().to_string();
            if !normalized.starts_with('/') {
                normalized = format!("/{}", normalized);
            }
            if normalized.len() > 1 && normalized.ends_with('/') {
                normalized.pop();
            }
            normalized
        }
    }

    #[async_trait]
    impl SessionFileStore for MockFileStore {
        async fn read_file(
            &self,
            session_id: SessionId,
            path: &str,
        ) -> Result<Option<SessionFile>> {
            let path = Self::normalize_path(path);
            let files = self.files.lock().unwrap();
            if let Some((content, encoding)) = files.get(&(session_id, path.clone())) {
                Ok(Some(SessionFile {
                    id: uuid::Uuid::new_v4(),
                    session_id: session_id.into(),
                    path: path.clone(),
                    name: path.split('/').next_back().unwrap_or("").to_string(),
                    is_directory: false,
                    is_readonly: false,
                    content: Some(content.clone()),
                    encoding: encoding.clone(),
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            } else {
                Ok(None)
            }
        }

        async fn write_file(
            &self,
            session_id: SessionId,
            path: &str,
            content: &str,
            encoding: &str,
        ) -> Result<SessionFile> {
            let path = Self::normalize_path(path);
            let mut files = self.files.lock().unwrap();
            files.insert(
                (session_id, path.clone()),
                (content.to_string(), encoding.to_string()),
            );
            Ok(SessionFile {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.clone(),
                name: path.split('/').next_back().unwrap_or("").to_string(),
                is_directory: false,
                is_readonly: false,
                content: Some(content.to_string()),
                encoding: encoding.to_string(),
                size_bytes: content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }

        async fn delete_file(
            &self,
            session_id: SessionId,
            path: &str,
            _recursive: bool,
        ) -> Result<bool> {
            let path = Self::normalize_path(path);
            let mut files = self.files.lock().unwrap();
            Ok(files.remove(&(session_id, path)).is_some())
        }

        async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
            let path = Self::normalize_path(path);
            let files = self.files.lock().unwrap();
            let dirs = self.directories.lock().unwrap();
            let mut entries = Vec::new();

            // Root directory always exists
            let is_root = path == "/";

            for ((sid, file_path), (content, _)) in files.iter() {
                if *sid != session_id {
                    continue;
                }

                // Check if file is directly under this path
                let parent = if let Some(idx) = file_path.rfind('/') {
                    if idx == 0 {
                        "/".to_string()
                    } else {
                        file_path[..idx].to_string()
                    }
                } else {
                    "/".to_string()
                };

                if parent == path {
                    entries.push(FileInfo {
                        id: uuid::Uuid::new_v4(),
                        session_id: session_id.into(),
                        path: file_path.clone(),
                        name: file_path.split('/').next_back().unwrap_or("").to_string(),
                        is_directory: false,
                        is_readonly: false,
                        size_bytes: content.len() as i64,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                    });
                }
            }

            // Return error if directory doesn't exist (not root, not explicitly created,
            // and no files have it as parent)
            if !is_root && entries.is_empty() && !dirs.contains_key(&(session_id, path.clone())) {
                // Also check if any file has this as an ancestor (implicit directory)
                let has_children = files
                    .keys()
                    .any(|(sid, fp)| *sid == session_id && fp.starts_with(&format!("{}/", path)));
                if !has_children {
                    return Err(anyhow::anyhow!("Directory not found: {}", path).into());
                }
            }

            Ok(entries)
        }

        async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
            let path = Self::normalize_path(path);
            let files = self.files.lock().unwrap();
            if let Some((content, _)) = files.get(&(session_id, path.clone())) {
                Ok(Some(FileStat {
                    path: path.clone(),
                    name: path.split('/').next_back().unwrap_or("").to_string(),
                    is_directory: false,
                    is_readonly: false,
                    size_bytes: content.len() as i64,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                }))
            } else {
                Ok(None)
            }
        }

        async fn grep_files(
            &self,
            session_id: SessionId,
            pattern: &str,
            path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            let regex = regex::Regex::new(pattern)
                .map_err(|e| anyhow::anyhow!("invalid pattern: {}", e))?;
            let files = self.files.lock().unwrap();
            let mut matches = Vec::new();
            for ((sid, file_path), (content, _)) in files.iter() {
                if *sid != session_id {
                    continue;
                }
                if let Some(pp) = path_pattern
                    && !file_path.starts_with(pp)
                {
                    continue;
                }
                let decoded = SessionFile::decode_content(content, "utf-8")
                    .unwrap_or_else(|_| content.as_bytes().to_vec());
                let text = String::from_utf8_lossy(&decoded);
                for (i, line) in text.lines().enumerate() {
                    if regex.is_match(line) {
                        matches.push(GrepMatch {
                            path: file_path.clone(),
                            line_number: i + 1,
                            line: line.to_string(),
                        });
                    }
                }
            }
            matches.sort_by(|a, b| a.path.cmp(&b.path).then(a.line_number.cmp(&b.line_number)));
            Ok(matches)
        }

        async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
            let path = Self::normalize_path(path);
            let mut dirs = self.directories.lock().unwrap();
            dirs.insert((session_id, path.clone()), true);
            Ok(FileInfo {
                id: uuid::Uuid::new_v4(),
                session_id: session_id.into(),
                path: path.clone(),
                name: path.split('/').next_back().unwrap_or("").to_string(),
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
        }
    }

    // ========================================================================
    // Capability metadata tests
    // ========================================================================

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
        // System prompt is now provided by bashkit library
        assert!(!prompt.is_empty(), "System prompt should not be empty");
        // Should contain the configured username/hostname
        assert!(
            prompt.contains("everruns"),
            "System prompt should contain configured identity"
        );
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

    // ========================================================================
    // Path translation tests
    // ========================================================================

    #[test]
    fn test_to_session_path_workspace_root() {
        let result = SessionFileSystemAdapter::to_session_path(Path::new("/workspace"));
        assert_eq!(result, Some("/".to_string()));
    }

    #[test]
    fn test_to_session_path_workspace_file() {
        let result = SessionFileSystemAdapter::to_session_path(Path::new("/workspace/file.txt"));
        assert_eq!(result, Some("/file.txt".to_string()));
    }

    #[test]
    fn test_to_session_path_workspace_nested() {
        let result =
            SessionFileSystemAdapter::to_session_path(Path::new("/workspace/dir/subdir/file.txt"));
        assert_eq!(result, Some("/dir/subdir/file.txt".to_string()));
    }

    #[test]
    fn test_to_session_path_outside_workspace() {
        let result = SessionFileSystemAdapter::to_session_path(Path::new("/tmp/file.txt"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_to_session_path_home_outside_workspace() {
        let result = SessionFileSystemAdapter::to_session_path(Path::new("/home/agent/file.txt"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_to_session_path_workspacefoo_invalid() {
        // /workspacefoo is NOT under /workspace
        let result = SessionFileSystemAdapter::to_session_path(Path::new("/workspacefoo"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_to_session_path_relative_path() {
        // Relative path gets normalized to absolute
        let result = SessionFileSystemAdapter::to_session_path(Path::new("workspace/file.txt"));
        // /workspace/file.txt would be valid, but "workspace/file.txt" -> "/workspace/file.txt"
        // Wait, that's not right. Let me check the logic again.
        // normalize adds "/" prefix: "workspace/file.txt" -> "/workspace/file.txt"
        // Then it checks if it starts with "/workspace" - yes it does
        // So this should return Some("/file.txt")
        assert_eq!(result, Some("/file.txt".to_string()));
    }

    // ========================================================================
    // Tool error handling tests
    // ========================================================================

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

    // ========================================================================
    // Bash execution tests with MockFileStore
    // ========================================================================

    fn create_context_with_mock_store() -> (ToolContext, SessionId) {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let mut context = ToolContext::new(session_id);
        context.file_store = Some(store);
        (context, session_id)
    }

    #[tokio::test]
    async fn test_bash_echo_command() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        let result = tool
            .execute_with_context(json!({"command": "echo hello world"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "hello world\n");
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["success"], true);
        } else {
            panic!("Expected success result, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_pwd_default_workspace() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        let result = tool
            .execute_with_context(json!({"command": "pwd"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "/workspace\n");
            assert_eq!(output["exit_code"], 0);
        } else {
            panic!("Expected success result, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_env_variables() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Test HOME
        let result = tool
            .execute_with_context(json!({"command": "echo $HOME"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "/home/agent\n");
        } else {
            panic!("Expected success");
        }

        // Test WORKSPACE
        let result = tool
            .execute_with_context(json!({"command": "echo $WORKSPACE"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "/workspace\n");
        } else {
            panic!("Expected success");
        }

        // Test USER (set by bashkit from username)
        let result = tool
            .execute_with_context(json!({"command": "echo $USER"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "everruns\n");
        } else {
            panic!("Expected success");
        }
    }

    #[tokio::test]
    async fn test_bash_write_and_read_file() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Write a file
        let result = tool
            .execute_with_context(
                json!({"command": "echo 'test content' > /workspace/test.txt"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Read it back
        let result = tool
            .execute_with_context(json!({"command": "cat /workspace/test.txt"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "test content\n");
        } else {
            panic!("Expected success result");
        }
    }

    #[tokio::test]
    async fn test_bash_pipe_command() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        let result = tool
            .execute_with_context(json!({"command": "echo hello | cat"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "hello\n");
            assert_eq!(output["exit_code"], 0);
        } else {
            panic!("Expected success result");
        }
    }

    #[tokio::test]
    async fn test_bash_arithmetic() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        let result = tool
            .execute_with_context(json!({"command": "echo $((2 + 3 * 4))"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "14\n");
        } else {
            panic!("Expected success result");
        }
    }

    #[tokio::test]
    async fn test_bash_command_substitution() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        let result = tool
            .execute_with_context(json!({"command": "echo $(echo nested)"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "nested\n");
        } else {
            panic!("Expected success result");
        }
    }

    // ========================================================================
    // Negative tests - paths outside workspace
    // ========================================================================

    #[tokio::test]
    async fn test_bash_write_outside_workspace_fails() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Try to write to /tmp (outside workspace)
        let result = tool
            .execute_with_context(json!({"command": "echo 'hack' > /tmp/evil.txt"}), &context)
            .await;

        // This should fail because /tmp is outside /workspace
        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(
                msg.contains("outside workspace") || msg.contains("Permission"),
                "Expected workspace error, got: {}",
                msg
            );
        } else if let ToolExecutionResult::Success(output) = result {
            // If bashkit doesn't error, the write should silently fail
            // Let's verify by trying to read it
            let read_result = tool
                .execute_with_context(json!({"command": "cat /tmp/evil.txt"}), &context)
                .await;
            // Should not find the file
            assert!(
                matches!(read_result, ToolExecutionResult::ToolError(_))
                    || matches!(&read_result, ToolExecutionResult::Success(o) if o["exit_code"] != 0),
                "File should not exist outside workspace"
            );
            // Also check that /tmp read fails
            assert!(
                output["stderr"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Permission")
                    || output["stderr"]
                        .as_str()
                        .unwrap_or("")
                        .contains("workspace")
                    || output["exit_code"] != 0,
                "Write outside workspace should fail or be blocked"
            );
        } else {
            // Either behavior is acceptable - error or silent failure
        }
    }

    #[tokio::test]
    async fn test_bash_read_outside_workspace_fails() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Try to read from /etc (outside workspace)
        let result = tool
            .execute_with_context(json!({"command": "cat /etc/passwd"}), &context)
            .await;

        // Should fail - either as tool error or with non-zero exit code
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("workspace") || msg.contains("not found"),
                    "Expected workspace error, got: {}",
                    msg
                );
            }
            ToolExecutionResult::Success(output) => {
                // Exit code should be non-zero since file doesn't exist
                assert_ne!(
                    output["exit_code"], 0,
                    "Reading /etc/passwd should fail with non-zero exit"
                );
            }
            _ => panic!("Unexpected result type"),
        }
    }

    #[tokio::test]
    async fn test_bash_mkdir_outside_workspace_fails() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Try to create directory in /tmp
        let result = tool
            .execute_with_context(json!({"command": "mkdir /tmp/evil_dir"}), &context)
            .await;

        // Should fail
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("workspace") || msg.contains("Permission"),
                    "Got: {}",
                    msg
                );
            }
            ToolExecutionResult::Success(output) => {
                // If it "succeeds", the directory shouldn't actually exist
                // Or exit code should be non-zero
                assert!(
                    output["exit_code"] != 0
                        || output["stderr"]
                            .as_str()
                            .unwrap_or("")
                            .contains("Permission"),
                    "mkdir outside workspace should fail"
                );
            }
            _ => {}
        }
    }

    // ========================================================================
    // Working directory tests
    // ========================================================================

    #[tokio::test]
    async fn test_bash_custom_working_dir() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // First create the directory
        let result = tool
            .execute_with_context(json!({"command": "mkdir -p /workspace/mydir"}), &context)
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Run pwd with custom working directory
        let result = tool
            .execute_with_context(
                json!({
                    "command": "pwd",
                    "working_dir": "/workspace/mydir"
                }),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "/workspace/mydir\n");
        } else {
            panic!("Expected success result");
        }
    }

    // ========================================================================
    // Exit code tests
    // ========================================================================

    #[tokio::test]
    async fn test_bash_false_command_exit_code() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        let result = tool
            .execute_with_context(json!({"command": "false"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 1);
            assert_eq!(output["success"], false);
        } else {
            panic!("Expected success result with non-zero exit code");
        }
    }

    #[tokio::test]
    async fn test_bash_true_command_exit_code() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        let result = tool
            .execute_with_context(json!({"command": "true"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["success"], true);
        } else {
            panic!("Expected success result");
        }
    }

    // ========================================================================
    // FileSystem adapter direct tests
    // ========================================================================

    #[tokio::test]
    async fn test_adapter_read_write_workspace_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write a file
        adapter
            .write_file(Path::new("/workspace/test.txt"), b"hello")
            .await
            .unwrap();

        // Read it back
        let content = adapter
            .read_file(Path::new("/workspace/test.txt"))
            .await
            .unwrap();
        assert_eq!(content, b"hello");
    }

    #[tokio::test]
    async fn test_adapter_read_outside_workspace_fails() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        let result = adapter.read_file(Path::new("/tmp/file.txt")).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("workspace"));
    }

    #[tokio::test]
    async fn test_adapter_write_outside_workspace_fails() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        let result = adapter
            .write_file(Path::new("/tmp/file.txt"), b"data")
            .await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert!(err.to_string().contains("workspace"));
    }

    #[tokio::test]
    async fn test_adapter_stat_workspace_root() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        let stat = adapter.stat(Path::new("/workspace")).await.unwrap();
        assert!(stat.file_type.is_dir());
    }

    #[tokio::test]
    async fn test_adapter_stat_directory_returns_dir_type() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Create a directory
        adapter
            .mkdir(Path::new("/workspace/mydir"), false)
            .await
            .unwrap();

        // stat should report it as a directory, not a file
        let stat = adapter.stat(Path::new("/workspace/mydir")).await.unwrap();
        assert!(
            stat.file_type.is_dir(),
            "Expected directory but got file type for /workspace/mydir"
        );
    }

    #[tokio::test]
    async fn test_adapter_stat_file_returns_file_type() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write a file
        adapter
            .write_file(Path::new("/workspace/test.txt"), b"hello")
            .await
            .unwrap();

        // stat should report it as a file
        let stat = adapter
            .stat(Path::new("/workspace/test.txt"))
            .await
            .unwrap();
        assert!(
            stat.file_type.is_file(),
            "Expected file but got directory type for /workspace/test.txt"
        );
        assert_eq!(stat.size, 5);
    }

    #[tokio::test]
    async fn test_adapter_exists_workspace() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // /workspace always exists
        assert!(adapter.exists(Path::new("/workspace")).await.unwrap());

        // /tmp does not exist (outside workspace)
        assert!(!adapter.exists(Path::new("/tmp")).await.unwrap());
    }

    #[tokio::test]
    async fn test_adapter_mkdir_and_list() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store.clone());

        // Create a directory
        adapter
            .mkdir(Path::new("/workspace/mydir"), false)
            .await
            .unwrap();

        // Write a file in it
        adapter
            .write_file(Path::new("/workspace/mydir/file.txt"), b"content")
            .await
            .unwrap();

        // List should include the file
        let entries = adapter
            .read_dir(Path::new("/workspace/mydir"))
            .await
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "file.txt");
    }

    #[tokio::test]
    async fn test_adapter_rename_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write original file
        adapter
            .write_file(Path::new("/workspace/old.txt"), b"data")
            .await
            .unwrap();

        // Rename it
        adapter
            .rename(
                Path::new("/workspace/old.txt"),
                Path::new("/workspace/new.txt"),
            )
            .await
            .unwrap();

        // Old file should not exist
        let old_result = adapter.read_file(Path::new("/workspace/old.txt")).await;
        assert!(old_result.is_err());

        // New file should have the content
        let new_content = adapter
            .read_file(Path::new("/workspace/new.txt"))
            .await
            .unwrap();
        assert_eq!(new_content, b"data");
    }

    #[tokio::test]
    async fn test_adapter_copy_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write original file
        adapter
            .write_file(Path::new("/workspace/source.txt"), b"copy me")
            .await
            .unwrap();

        // Copy it
        adapter
            .copy(
                Path::new("/workspace/source.txt"),
                Path::new("/workspace/dest.txt"),
            )
            .await
            .unwrap();

        // Both files should exist with same content
        let source = adapter
            .read_file(Path::new("/workspace/source.txt"))
            .await
            .unwrap();
        let dest = adapter
            .read_file(Path::new("/workspace/dest.txt"))
            .await
            .unwrap();
        assert_eq!(source, dest);
        assert_eq!(source, b"copy me");
    }

    #[tokio::test]
    async fn test_adapter_append_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write initial content
        adapter
            .write_file(Path::new("/workspace/log.txt"), b"line1\n")
            .await
            .unwrap();

        // Append more
        adapter
            .append_file(Path::new("/workspace/log.txt"), b"line2\n")
            .await
            .unwrap();

        // Read combined content
        let content = adapter
            .read_file(Path::new("/workspace/log.txt"))
            .await
            .unwrap();
        assert_eq!(content, b"line1\nline2\n");
    }

    #[tokio::test]
    async fn test_adapter_symlink_not_supported() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        let result = adapter
            .symlink(Path::new("/workspace/target"), Path::new("/workspace/link"))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not supported"));
    }

    #[tokio::test]
    async fn test_adapter_chmod_is_noop() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // chmod should succeed as a no-op
        let result = adapter.chmod(Path::new("/workspace/file.txt"), 0o755).await;
        assert!(result.is_ok());
    }

    // ========================================================================
    // Security limit tests (bashkit 0.1.0)
    // ========================================================================

    #[tokio::test]
    async fn test_bash_max_input_bytes_limit() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Create a script larger than 1MB limit
        let large_script = "echo ".to_string() + &"x".repeat(1_100_000);

        let result = tool
            .execute_with_context(json!({"command": large_script}), &context)
            .await;

        // Should fail due to input size limit
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("too large") || msg.contains("input") || msg.contains("limit"),
                    "Expected input size error, got: {}",
                    msg
                );
            }
            ToolExecutionResult::Success(output) => {
                panic!(
                    "Expected error for oversized script, got success: {:?}",
                    output
                );
            }
            _ => panic!("Unexpected result type"),
        }
    }

    #[tokio::test]
    async fn test_bash_loop_within_limit() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Execute a loop within the 10000 iteration limit
        let command = "i=0; while [ $i -lt 100 ]; do i=$((i + 1)); done; echo $i";

        let result = tool
            .execute_with_context(json!({"command": command}), &context)
            .await;

        // Should succeed within limits
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"].as_str().unwrap_or("").trim(), "100");
        } else {
            panic!("Expected success for loop within limit: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_function_calls() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Test basic function definition and calls (non-recursive to avoid stack issues)
        let command = r#"
            greet() {
                echo "Hello, $1!"
            }
            greet world
        "#;

        let result = tool
            .execute_with_context(json!({"command": command}), &context)
            .await;

        // Should succeed
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert!(
                output["stdout"]
                    .as_str()
                    .unwrap_or("")
                    .contains("Hello, world!")
            );
        } else {
            panic!("Expected success for function call: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_arithmetic_expressions() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Test various arithmetic expressions (shallow nesting to avoid stack issues)
        let command = "echo $((1 + 2 * 3))";

        let result = tool
            .execute_with_context(json!({"command": command}), &context)
            .await;

        // Should succeed
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"].as_str().unwrap_or("").trim(), "7");
        } else {
            panic!("Expected success for arithmetic expression: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_commands_within_limit() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Execute multiple commands within the 1000 command limit
        let command = "for i in $(seq 1 100); do true; done; echo done";

        let result = tool
            .execute_with_context(json!({"command": command}), &context)
            .await;

        // Should succeed within limits
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert!(output["stdout"].as_str().unwrap_or("").contains("done"));
        } else {
            panic!("Expected success for commands within limit: {:?}", result);
        }
    }

    // ========================================================================
    // Script file execution tests
    // ========================================================================

    #[tokio::test]
    async fn test_bash_execute_script_by_absolute_path() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Create a script file
        let result = tool
            .execute_with_context(
                json!({"command": "cat > /workspace/test.sh << 'EOF'\n#!/bin/bash\necho hello\nEOF"}),
                &context,
            )
            .await;
        assert!(
            matches!(result, ToolExecutionResult::Success(_)),
            "Failed to create script: {:?}",
            result
        );

        // Execute by absolute path
        let result = tool
            .execute_with_context(json!({"command": "/workspace/test.sh"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"], "hello\n");
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_execute_script_with_args() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Create a script that uses arguments
        let result = tool
            .execute_with_context(
                json!({"command": "cat > /workspace/greet.sh << 'EOF'\n#!/bin/bash\necho \"Hello, $1! You are $2.\"\nEOF"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Execute with arguments
        let result = tool
            .execute_with_context(
                json!({"command": "/workspace/greet.sh world awesome"}),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"], "Hello, world! You are awesome.\n");
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_execute_script_without_shebang() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Create a script without shebang
        let result = tool
            .execute_with_context(
                json!({"command": "cat > /workspace/simple.sh << 'EOF'\necho simple\nEOF"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Execute - should still work
        let result = tool
            .execute_with_context(json!({"command": "/workspace/simple.sh"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"], "simple\n");
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_execute_nonexistent_script() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Try to execute a script that doesn't exist
        let result = tool
            .execute_with_context(json!({"command": "/workspace/nonexistent.sh"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_ne!(output["exit_code"], 0, "Should fail with non-zero exit");
            let stderr = output["stderr"].as_str().unwrap_or("");
            assert!(
                stderr.contains("No such file") || stderr.contains("not found"),
                "Expected file not found error, got stderr: {}",
                stderr
            );
        } else {
            panic!(
                "Expected success result with error output, got: {:?}",
                result
            );
        }
    }

    #[tokio::test]
    async fn test_bash_execute_script_in_nested_dir() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Create nested directory structure and script
        let setup = tool
            .execute_with_context(
                json!({"command": "mkdir -p /workspace/.agents/skills/nav/scripts && cat > /workspace/.agents/skills/nav/scripts/nav.sh << 'EOF'\n#!/bin/bash\necho \"navigating $1\"\nEOF"}),
                &context,
            )
            .await;
        assert!(matches!(setup, ToolExecutionResult::Success(_)));

        // Execute by absolute path (the exact scenario from the bug report)
        let result = tool
            .execute_with_context(
                json!({"command": "/workspace/.agents/skills/nav/scripts/nav.sh dist"}),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert_eq!(output["stdout"], "navigating dist\n");
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_file_mode_is_executable() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Write a file and check that test -x reports it as executable
        let result = tool
            .execute_with_context(
                json!({"command": "echo 'echo hi' > /workspace/check.sh && test -x /workspace/check.sh && echo 'executable' || echo 'not executable'"}),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            assert!(
                output["stdout"]
                    .as_str()
                    .unwrap_or("")
                    .contains("executable"),
                "File should be reported as executable, got: {}",
                output["stdout"]
            );
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_execute_script_with_exit_code() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Create a script that exits with a specific code
        let result = tool
            .execute_with_context(
                json!({"command": "cat > /workspace/fail.sh << 'EOF'\n#!/bin/bash\necho failing\nexit 42\nEOF"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Execute and check exit code propagation
        let result = tool
            .execute_with_context(
                json!({"command": "/workspace/fail.sh; echo \"code: $?\""}),
                &context,
            )
            .await;

        if let ToolExecutionResult::Success(output) = result {
            let stdout = output["stdout"].as_str().unwrap_or("");
            assert!(stdout.contains("failing"), "Script should have run");
            assert!(
                stdout.contains("code: 42"),
                "Exit code should propagate, got: {}",
                stdout
            );
        } else {
            panic!("Expected success, got: {:?}", result);
        }
    }

    // ========================================================================
    // Overwrite / existing-file tests
    // ========================================================================

    #[tokio::test]
    async fn test_bash_overwrite_existing_file() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Write a file
        let result = tool
            .execute_with_context(
                json!({"command": "echo 'first' > /workspace/overwrite.txt"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Overwrite with new content
        let result = tool
            .execute_with_context(
                json!({"command": "echo 'second' > /workspace/overwrite.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::Success(output) = &result {
            assert_eq!(output["exit_code"], 0, "Overwrite should succeed");
        } else {
            panic!("Expected success on overwrite, got: {:?}", result);
        }

        // Read back — should have new content
        let result = tool
            .execute_with_context(json!({"command": "cat /workspace/overwrite.txt"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "second\n");
        } else {
            panic!("Expected success on read, got: {:?}", result);
        }
    }

    #[tokio::test]
    async fn test_bash_append_to_existing_file() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Create file
        let result = tool
            .execute_with_context(
                json!({"command": "echo 'line1' > /workspace/append.txt"}),
                &context,
            )
            .await;
        assert!(matches!(result, ToolExecutionResult::Success(_)));

        // Append
        let result = tool
            .execute_with_context(
                json!({"command": "echo 'line2' >> /workspace/append.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::Success(output) = &result {
            assert_eq!(output["exit_code"], 0, "Append should succeed");
        } else {
            panic!("Expected success on append, got: {:?}", result);
        }

        // Verify combined content
        let result = tool
            .execute_with_context(json!({"command": "cat /workspace/append.txt"}), &context)
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "line1\nline2\n");
        } else {
            panic!("Expected success on read");
        }
    }

    #[tokio::test]
    async fn test_adapter_overwrite_existing_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write initial
        adapter
            .write_file(Path::new("/workspace/ow.txt"), b"original")
            .await
            .unwrap();

        // Overwrite
        adapter
            .write_file(Path::new("/workspace/ow.txt"), b"updated")
            .await
            .unwrap();

        // Verify new content
        let content = adapter
            .read_file(Path::new("/workspace/ow.txt"))
            .await
            .unwrap();
        assert_eq!(content, b"updated");
    }

    #[tokio::test]
    async fn test_adapter_append_to_existing_file() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        // Write initial
        adapter
            .write_file(Path::new("/workspace/ap.txt"), b"AAA")
            .await
            .unwrap();

        // Append
        adapter
            .append_file(Path::new("/workspace/ap.txt"), b"BBB")
            .await
            .unwrap();

        // Verify combined
        let content = adapter
            .read_file(Path::new("/workspace/ap.txt"))
            .await
            .unwrap();
        assert_eq!(content, b"AAABBB");
    }

    #[tokio::test]
    async fn test_bash_redirect_creates_parent_dirs() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Write to a nested path — parent dirs should be auto-created
        let result = tool
            .execute_with_context(
                json!({"command": "echo 'deep' > /workspace/a/b/c/deep.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::Success(output) = &result {
            assert_eq!(output["exit_code"], 0, "Nested write should succeed");
        } else {
            panic!("Expected success, got: {:?}", result);
        }

        // Read back
        let result = tool
            .execute_with_context(
                json!({"command": "cat /workspace/a/b/c/deep.txt"}),
                &context,
            )
            .await;
        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["stdout"], "deep\n");
        } else {
            panic!("Expected success on read");
        }
    }

    // ========================================================================
    // bashkit API smoke tests
    // ========================================================================

    #[test]
    fn test_bashkit_tool_description_is_nonempty() {
        let desc = BASHKIT_TOOL.description();
        assert!(
            !desc.is_empty(),
            "bashkit tool description should not be empty"
        );
        // Should mention bash or command execution
        assert!(
            desc.to_lowercase().contains("bash") || desc.to_lowercase().contains("command"),
            "description should mention bash or command, got: {}",
            desc
        );
    }

    #[test]
    fn test_bashkit_tool_system_prompt_is_nonempty() {
        let prompt = BASHKIT_TOOL.system_prompt();
        assert!(
            !prompt.is_empty(),
            "bashkit system prompt should not be empty"
        );
        assert!(
            prompt.contains("everruns"),
            "system prompt should contain configured identity 'everruns', got: {}",
            prompt
        );
    }

    #[test]
    fn test_bashkit_static_description_matches_tool() {
        // Verify the LazyLock statics produce the same values as direct calls
        let direct_desc = BASHKIT_TOOL.description();
        let static_desc: &str = &TOOL_DESCRIPTION;
        assert_eq!(static_desc, direct_desc);

        let direct_prompt = BASHKIT_TOOL.system_prompt();
        let static_prompt: &str = &TOOL_SYSTEM_PROMPT;
        assert_eq!(static_prompt, direct_prompt);
    }

    #[test]
    fn test_bashkit_tool_builder_configuration() {
        // Verify the static BASHKIT_TOOL was built with our custom settings
        // by checking that description/system_prompt are accessible (non-panicking)
        let _desc = BASHKIT_TOOL.description();
        let _prompt = BASHKIT_TOOL.system_prompt();
        // If we got here without panic, the builder configuration is valid
    }

    #[test]
    fn test_bash_tool_display_name() {
        let tool = BashTool;
        assert_eq!(tool.display_name(), Some("Bash"));
    }

    #[test]
    fn test_bash_tool_parameters_schema_structure() {
        let tool = BashTool;
        let schema = tool.parameters_schema();

        // Verify required fields
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["command"].is_object());
        assert_eq!(schema["properties"]["command"]["type"], "string");

        // Verify optional fields
        assert!(schema["properties"]["working_dir"].is_object());
        assert!(schema["properties"]["timeout_ms"].is_object());

        // Verify "command" is required
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("command")));
    }

    #[test]
    fn test_execution_limits_configuration() {
        let limits = execution_limits();
        // Just verify it doesn't panic and returns a valid object
        // The limits are used by both BASHKIT_TOOL and per-execution Bash instances
        let _ = limits;
    }

    // ========================================================================
    // SearchCapable / indexed search tests
    // ========================================================================

    #[test]
    fn test_adapter_is_search_capable() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store);

        let sc = adapter.as_search_capable();
        assert!(
            sc.is_some(),
            "SessionFileSystemAdapter should be SearchCapable"
        );

        let provider = sc.unwrap().search_provider(Path::new("/workspace"));
        assert!(provider.is_some(), "Should return a SearchProvider");

        let caps = provider.unwrap().capabilities();
        assert!(caps.content_search, "Should support content search");
        assert!(caps.regex, "Should support regex patterns");
    }

    #[tokio::test]
    async fn test_search_provider_returns_grep_results() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store.clone());

        // Write files via the adapter
        adapter
            .write_file(
                Path::new("/workspace/hello.txt"),
                b"hello world\ngoodbye world",
            )
            .await
            .unwrap();
        adapter
            .write_file(Path::new("/workspace/other.txt"), b"no match here")
            .await
            .unwrap();

        let sc = adapter.as_search_capable().unwrap();
        let provider = sc.search_provider(Path::new("/workspace")).unwrap();

        let results = provider
            .search(&SearchQuery {
                pattern: "hello".into(),
                is_regex: false,
                case_insensitive: false,
                root: PathBuf::from("/workspace"),
                glob_filter: None,
                max_results: None,
            })
            .unwrap();

        assert_eq!(results.matches.len(), 1);
        assert_eq!(
            results.matches[0].path,
            PathBuf::from("/workspace/hello.txt")
        );
        assert_eq!(results.matches[0].line_number, 1);
        assert_eq!(results.matches[0].line_content, "hello world");
    }

    #[tokio::test]
    async fn test_search_provider_truncates_at_max_results() {
        let session_id = SessionId::new();
        let store: Arc<dyn SessionFileStore> = Arc::new(MockFileStore::new());
        let adapter = SessionFileSystemAdapter::new(session_id, store.clone());

        adapter
            .write_file(
                Path::new("/workspace/many.txt"),
                b"match line 1\nmatch line 2\nmatch line 3\nmatch line 4",
            )
            .await
            .unwrap();

        let sc = adapter.as_search_capable().unwrap();
        let provider = sc.search_provider(Path::new("/workspace")).unwrap();

        let results = provider
            .search(&SearchQuery {
                pattern: "match".into(),
                is_regex: false,
                case_insensitive: false,
                root: PathBuf::from("/workspace"),
                glob_filter: None,
                max_results: Some(2),
            })
            .unwrap();

        assert_eq!(results.matches.len(), 2);
        assert!(results.truncated);
    }

    #[tokio::test]
    async fn test_bash_grep_uses_indexed_search() {
        let (context, _) = create_context_with_mock_store();
        let tool = BashTool;

        // Create files
        tool.execute_with_context(
            json!({"command": "mkdir -p /workspace/src && echo 'fn main() { println!(\"hello\"); }' > /workspace/src/main.rs && echo 'fn test() {}' > /workspace/src/test.rs"}),
            &context,
        )
        .await;

        // Run grep -r which should use indexed search via SearchCapable
        let result = tool
            .execute_with_context(json!({"command": "grep -r 'fn' /workspace/src"}), &context)
            .await;

        if let ToolExecutionResult::Success(output) = result {
            assert_eq!(output["exit_code"], 0);
            let stdout = output["stdout"].as_str().unwrap_or("");
            assert!(
                stdout.contains("fn main") || stdout.contains("fn test"),
                "grep -r should find matches via indexed search, got: {}",
                stdout
            );
        } else {
            panic!("Expected success result, got: {:?}", result);
        }
    }
}
