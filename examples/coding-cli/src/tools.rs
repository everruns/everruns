// Real-filesystem tools for the coding CLI.
// Decision: a coding agent must touch the user's actual project files, so these
// tools operate on disk under a workspace root rather than the runtime's
// in-memory VFS. Path traversal outside the root is rejected.

use async_trait::async_trait;
use everruns_core::tool_types::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{Value, json};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Shared workspace context for all tools.
#[derive(Clone)]
pub struct Workspace {
    root: Arc<PathBuf>,
}

impl Workspace {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root: Arc::new(root),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve a user-supplied path against the workspace root and ensure it
    /// stays inside the root. Relative paths are joined onto the root.
    fn resolve(&self, raw: &str) -> Result<PathBuf, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("path must not be empty".into());
        }

        let candidate = if Path::new(trimmed).is_absolute() {
            PathBuf::from(trimmed)
        } else {
            self.root.join(trimmed)
        };

        let normalized = normalize(&candidate);
        let root_norm = normalize(self.root.as_path());
        if !normalized.starts_with(&root_norm) {
            return Err(format!(
                "path escapes workspace root: {} (root: {})",
                normalized.display(),
                root_norm.display()
            ));
        }
        Ok(normalized)
    }

    fn display(&self, abs: &Path) -> String {
        match abs.strip_prefix(self.root.as_path()) {
            Ok(rel) => {
                let s = rel.display().to_string();
                if s.is_empty() { ".".to_string() } else { s }
            }
            Err(_) => abs.display().to_string(),
        }
    }
}

fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

// ---------- read_file ----------

pub struct ReadFileTool {
    ws: Workspace,
}
impl ReadFileTool {
    pub fn new(ws: Workspace) -> Self {
        Self { ws }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }
    fn display_name(&self) -> Option<&str> {
        Some("Read File")
    }
    fn description(&self) -> &str {
        "Read a text file from the workspace. Returns up to `limit` lines starting at `offset` (1-based)."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path relative to the workspace root."},
                "offset": {"type": "integer", "description": "1-based line to start at (default 1).", "minimum": 1},
                "limit": {"type": "integer", "description": "Maximum number of lines to return (default 2000).", "minimum": 1}
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }
    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let path = match arguments.get("path").and_then(Value::as_str) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("'path' is required"),
        };
        let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(1) as usize;
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(2000) as usize;

        let resolved = match self.ws.resolve(path) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::tool_error(format!("read failed: {e}")),
        };
        let total_lines = content.lines().count();
        let start = offset.saturating_sub(1);
        let selected: Vec<&str> = content.lines().skip(start).take(limit).collect();
        let truncated = start + selected.len() < total_lines;
        let body = selected.join("\n");
        ToolExecutionResult::success(json!({
            "path": self.ws.display(&resolved),
            "offset": offset,
            "lines_returned": selected.len(),
            "total_lines": total_lines,
            "truncated": truncated,
            "content": body,
        }))
    }
}

// ---------- write_file ----------

pub struct WriteFileTool {
    ws: Workspace,
}
impl WriteFileTool {
    pub fn new(ws: Workspace) -> Self {
        Self { ws }
    }
}

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }
    fn display_name(&self) -> Option<&str> {
        Some("Write File")
    }
    fn description(&self) -> &str {
        "Create or overwrite a file with the given content. Parent directories are created as needed."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "content": {"type": "string"}
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let path = match arguments.get("path").and_then(Value::as_str) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("'path' is required"),
        };
        let content = match arguments.get("content").and_then(Value::as_str) {
            Some(c) => c,
            None => return ToolExecutionResult::tool_error("'content' is required"),
        };
        let resolved = match self.ws.resolve(path) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        if let Some(parent) = resolved.parent()
            && let Err(e) = tokio::fs::create_dir_all(parent).await
        {
            return ToolExecutionResult::tool_error(format!("mkdir failed: {e}"));
        }
        if let Err(e) = tokio::fs::write(&resolved, content).await {
            return ToolExecutionResult::tool_error(format!("write failed: {e}"));
        }
        ToolExecutionResult::success(json!({
            "path": self.ws.display(&resolved),
            "bytes_written": content.len(),
        }))
    }
}

// ---------- edit_file ----------

