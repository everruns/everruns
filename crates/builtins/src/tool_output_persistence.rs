// Tool Output Persistence Capability (EVE-222, EVE-245, EVE-562)
//
// Persists full exec tool output to session VFS before truncation,
// enabling lossless retrieval via read_file/grep. The LLM gets a
// truncated summary with `full_output` path and `output_files`
// array pointing to the persisted files.
//
// Design decisions:
// - Implemented as PostToolExecHook, not baked into each tool — VFS
//   persistence is a cross-cutting concern the tool shouldn't know about
// - Reads `persist_output` hint from ToolDefinition to decide what to persist
// - Writes stdout to /outputs/{safe_id}.stdout, stderr to
//   /outputs/{safe_id}.stderr when stderr is persisted (session-scoped)
// - Injects `output_files` array into result for agent to read selectively
// - Graceful degradation: skip silently if file_store is unavailable
// - Runs as an always-on final hook before EVE-225 hard-limit truncation
// - EVE-245: annotates truncated stdout with file reference so agent knows
//   it can read_file the full output with offset/limit
// - EVE-562: after persistence, applies the requested output mode to
//   model-facing stdout/stderr. Default auto mode uses a compact success
//   window; explicit modes retain their fixed budgets.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::atoms::PostToolExecHook;
use crate::tool_output_sanitizer::{
    output_verbosity_budget, priority_aware_truncate, resolve_auto_mode, truncate_exec_stream,
};
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::typed_id::SessionId;
use everruns_core::{session_files::SessionFileSystem, tool_context::ToolContext};

/// Max bytes persisted per output stream file to avoid storage exhaustion.
const MAX_PERSISTED_STREAM_BYTES: usize = 1024 * 1024; // 1 MiB
const OUTPUT_DIR: &str = "/outputs";

/// Result of persisting large exec output to session VFS.
pub struct PersistResult {
    /// Path to the persisted stdout file in session VFS form
    /// (e.g. `/outputs/{safe_id}.stdout`).
    pub stdout_path: Option<String>,
    /// Path to the persisted stderr file in session VFS form, if non-empty
    /// stderr was persisted.
    pub stderr_path: Option<String>,
    /// Total line count of the persisted stdout.
    pub stdout_total_lines: usize,
}

/// Persist exec output to session VFS.
///
/// Writes stdout to `/outputs/{safe_id}.stdout` and stderr to
/// `/outputs/{safe_id}.stderr` (if non-empty). Returns paths and metadata.
///
/// This is the shared helper that any sandbox tool or hook can call.
pub async fn persist_output(
    file_store: &Arc<dyn SessionFileSystem>,
    session_id: SessionId,
    tool_call_id: &str,
    stdout: &str,
    stderr: &str,
) -> Option<PersistResult> {
    if stdout.is_empty() && stderr.is_empty() {
        return None;
    }

    let safe_id: String = tool_call_id
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if safe_id.is_empty() {
        return None;
    }

    let mut result = PersistResult {
        stdout_path: None,
        stderr_path: None,
        stdout_total_lines: 0,
    };

    // Persist stdout whenever present so verbosity-truncated results remain recoverable.
    if !stdout.is_empty() {
        let stdout_to_persist = truncate_for_persistence(stdout, MAX_PERSISTED_STREAM_BYTES);
        let path = format!("{OUTPUT_DIR}/{safe_id}.stdout");
        result.stdout_total_lines = stdout.lines().count();
        if file_store
            .write_file(session_id, &path, &stdout_to_persist, "utf-8")
            .await
            .is_ok()
        {
            result.stdout_path = Some(path);
        }
    }

    // Persist stderr separately whenever present.
    if !stderr.is_empty() {
        let stderr_to_persist = truncate_for_persistence(stderr, MAX_PERSISTED_STREAM_BYTES);
        let path = format!("{OUTPUT_DIR}/{safe_id}.stderr");
        if file_store
            .write_file(session_id, &path, &stderr_to_persist, "utf-8")
            .await
            .is_ok()
        {
            result.stderr_path = Some(path);
        }
    }

    // Only return Some if at least one file was persisted
    if result.stdout_path.is_some() || result.stderr_path.is_some() {
        Some(result)
    } else {
        None
    }
}

