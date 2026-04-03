// Tool Output Persistence Capability (EVE-222, EVE-245)
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
// - Writes stdout to /.outputs/{safe_id}.stdout, stderr to
//   /.outputs/{safe_id}.stderr when stderr is persisted (session-scoped)
// - Injects `output_files` array into result for agent to read selectively
// - Graceful degradation: skip silently if file_store is unavailable
// - Runs before FinalPostToolExecHook (EVE-225 hard limit)
// - EVE-245: annotates truncated stdout with file reference so agent knows
//   it can read_file the full output with offset/limit

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::{Capability, CapabilityStatus};
use crate::atoms::PostToolExecHook;
use crate::tool_output_sanitizer::EXEC_OUTPUT_BUDGET;
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::{SessionFileStore, ToolContext};
use crate::typed_id::SessionId;

/// Result of persisting large exec output to session VFS.
pub struct PersistResult {
    /// Path to the persisted stdout file in session VFS form
    /// (e.g. `/.outputs/{safe_id}.stdout`).
    pub stdout_path: Option<String>,
    /// Path to the persisted stderr file in session VFS form, if non-empty
    /// stderr was persisted.
    pub stderr_path: Option<String>,
    /// Total line count of the persisted stdout.
    pub stdout_total_lines: usize,
}

/// Persist large exec output to session VFS when it exceeds the budget.
///
/// Writes stdout to `/.outputs/{safe_id}.stdout` and stderr to
/// `/.outputs/{safe_id}.stderr` (if non-empty). Returns paths and metadata.
///
/// This is the shared helper that any sandbox tool or hook can call.
pub async fn persist_large_output(
    file_store: &Arc<dyn SessionFileStore>,
    session_id: SessionId,
    tool_call_id: &str,
    stdout: &str,
    stderr: &str,
) -> Option<PersistResult> {
    // Only persist if stdout exceeds its budget or stderr exceeds 4096 bytes
    if stdout.len() <= EXEC_OUTPUT_BUDGET && stderr.len() <= 4096 {
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

    // Persist stdout
    if stdout.len() > EXEC_OUTPUT_BUDGET {
        let path = format!("/.outputs/{safe_id}.stdout");
        result.stdout_total_lines = stdout.lines().count();
        if file_store
            .write_file(session_id, &path, stdout, "utf-8")
            .await
            .is_ok()
        {
            result.stdout_path = Some(path);
        }
    }

    // Persist stderr separately if large
    if stderr.len() > 4096 {
        let path = format!("/.outputs/{safe_id}.stderr");
        if file_store
            .write_file(session_id, &path, stderr, "utf-8")
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

/// Build the annotated truncated stdout string with file reference.
pub fn annotate_truncated_output(truncated: &str, file_path: &str, full_size: usize) -> String {
    let size_kb = full_size / 1024;
    format!(
        "{truncated}\n\n[full output saved to /workspace{file_path} ({size_kb} KiB) — use read_file with offset/limit]"
    )
}

/// Capability that persists full tool output to session VFS.
pub struct ToolOutputPersistenceCapability;

impl Capability for ToolOutputPersistenceCapability {
    fn id(&self) -> &str {
        "tool_output_persistence"
    }

    fn name(&self) -> &str {
        "Tool Output Persistence"
    }

    fn description(&self) -> &str {
        "Persists full exec tool output to session VFS before truncation, \
         enabling lossless retrieval via read_file or grep."
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
struct PersistOutputHook;

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
            // Enrich result JSON with file references
            if let Some(ref mut json_val) = result.result
                && let Some(obj) = json_val.as_object_mut()
            {
                let mut output_files = Vec::new();

                if let Some(ref path) = persist_result.stdout_path {
                    // Annotate the truncated stdout with file reference
                    if let Some(current_stdout) = obj.get("stdout").and_then(|v| v.as_str()) {
                        let annotated =
                            annotate_truncated_output(current_stdout, path, stdout.len());
                        obj.insert("stdout".to_string(), json!(annotated));
                    }
                    output_files.push(format!("/workspace{path}"));
                    // full_output points to the stdout file only; stderr (if persisted)
                    // is available via the output_files array
                    obj.insert("full_output".to_string(), json!(path));
                    obj.insert(
                        "total_lines".to_string(),
                        json!(persist_result.stdout_total_lines),
                    );
                }

                if let Some(ref path) = persist_result.stderr_path {
                    output_files.push(format!("/workspace{path}"));
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
/// (see `virtual_bash.rs` and other sandbox tools). Uses `rfind` to split at
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
        let annotated =
            annotate_truncated_output("truncated text...", "/.outputs/abc.stdout", 50 * 1024);
        assert!(annotated.contains("truncated text..."));
        assert!(annotated.contains("/workspace/.outputs/abc.stdout"));
        assert!(annotated.contains("50 KiB"));
        assert!(annotated.contains("read_file"));
    }

    #[test]
    fn test_annotate_truncated_output_small() {
        let annotated = annotate_truncated_output("short", "/.outputs/x.stdout", 1024);
        assert!(annotated.contains("1 KiB"));
        assert!(annotated.contains("read_file with offset/limit"));
    }
}