pub struct EditFileTool {
    ws: Workspace,
}
impl EditFileTool {
    pub fn new(ws: Workspace) -> Self {
        Self { ws }
    }
}

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn display_name(&self) -> Option<&str> {
        Some("Edit File")
    }
    fn description(&self) -> &str {
        "Apply a single string replacement to an existing file. `old_string` must occur exactly once unless `replace_all` is true."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "old_string": {"type": "string"},
                "new_string": {"type": "string"},
                "replace_all": {"type": "boolean", "default": false}
            },
            "required": ["path", "old_string", "new_string"],
            "additionalProperties": false
        })
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let path = match arguments.get("path").and_then(Value::as_str) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("'path' is required"),
        };
        let old_string = match arguments.get("old_string").and_then(Value::as_str) {
            Some(s) => s,
            None => return ToolExecutionResult::tool_error("'old_string' is required"),
        };
        let new_string = match arguments.get("new_string").and_then(Value::as_str) {
            Some(s) => s,
            None => return ToolExecutionResult::tool_error("'new_string' is required"),
        };
        let replace_all = arguments
            .get("replace_all")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if old_string.is_empty() {
            return ToolExecutionResult::tool_error("'old_string' must not be empty");
        }
        let resolved = match self.ws.resolve(path) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        let original = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::tool_error(format!("read failed: {e}")),
        };
        let occurrences = original.matches(old_string).count();
        if occurrences == 0 {
            return ToolExecutionResult::tool_error(
                "old_string not found in file; the snippet must match verbatim",
            );
        }
        if !replace_all && occurrences > 1 {
            return ToolExecutionResult::tool_error(format!(
                "old_string matches {occurrences} times; pass replace_all=true or include more context"
            ));
        }
        let updated = if replace_all {
            original.replace(old_string, new_string)
        } else {
            original.replacen(old_string, new_string, 1)
        };
        if let Err(e) = tokio::fs::write(&resolved, &updated).await {
            return ToolExecutionResult::tool_error(format!("write failed: {e}"));
        }
        ToolExecutionResult::success(json!({
            "path": self.ws.display(&resolved),
            "replacements": if replace_all { occurrences } else { 1 },
        }))
    }
}

// ---------- list_directory ----------

pub struct ListDirectoryTool {
    ws: Workspace,
}
impl ListDirectoryTool {
    pub fn new(ws: Workspace) -> Self {
        Self { ws }
    }
}

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }
    fn display_name(&self) -> Option<&str> {
        Some("List Directory")
    }
    fn description(&self) -> &str {
        "List entries in a directory (non-recursive). Use `path` of '.' for the workspace root."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "default": "."}
            },
            "additionalProperties": false
        })
    }
    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let resolved = match self.ws.resolve(path) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        let mut entries = match tokio::fs::read_dir(&resolved).await {
            Ok(rd) => rd,
            Err(e) => return ToolExecutionResult::tool_error(format!("list failed: {e}")),
        };
        let mut items = Vec::new();
        while let Ok(Some(ent)) = entries.next_entry().await {
            let name = ent.file_name().to_string_lossy().to_string();
            let ftype = ent.file_type().await.ok();
            items.push(json!({
                "name": name,
                "kind": match ftype {
                    Some(t) if t.is_dir() => "dir",
                    Some(t) if t.is_symlink() => "symlink",
                    Some(_) => "file",
                    None => "unknown",
                },
            }));
        }
        items.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });
        ToolExecutionResult::success(json!({
            "path": self.ws.display(&resolved),
            "entries": items,
        }))
    }
}

// ---------- grep ----------

pub struct GrepTool {
    ws: Workspace,
}
impl GrepTool {
    pub fn new(ws: Workspace) -> Self {
        Self { ws }
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }
    fn display_name(&self) -> Option<&str> {
        Some("Grep")
    }
    fn description(&self) -> &str {
        "Recursively search the workspace for a regex pattern. Honors .gitignore. Returns up to `limit` matches."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {"type": "string", "description": "Rust regex pattern."},
                "path": {"type": "string", "description": "Subdirectory under the workspace.", "default": "."},
                "limit": {"type": "integer", "default": 200, "minimum": 1}
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }
    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let pattern = match arguments.get("pattern").and_then(Value::as_str) {
            Some(p) => p,
            None => return ToolExecutionResult::tool_error("'pattern' is required"),
        };
        let path = arguments.get("path").and_then(Value::as_str).unwrap_or(".");
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(200) as usize;