/// Back-compat wrapper for older callers. Despite the historical name, this now
/// persists any non-empty opted-in output so verbosity-truncated results are
/// recoverable through `read_file`.
pub async fn persist_large_output(
    file_store: &Arc<dyn SessionFileSystem>,
    session_id: SessionId,
    tool_call_id: &str,
    stdout: &str,
    stderr: &str,
) -> Option<PersistResult> {
    persist_output(file_store, session_id, tool_call_id, stdout, stderr).await
}

/// Truncate persisted stream content to a bounded size with a clear marker.
fn truncate_for_persistence(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }

    // Keep valid UTF-8 boundary at or below the byte cap.
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_string();
    truncated.push_str("\n\n[output truncated before persistence due to size limit]");
    truncated
}

/// Build the annotated truncated stdout string with file reference.
pub fn annotate_truncated_output(truncated: &str, file_path: &str, full_size: usize) -> String {
    let size_kb = full_size / 1024;
    format!(
        "{truncated}\n\n[full output saved to {file_path} ({size_kb} KiB) — use read_file with offset/limit]"
    )
}

/// After successful persistence, compact the inline model-facing output fields
/// in the result JSON so the LLM never carries unbounded content forward.
///
/// Budget policy (EVE-562): resolve the requested mode against the process
/// outcome. Default `auto` uses the compact success window or normal failure
/// window; explicit modes retain their documented fixed budgets.
///
/// The same budget applies to both `stdout`/`output` and `stderr`. Full content
/// is always recoverable from the persisted `/outputs/` files.
pub fn compact_persisted_result_for_model(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    stdout_path: &str,
    stdout_full_size: usize,
    output_mode: &str,
) {
    let exit_code = result_exit_code(obj);
    let budget = output_verbosity_budget(resolve_auto_mode(output_mode, exit_code));

    // Compact stdout (or output fallback field) and append file pointer.
    for field in ["stdout", "output"] {
        if let Some(text) = obj.get(field).and_then(|v| v.as_str()) {
            let compacted =
                compact_with_annotation(text, budget, exit_code, stdout_path, stdout_full_size);
            obj.insert(field.to_string(), json!(compacted));
            break; // only one of stdout/output is expected
        }
    }

    // Compact stderr inline if present (no separate file pointer needed — already in output_files).
    compact_stderr_inline(obj, budget);
}

/// Compact the inline `stderr` field to `budget` bytes using outcome-aware truncation.
///
/// Called from `compact_persisted_result_for_model` and also directly when stderr was
/// persisted but there is no stdout file (so `compact_persisted_result_for_model` was
/// not invoked).
fn compact_stderr_inline(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    budget: Option<usize>,
) {
    if let Some(budget) = budget
        && let Some(stderr_text) = obj.get("stderr").and_then(|v| v.as_str())
        && stderr_text.len() > budget
    {
        let compacted = priority_aware_truncate(stderr_text, budget);
        obj.insert("stderr".to_string(), json!(compacted));
    }
}

/// Return a sentinel exit code suitable for outcome-aware truncation.
fn result_exit_code(obj: &serde_json::Map<String, serde_json::Value>) -> i32 {
    if let Some(code) = obj.get("exit_code").and_then(|v| v.as_i64()) {
        return if code == 0 { 0 } else { 1 };
    }
    if let Some(ok) = obj.get("success").and_then(|v| v.as_bool()) {
        return if ok { 0 } else { 1 };
    }
    // Default to success when neither field is present (non-exec tools).
    0
}

/// Compact `text` to `budget` bytes and append a file-reference annotation.
fn compact_with_annotation(
    text: &str,
    budget: Option<usize>,
    exit_code: i32,
    file_path: &str,
    full_size: usize,
) -> String {
    let compacted = budget.filter(|budget| text.len() > *budget).map_or_else(
        || text.to_string(),
        |budget| truncate_exec_stream(text, budget, exit_code),
    );
    annotate_truncated_output(&compacted, file_path, full_size)
}

