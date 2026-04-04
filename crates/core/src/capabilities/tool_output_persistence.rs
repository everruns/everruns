// Tool Output Persistence Capability (EVE-222, EVE-245)
//
// Persists full exec tool output to session VFS before truncation,
// enabling lossless retrieval via read_file/grep. The LLM gets a
// truncated summary with file paths to the full output.
//
// Design decisions:
// - Implemented as PostToolExecHook, not baked into each tool — VFS
//   persistence is a cross-cutting concern the tool shouldn't know about
// - Reads `persist_output` hint from ToolDefinition to decide what to persist
// - Writes stdout to /.outputs/{tool_call_id}.stdout, stderr to /.outputs/{tool_call_id}.stderr (EVE-245)
// - Injects `output_files` array into result for agent to read selectively
// - Graceful degradation: skip silently if file_store is unavailable
// - Runs before FinalPostToolExecHook (EVE-225 hard limit)

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use super::{Capability, CapabilityStatus};
use crate::atoms::PostToolExecHook;
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::ToolContext;

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

        // Sanitize tool_call.id for path safety — use only alphanumeric, dash, underscore
        let safe_id: String = tool_call
            .id
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if safe_id.is_empty() {
            return;
        }

        // Split into stdout and stderr for separate persistence (EVE-245)
        let (stdout_text, stderr_text) = split_output_streams(&output_text);
        let total_lines = stdout_text.lines().count();

        let stdout_path = format!("/.outputs/{safe_id}.stdout");
        if let Err(e) = file_store
            .write_file(context.session_id, &stdout_path, stdout_text, "utf-8")
            .await
        {
            tracing::warn!(
                tool_name = %tool_call.name,
                tool_call_id = %tool_call.id,
                error = %e,
                "PersistOutputHook: failed to write stdout output"
            );
            return;
        }

        let mut output_files = vec![stdout_path.clone()];

        // Persist stderr separately if non-empty
        if !stderr_text.is_empty() {
            let stderr_path = format!("/.outputs/{safe_id}.stderr");
            if let Err(e) = file_store
                .write_file(context.session_id, &stderr_path, stderr_text, "utf-8")
                .await
            {
                tracing::warn!(
                    tool_name = %tool_call.name,
                    tool_call_id = %tool_call.id,
                    error = %e,
                    "PersistOutputHook: failed to write stderr output"
                );
                // Continue — stdout was persisted successfully
            } else {
                output_files.push(stderr_path);
            }
        }

        // Enrich result JSON with file references (EVE-245)
        if let Some(ref mut json_val) = result.result
            && let Some(obj) = json_val.as_object_mut()
        {
            obj.insert("full_output".to_string(), json!(stdout_path));
            obj.insert("total_lines".to_string(), json!(total_lines));
            obj.insert("output_files".to_string(), json!(output_files));
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
}