        let re = match Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return ToolExecutionResult::tool_error(format!("invalid regex: {e}")),
        };
        let resolved = match self.ws.resolve(path) {
            Ok(p) => p,
            Err(e) => return ToolExecutionResult::tool_error(e),
        };
        let ws = self.ws.clone();
        let matches = tokio::task::spawn_blocking(move || {
            let mut out: Vec<Value> = Vec::new();
            let walker = WalkBuilder::new(&resolved)
                .hidden(false)
                .git_ignore(true)
                .build();
            for entry in walker.flatten() {
                if out.len() >= limit {
                    break;
                }
                let p = entry.path();
                if !p.is_file() {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(p) else {
                    continue;
                };
                for (idx, line) in text.lines().enumerate() {
                    if re.is_match(line) {
                        out.push(json!({
                            "path": ws.display(p),
                            "line_number": idx + 1,
                            "line": line.chars().take(400).collect::<String>(),
                        }));
                        if out.len() >= limit {
                            break;
                        }
                    }
                }
            }
            out
        })
        .await
        .unwrap_or_default();

        let truncated = matches.len() >= limit;
        ToolExecutionResult::success(json!({
            "pattern": pattern,
            "match_count": matches.len(),
            "truncated": truncated,
            "matches": matches,
        }))
    }
}

// ---------- bash ----------

pub struct BashTool {
    ws: Workspace,
    timeout_secs: u64,
    max_output_bytes: usize,
}
impl BashTool {
    pub fn new(ws: Workspace) -> Self {
        Self {
            ws,
            timeout_secs: 120,
            max_output_bytes: 64 * 1024,
        }
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }
    fn display_name(&self) -> Option<&str> {
        Some("Bash")
    }
    fn description(&self) -> &str {
        "Run a bash command from the workspace root. Captures stdout/stderr (truncated). 120s timeout."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Shell command to run via bash -lc."}
            },
            "required": ["command"],
            "additionalProperties": false
        })
    }
    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let command = match arguments.get("command").and_then(Value::as_str) {
            Some(c) => c.to_string(),
            None => return ToolExecutionResult::tool_error("'command' is required"),
        };
        let root = self.ws.root().to_path_buf();
        let timeout = std::time::Duration::from_secs(self.timeout_secs);
        let max_bytes = self.max_output_bytes;

        let mut child = match Command::new("bash")
            .arg("-lc")
            .arg(&command)
            .current_dir(&root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return ToolExecutionResult::tool_error(format!("spawn failed: {e}")),
        };
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();

        let run = async {
            let mut out_buf = Vec::with_capacity(4096);
            let mut err_buf = Vec::with_capacity(4096);
            let mut o = vec![0u8; 4096];
            let mut e = vec![0u8; 4096];
            loop {
                tokio::select! {
                    n = stdout.read(&mut o) => match n {
                        Ok(0) => break,
                        Ok(n) => out_buf.extend_from_slice(&o[..n]),
                        Err(_) => break,
                    },
                    n = stderr.read(&mut e) => match n {
                        Ok(0) => {},
                        Ok(n) => err_buf.extend_from_slice(&e[..n]),
                        Err(_) => {},
                    },
                }
                if out_buf.len() + err_buf.len() > max_bytes * 2 {
                    let _ = child.start_kill();
                    break;
                }
            }
            while let Ok(n) = stderr.read(&mut e).await {
                if n == 0 {
                    break;
                }
                err_buf.extend_from_slice(&e[..n]);
                if err_buf.len() > max_bytes {
                    break;
                }
            }
            let status = child.wait().await;
            (status, out_buf, err_buf)
        };
        let (status, mut out_buf, mut err_buf) = match tokio::time::timeout(timeout, run).await {
            Ok(r) => r,
            Err(_) => {
                return ToolExecutionResult::tool_error(format!(
                    "command timed out after {}s",
                    self.timeout_secs
                ));
            }
        };
        let out_truncated = out_buf.len() > max_bytes;
        if out_truncated {
            out_buf.truncate(max_bytes);
        }
        let err_truncated = err_buf.len() > max_bytes;
        if err_truncated {
            err_buf.truncate(max_bytes);
        }
        let stdout_text = String::from_utf8_lossy(&out_buf).to_string();
        let stderr_text = String::from_utf8_lossy(&err_buf).to_string();
        let exit_code = status.ok().and_then(|s| s.code());
        ToolExecutionResult::success(json!({
            "command": command,
            "exit_code": exit_code,
            "stdout": stdout_text,
            "stderr": stderr_text,
            "stdout_truncated": out_truncated,
            "stderr_truncated": err_truncated,
        }))
    }
}