pub const TOOL_OUTPUT_PERSISTENCE_CAPABILITY_ID: &str = "tool_output_persistence";

/// Capability that persists full tool output to session VFS.
pub struct ToolOutputPersistenceCapability;

impl Capability for ToolOutputPersistenceCapability {
    fn id(&self) -> &str {
        TOOL_OUTPUT_PERSISTENCE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Tool Output Persistence"
    }

    fn description(&self) -> &str {
        "Persists full exec tool output to session VFS before truncation, \
         enabling lossless retrieval via read_file or grep."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Збереження виводу інструментів",
            "Зберігає повний вивід інструментів виконання у VFS сесії перед обрізанням, що дає змогу отримати його без втрат через read_file або grep.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }

    fn post_tool_exec_hooks(&self) -> Vec<Arc<dyn PostToolExecHook>> {
        vec![Arc::new(PersistOutputHook)]
    }
}

/// Hook that persists tool output to VFS when `persist_output` hint is set.
pub struct PersistOutputHook;

#[async_trait]
impl PostToolExecHook for PersistOutputHook {
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    ) {
        // Only persist if tool declares persist_output hint
        if tool_def.hints().persist_output != Some(true) {
            return;
        }

        if result
            .result
            .as_ref()
            .and_then(|value| value.get("output_files"))
            .is_some()
        {
            return;
        }

        // Take raw_output (pre-truncation cleaned output) if available.
        // Falls back to extracting from the (truncated) result JSON.
        let output_text = result.raw_output.take().unwrap_or_else(|| {
            result
                .result
                .as_ref()
                .map(extract_output_text)
                .unwrap_or_default()
        });
        if output_text.is_empty() {
            return;
        }

        // Need file_store from context
        let Some(ref file_store) = context.file_store else {
            return;
        };

        // Split into stdout/stderr for the shared helper
        let (stdout, stderr) = split_output_streams(&output_text);

        if let Some(persist_result) = persist_large_output(
            file_store,
            context.session_id,
            &tool_call.id,
            stdout,
            stderr,
        )
        .await
        {
            let output_mode = tool_call
                .arguments
                .get("output")
                .and_then(|value| value.as_str())
                .unwrap_or("auto");
            // Enrich result JSON with file references and compact inline output (EVE-562).
            if let Some(ref mut json_val) = result.result
                && let Some(obj) = json_val.as_object_mut()
            {
                let mut output_files = Vec::new();

                if let Some(ref path) = persist_result.stdout_path {
                    let display_path = file_store.display_path(path);
                    compact_persisted_result_for_model(
                        obj,
                        &display_path,
                        stdout.len(),
                        output_mode,
                    );

                    output_files.push(display_path.clone());
                    // full_output points to the stdout file only; stderr (if persisted)
                    // is available via the output_files array
                    obj.insert("full_output".to_string(), json!(display_path));
                    obj.insert(
                        "total_lines".to_string(),
                        json!(persist_result.stdout_total_lines),
                    );
                }

                if let Some(ref path) = persist_result.stderr_path {
                    output_files.push(file_store.display_path(path));
                    // Compact stderr inline when stdout was not persisted (if stdout was
                    // persisted, compact_persisted_result_for_model already handled this).
                    if persist_result.stdout_path.is_none() {
                        let exit_code = result_exit_code(obj);
                        let budget =
                            output_verbosity_budget(resolve_auto_mode(output_mode, exit_code));
                        compact_stderr_inline(obj, budget);
                    }
                }

                if !output_files.is_empty() {
                    obj.insert("output_files".to_string(), json!(output_files));
                }
            }
        }
    }
}

/// Split combined output text into stdout and stderr streams.
///
/// The raw output from exec tools uses `\n--- stderr ---\n` as a separator
/// (see `bashkit_shell.rs` and other sandbox tools). Uses `rfind` to split at
/// the *last* occurrence, since the separator is injected by our tools and
/// shouldn't appear more than once — but if stdout happens to contain the
/// marker text, taking the last match minimizes corruption.
/// If no separator is found, the entire text is treated as stdout.
fn split_output_streams(text: &str) -> (&str, &str) {
    const STDERR_SEPARATOR: &str = "\n--- stderr ---\n";
    if let Some(pos) = text.rfind(STDERR_SEPARATOR) {
        (&text[..pos], &text[pos + STDERR_SEPARATOR.len()..])
    } else {
        (text, "")
    }
}

/// Extract readable text content from a tool result JSON value.
///
/// Looks for common exec tool output fields: `stdout`, `stderr`, `output`.
/// Concatenates them into a single string for persistence.
fn extract_output_text(json: &serde_json::Value) -> String {
    let mut parts = Vec::new();

    if let Some(stdout) = json.get("stdout").and_then(|v| v.as_str())
        && !stdout.is_empty()
    {
        parts.push(stdout);
    }
    if let Some(stderr) = json.get("stderr").and_then(|v| v.as_str())
        && !stderr.is_empty()
    {
        if !parts.is_empty() {
            parts.push("\n--- stderr ---\n");
        }
        parts.push(stderr);
    }
    // Fallback: if no stdout/stderr, try "output" field (daytona_exec)
    if parts.is_empty()
        && let Some(output) = json.get("output").and_then(|v| v.as_str())
        && !output.is_empty()
    {
        parts.push(output);
    }

    parts.join("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_output_sanitizer::EXEC_OUTPUT_BUDGET;

    #[test]
    fn test_extract_output_text_stdout_stderr() {
        let json = json!({
            "stdout": "hello world",
            "stderr": "warning: unused variable",
            "exit_code": 0
        });
        let text = extract_output_text(&json);
        assert!(text.contains("hello world"));
        assert!(text.contains("warning: unused variable"));
        assert!(text.contains("--- stderr ---"));
    }

    #[test]
    fn test_extract_output_text_stdout_only() {
        let json = json!({
            "stdout": "hello",
            "stderr": "",
            "exit_code": 0
        });
        assert_eq!(extract_output_text(&json), "hello");
    }

    #[test]
    fn test_extract_output_text_output_field() {
        let json = json!({
            "output": "combined output",
            "exit_code": 0
        });
        assert_eq!(extract_output_text(&json), "combined output");
    }

    #[test]
    fn test_extract_output_text_empty() {
        let json = json!({ "exit_code": 0 });
        assert_eq!(extract_output_text(&json), "");
    }

    #[test]
    fn test_extract_output_text_prefers_stdout_over_output() {
        let json = json!({
            "stdout": "from stdout",
            "output": "from output",
            "exit_code": 0
        });
        // stdout takes precedence — output field is fallback
        let text = extract_output_text(&json);
        assert!(text.contains("from stdout"));
        assert!(!text.contains("from output"));
    }

    #[test]
    fn test_capability_metadata() {
        let cap = ToolOutputPersistenceCapability;
        assert_eq!(cap.id(), "tool_output_persistence");
        assert!(!cap.post_tool_exec_hooks().is_empty());
        assert!(cap.dependencies().contains(&"session_file_system"));
    }

    #[test]
    fn test_split_output_streams_both() {
        let text = "hello world\n--- stderr ---\nwarning: unused";
        let (stdout, stderr) = split_output_streams(text);
        assert_eq!(stdout, "hello world");
        assert_eq!(stderr, "warning: unused");
    }

    #[test]
    fn test_split_output_streams_stdout_only() {
        let text = "hello world\nline two";
        let (stdout, stderr) = split_output_streams(text);
        assert_eq!(stdout, "hello world\nline two");
        assert!(stderr.is_empty());
    }

    #[test]
    fn test_split_output_streams_empty() {
        let (stdout, stderr) = split_output_streams("");
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn test_annotate_truncated_output() {
        let annotated = annotate_truncated_output(
            "truncated text...",
            "/workspace/outputs/abc.stdout",
            50 * 1024,
        );
        assert!(annotated.contains("truncated text..."));
        assert!(annotated.contains("/workspace/outputs/abc.stdout"));
        assert!(annotated.contains("50 KiB"));
        assert!(annotated.contains("read_file"));
    }

    #[test]
    fn test_annotate_truncated_output_small() {
        let annotated = annotate_truncated_output("short", "/workspace/outputs/x.stdout", 1024);
        assert!(annotated.contains("1 KiB"));
        assert!(annotated.contains("read_file with offset/limit"));
    }

    #[test]
    fn test_truncate_for_persistence_noop_when_under_limit() {
        let text = "hello";
        assert_eq!(truncate_for_persistence(text, 1024), text);
    }

    #[test]
    fn test_truncate_for_persistence_adds_marker_when_over_limit() {
        let text = "x".repeat(128);
        let out = truncate_for_persistence(&text, 32);
        assert!(out.len() > 32);
        assert!(out.starts_with(&"x".repeat(32)));
        assert!(out.contains("output truncated before persistence"));
    }

    // --- persist_large_output tests ---

    use crate::error::Result;
    use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use chrono::Utc;
    use everruns_core::session_files::SessionFileSystem;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct MockFileStore {
        files: Mutex<HashMap<String, String>>,
    }

    impl MockFileStore {
        fn content(&self, path: &str) -> Option<String> {
            self.files.lock().unwrap().get(path).cloned()
        }
    }

    #[async_trait]
    impl SessionFileSystem for MockFileStore {
        fn is_mount_resolver(&self) -> bool {
            false
        }

        async fn read_file(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> Result<Option<SessionFile>> {
            Ok(None)
        }

        async fn write_file(
            &self,
            _session_id: SessionId,
            path: &str,
            content: &str,
            _encoding: &str,
        ) -> Result<SessionFile> {
            self.files
                .lock()
                .unwrap()
                .insert(path.to_string(), content.to_string());
            Ok(SessionFile {
                id: Uuid::new_v4(),
                session_id: Uuid::nil(),
                path: path.to_string(),
                name: path.rsplit('/').next().unwrap_or("").to_string(),
                content: Some(content.to_string()),
                encoding: "utf-8".to_string(),
                is_directory: false,
                is_readonly: false,
                size_bytes: content.len() as i64,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
        }

        async fn delete_file(
            &self,
            _session_id: SessionId,
            _path: &str,
            _recursive: bool,
        ) -> Result<bool> {
            Ok(false)
        }

        async fn list_directory(
            &self,
            _session_id: SessionId,
            _path: &str,
        ) -> Result<Vec<FileInfo>> {
            Ok(vec![])
        }

        async fn stat_file(&self, _session_id: SessionId, _path: &str) -> Result<Option<FileStat>> {
            Ok(None)
        }

        async fn grep_files(
            &self,
            _session_id: SessionId,
            _pattern: &str,
            _path_pattern: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            Ok(vec![])
        }

        async fn create_directory(&self, _session_id: SessionId, _path: &str) -> Result<FileInfo> {
            Err(anyhow::anyhow!("not implemented").into())
        }
    }

    fn test_session_id() -> SessionId {
        SessionId::from(Uuid::nil())
    }

    fn persistence_tool_def() -> ToolDefinition {
        use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};

        ToolDefinition::Builtin(BuiltinTool {
            name: "bash".to_string(),
            display_name: None,
            description: "execute a command".to_string(),
            parameters: json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default().with_persist_output(true),
            full_parameters: None,
        })
    }

    #[tokio::test]
    async fn test_hook_honors_explicit_normal_and_persists_losslessly() {
        let mut lines = vec!["src/runtime.rs:10: first relevant match".to_string()];
        lines.extend((0..1200).map(|i| format!("src/module_{i}.rs: ordinary source line")));
        lines.extend((0..300).map(|i| format!("src/errors_{i}.rs: struct ErrorContext{i}")));
        let full_output = lines.join("\n");
        let visible_output =
            crate::tool_output_sanitizer::truncate_exec_stream(&full_output, NORMAL_BUDGET, 0);
        let store = Arc::new(MockFileStore::default());
        let mut context = ToolContext::new(test_session_id());
        context.file_store = Some(store.clone());
        let tool_call = ToolCall {
            id: "call-normal".to_string(),
            name: "bash".to_string(),
            arguments: json!({"command": "git grep Error", "output": "normal"}),
        };
        let mut result = ToolResult {
            tool_call_id: tool_call.id.clone(),
            result: Some(json!({
                "stdout": visible_output,
                "stderr": "",
                "exit_code": 0,
                "success": true,
            })),
            images: None,
            error: None,
            connection_required: None,
            raw_output: Some(full_output.clone()),
        };

        PersistOutputHook
            .after_exec(&tool_call, &persistence_tool_def(), &mut result, &context)
            .await;

        let inline = result.result.as_ref().unwrap()["stdout"].as_str().unwrap();
        assert!(inline.len() > AUTO_SUCCESS_BUDGET + 200);
        assert!(inline.len() <= NORMAL_BUDGET + 200);
        assert!(inline.contains("first relevant match"));
        assert_eq!(
            store.content("/outputs/call-normal.stdout").as_deref(),
            Some(full_output.as_str())
        );
    }

    #[tokio::test]
    async fn test_persist_large_output_persists_stdout_below_budget() {
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::default());
        let small = "a".repeat(EXEC_OUTPUT_BUDGET);
        let result = persist_large_output(&store, test_session_id(), "call-1", &small, "").await;
        let r = result.expect("should persist non-empty stdout");
        assert_eq!(r.stdout_path.as_deref(), Some("/outputs/call-1.stdout"));
    }

    #[tokio::test]
    async fn test_persist_large_output_persists_stdout_above_budget() {
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        let large = "x".repeat(EXEC_OUTPUT_BUDGET + 1);
        let result = persist_large_output(&store, test_session_id(), "call-2", &large, "").await;
        let r = result.expect("should persist");
        assert!(r.stdout_path.is_some());
        assert_eq!(r.stdout_path.as_deref(), Some("/outputs/call-2.stdout"));
        assert!(r.stderr_path.is_none());
        assert_eq!(r.stdout_total_lines, 1);
        // Verify content was written
        assert_eq!(
            mock.content("/outputs/call-2.stdout").unwrap().len(),
            large.len()
        );
    }

    #[tokio::test]
    async fn test_persist_large_output_caps_persisted_stdout_size() {
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        let huge = "x".repeat(MAX_PERSISTED_STREAM_BYTES + 2048);
        let result = persist_large_output(&store, test_session_id(), "call-cap", &huge, "").await;
        assert!(result.is_some());
        let persisted = mock.content("/outputs/call-cap.stdout").unwrap();
        assert!(persisted.len() < huge.len());
        assert!(persisted.contains("output truncated before persistence"));
    }

    #[tokio::test]
    async fn test_persist_large_output_persists_stderr_above_threshold() {
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        let large_stderr = "e".repeat(4097);
        let result =
            persist_large_output(&store, test_session_id(), "call-3", "small", &large_stderr).await;
        let r = result.expect("should persist stderr");
        assert_eq!(r.stdout_path.as_deref(), Some("/outputs/call-3.stdout"));
        assert_eq!(r.stderr_path.as_deref(), Some("/outputs/call-3.stderr"));
        assert_eq!(mock.content("/outputs/call-3.stderr").unwrap().len(), 4097);
    }

    #[tokio::test]
    async fn test_persist_large_output_both_stdout_and_stderr() {
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        let large_stdout = "o".repeat(EXEC_OUTPUT_BUDGET + 100);
        let large_stderr = "e".repeat(5000);
        let result = persist_large_output(
            &store,
            test_session_id(),
            "call-4",
            &large_stdout,
            &large_stderr,
        )
        .await;
        let r = result.expect("should persist both");
        assert!(r.stdout_path.is_some());
        assert!(r.stderr_path.is_some());
    }

    #[tokio::test]
    async fn test_persist_large_output_sanitizes_id() {
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        let large = "x".repeat(EXEC_OUTPUT_BUDGET + 1);
        let result =
            persist_large_output(&store, test_session_id(), "call/../../../etc", &large, "").await;
        let r = result.expect("should persist with sanitized id");
        // Path traversal chars stripped, only alphanumeric + dash + underscore kept
        assert_eq!(r.stdout_path.as_deref(), Some("/outputs/calletc.stdout"));
    }

    #[tokio::test]
    async fn test_persist_large_output_empty_id_returns_none() {
        let store: Arc<dyn SessionFileSystem> = Arc::new(MockFileStore::default());
        let large = "x".repeat(EXEC_OUTPUT_BUDGET + 1);
        let result = persist_large_output(&store, test_session_id(), "///...", &large, "").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_persist_large_output_line_count() {
        let mock = Arc::new(MockFileStore::default());
        let store: Arc<dyn SessionFileSystem> = mock.clone();
        // Create multi-line output that exceeds budget
        let line = "x".repeat(200);
        let lines: Vec<&str> = std::iter::repeat_n(line.as_str(), 100).collect();
        let large = lines.join("\n");
        assert!(large.len() > EXEC_OUTPUT_BUDGET);
        let result =
            persist_large_output(&store, test_session_id(), "call-lines", &large, "").await;
        let r = result.expect("should persist");
        assert_eq!(r.stdout_total_lines, 100);
    }

    // -------------------------------------------------------------------------
    // compact_persisted_result_for_model (EVE-562)
    // -------------------------------------------------------------------------
    use crate::tool_output_sanitizer::{AUTO_SUCCESS_BUDGET, NORMAL_BUDGET};

    #[test]
    fn test_compact_success_caps_stdout_to_auto_success_budget() {
        let large_stdout = "x".repeat(NORMAL_BUDGET + 1);
        let large_stdout_len = large_stdout.len();
        let mut obj = serde_json::Map::new();
        obj.insert("stdout".to_string(), json!(large_stdout));
        obj.insert("exit_code".to_string(), json!(0));

        compact_persisted_result_for_model(
            &mut obj,
            "/outputs/call.stdout",
            large_stdout_len,
            "auto",
        );

        let inline = obj["stdout"].as_str().unwrap();
        // Inline stdout must be compact (budget + annotation pointer).
        assert!(
            inline.len() <= AUTO_SUCCESS_BUDGET + 200,
            "inline stdout ({} bytes) should be ≤ AUTO_SUCCESS_BUDGET + pointer",
            inline.len()
        );
        assert!(
            inline.contains("full output saved to"),
            "must have file pointer"
        );
    }

    #[test]
    fn test_compact_failure_caps_stdout_to_normal_budget() {
        let large_stdout = "y".repeat(NORMAL_BUDGET * 2);
        let large_stdout_len = large_stdout.len();
        let mut obj = serde_json::Map::new();
        obj.insert("stdout".to_string(), json!(large_stdout));
        obj.insert("exit_code".to_string(), json!(1));

        compact_persisted_result_for_model(
            &mut obj,
            "/outputs/call.stdout",
            large_stdout_len,
            "auto",
        );

        let inline = obj["stdout"].as_str().unwrap();
        assert!(
            inline.len() <= NORMAL_BUDGET + 200,
            "failure stdout ({} bytes) should be ≤ NORMAL_BUDGET + pointer",
            inline.len()
        );
        assert!(
            inline.contains("full output saved to"),
            "must have file pointer"
        );
    }

    #[test]
    fn test_compact_small_stdout_unchanged_except_annotation() {
        let small = "hello world";
        let mut obj = serde_json::Map::new();
        obj.insert("stdout".to_string(), json!(small));
        obj.insert("exit_code".to_string(), json!(0));

        compact_persisted_result_for_model(&mut obj, "/outputs/call.stdout", small.len(), "auto");

        let inline = obj["stdout"].as_str().unwrap();
        assert!(
            inline.starts_with("hello world"),
            "small content must be preserved"
        );
        assert!(
            inline.contains("full output saved to"),
            "must have file pointer"
        );
    }

    #[test]
    fn test_compact_oversized_full_mode_stays_full() {
        let large = "z".repeat(NORMAL_BUDGET * 3);
        let large_len = large.len();
        let mut obj = serde_json::Map::new();
        obj.insert("stdout".to_string(), json!(large.clone()));
        obj.insert("exit_code".to_string(), json!(0));

        compact_persisted_result_for_model(&mut obj, "/outputs/call.stdout", large_len, "full");

        let inline = obj["stdout"].as_str().unwrap();
        assert!(inline.starts_with(&large));
        assert!(inline.contains("full output saved to"));
    }

    #[test]
    fn test_compact_explicit_normal_preserves_leading_search_matches() {
        let mut lines = vec![
            "src/runtime.rs:10: query_history registration".to_string(),
            "src/runtime.rs:11: history capability".to_string(),
        ];
        lines.extend((0..1200).map(|i| format!("src/module_{i}.rs: ordinary source line")));
        lines.extend((0..300).map(|i| format!("src/errors_{i}.rs: struct ErrorContext{i}")));
        let output = lines.join("\n");
        let mut obj = serde_json::Map::new();
        obj.insert("stdout".to_string(), json!(output.clone()));
        obj.insert("exit_code".to_string(), json!(0));

        compact_persisted_result_for_model(
            &mut obj,
            "/outputs/call.stdout",
            output.len(),
            "normal",
        );

        let inline = obj["stdout"].as_str().unwrap();
        assert!(inline.len() > AUTO_SUCCESS_BUDGET + 200);
        assert!(inline.len() <= NORMAL_BUDGET + 200);
        assert!(inline.contains("query_history registration"));
        assert!(inline.contains("history capability"));
        assert!(inline.contains("full output saved to"));
    }

    #[test]
    fn test_compact_large_stderr_capped_on_failure() {
        let large_stderr = "e".repeat(NORMAL_BUDGET * 2);
        let large_stdout = "o".repeat(100);
        let large_stdout_len = large_stdout.len();
        let mut obj = serde_json::Map::new();
        obj.insert("stdout".to_string(), json!(large_stdout));
        obj.insert("stderr".to_string(), json!(large_stderr));
        obj.insert("exit_code".to_string(), json!(1));

        compact_persisted_result_for_model(
            &mut obj,
            "/outputs/call.stdout",
            large_stdout_len,
            "auto",
        );

        let inline_stderr = obj["stderr"].as_str().unwrap();
        assert!(
            inline_stderr.len() <= NORMAL_BUDGET + 200,
            "stderr ({} bytes) must be capped to NORMAL_BUDGET",
            inline_stderr.len()
        );
    }

    #[test]
    fn test_compact_non_exec_output_field() {
        // daytona_exec uses "output" instead of "stdout"
        let large_output = "d".repeat(NORMAL_BUDGET + 1);
        let large_output_len = large_output.len();
        let mut obj = serde_json::Map::new();
        obj.insert("output".to_string(), json!(large_output));
        obj.insert("exit_code".to_string(), json!(0));

        compact_persisted_result_for_model(
            &mut obj,
            "/outputs/call.stdout",
            large_output_len,
            "auto",
        );

        let inline = obj["output"].as_str().unwrap();
        assert!(
            inline.len() <= AUTO_SUCCESS_BUDGET + 200,
            "output field must be compacted"
        );
        assert!(inline.contains("full output saved to"));
    }

    #[test]
    fn test_compact_no_file_store_preserves_current_behavior() {
        // When there's no stdout field (unusual result shape), nothing is corrupted.
        let mut obj = serde_json::Map::new();
        obj.insert("result".to_string(), json!("ok"));
        obj.insert("exit_code".to_string(), json!(0));

        compact_persisted_result_for_model(&mut obj, "/outputs/call.stdout", 0, "auto");

        // Non-exec result shape untouched
        assert_eq!(obj.get("stdout"), None);
        assert_eq!(obj.get("output"), None);
        assert_eq!(obj["result"].as_str(), Some("ok"));
    }

    #[test]
    fn test_compact_stderr_inline_caps_stderr() {
        // Exercises the compact_stderr_inline helper directly, which is also called
        // in after_exec when stderr is persisted but stdout is not.
        let large = "e".repeat(NORMAL_BUDGET * 2);
        let mut obj = serde_json::Map::new();
        obj.insert("stderr".to_string(), json!(large));

        compact_stderr_inline(&mut obj, Some(NORMAL_BUDGET));

        let inline = obj["stderr"].as_str().unwrap();
        assert!(
            inline.len() <= NORMAL_BUDGET + 200,
            "compact_stderr_inline must cap stderr to budget ({} bytes got)",
            inline.len()
        );
    }
}
