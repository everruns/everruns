// Tool Output Distillation Capability
//
// Produces a compact, content-aware *inline view* of large tool results at
// capture time, while keeping the full original recoverable via `read_file`.
//
// Motivation:
// - Exec/sandbox tools already get a verbosity budget (`tool_output_sanitizer`)
//   plus lossless persistence (`tool_output_persistence`). Their output is
//   handled well.
// - Tools that do NOT declare `persist_output` — most notably MCP tools and
//   `web_fetch` — have neither. Their output enters history verbatim and is
//   only capped by the 64 KiB hard-limit hook, which head-truncates and
//   destroys the tail. This capability targets exactly that gap.
//
// Design decisions:
// - Implemented as a `PostToolExecHook`, the same seam as
//   `tool_output_persistence`. Capability hooks run BEFORE the final
//   infrastructure hooks (persistence + hard-limit), so a distilled result is
//   what the downstream hooks see.
// - Reversibility is mandatory: we never replace content with a lossy view
//   unless the full original was successfully persisted to the session VFS.
//   We reuse `tool_output_persistence::{persist_output, annotate_truncated_output}`
//   rather than reimplementing persistence, and inject the same `output_files`
//   pointer. `PersistOutputHook` skips when `output_files` is already present,
//   so the two never double-write.
// - Routing is by content *shape*, not tool name: MCP tool names are arbitrary
//   and namespaced, so name allowlists (used by compaction masking) do not
//   generalize. We sniff the JSON value instead.
// - Determinism: every transform is deterministic so identical output distills
//   identically, preserving provider KV-cache reuse across turns.
// - Skips tools that declare `persist_output` (exec/sandbox): their existing
//   budget + persistence pipeline already solves the problem, and distilling
//   their already-budgeted inline view would fight that design.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use super::tool_output_persistence::{annotate_truncated_output, persist_output};
use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::atoms::PostToolExecHook;
use crate::tool_types::{ToolCall, ToolDefinition, ToolResult};
use crate::traits::ToolContext;

/// Only distill results whose serialized size exceeds this threshold. Smaller
/// results are left verbatim (cheap, cache-stable, and rarely worth a VFS write).
const MIN_DISTILL_BYTES: usize = 8 * 1024;

/// Strings longer than this are clipped to a head+tail window. Also the array
/// serialized-size threshold above which array sampling kicks in.
const MAX_FIELD_BYTES: usize = 2 * 1024;

/// Number of leading elements kept when sampling a large array.
const SAMPLE_ROWS: usize = 5;

/// Maximum recursion depth when walking a result value (DoS bound).
const MAX_DEPTH: usize = 8;

/// Maximum number of nodes visited while distilling (DoS bound).
const MAX_NODES: usize = 100_000;

/// Capability that distills large tool results into a compact inline view while
/// persisting the full original for lossless retrieval.
pub struct ToolOutputDistillationCapability;

impl Capability for ToolOutputDistillationCapability {
    fn id(&self) -> &str {
        "tool_output_distillation"
    }

    fn name(&self) -> &str {
        "Tool Output Distillation"
    }

    fn description(&self) -> &str {
        "Distills large tool results (notably MCP and web fetch) into a compact, \
         content-aware inline view while persisting the full original to the \
         session filesystem for lossless retrieval via read_file."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Стиснення виводу інструментів",
            "Стискає великий вивід інструментів (зокрема MCP та web fetch) у компактний \
             вигляд із урахуванням типу вмісту, зберігаючи повний оригінал у файловій \
             системі сесії для отримання без втрат через read_file.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }

    fn post_tool_exec_hooks(&self) -> Vec<Arc<dyn PostToolExecHook>> {
        vec![Arc::new(DistillOutputHook)]
    }
}

/// Hook that distills large non-exec tool results in place.
pub struct DistillOutputHook;

#[async_trait]
impl PostToolExecHook for DistillOutputHook {
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
        result: &mut ToolResult,
        context: &ToolContext,
    ) {
        // Exec/sandbox tools are already handled by the verbosity budget +
        // tool_output_persistence. Distilling their budgeted inline view would
        // fight that design, so leave them alone.
        if tool_def.hints().persist_output == Some(true) {
            return;
        }

        // Only operate on successful results. Errors are usually small and
        // diagnostically important; keep them verbatim.
        if result.error.is_some() {
            return;
        }

        let Some(result_value) = result.result.as_mut() else {
            return;
        };

        // Already recoverable (another hook persisted it, or a tool injected
        // its own pointer). Nothing to do.
        if result_value.get("output_files").is_some() {
            return;
        }

        // Size gate: only act on large results.
        let serialized = serde_json::to_string(result_value).unwrap_or_default();
        if serialized.len() < MIN_DISTILL_BYTES {
            return;
        }

        // Reversibility requires a session file store. Without one we cannot
        // guarantee the original is recoverable, so we refuse to distill.
        let Some(file_store) = context.file_store.as_ref() else {
            return;
        };

        // Distill in place, keeping the pre-distill original for persistence.
        let original = result_value.clone();
        let mut stats = DistillStats::default();
        distill_value(result_value, 0, &mut stats);
        if !stats.changed {
            // The shape walker found nothing to clip — e.g. a large object or
            // array made entirely of sub-threshold fields. Such a result is
            // still large; if it later exceeds the 64 KiB OutputHardLimitHook it
            // would be head-truncated with no recovery pointer, losing the tail.
            // Fall back to a head+tail window over the serialized value so the
            // result is always bounded, persisted, and recoverable.
            *result_value = Value::String(head_tail(&serialized, MAX_FIELD_BYTES));
        }

        // Persist the full original (pretty-printed JSON) for lossless retrieval.
        let original_text =
            serde_json::to_string_pretty(&original).unwrap_or_else(|_| original.to_string());
        let original_len = original_text.len();
        let persisted = persist_output(
            file_store,
            context.session_id,
            &tool_call.id,
            &original_text,
            "",
        )
        .await;

        let Some(stdout_path) = persisted.and_then(|p| p.stdout_path) else {
            // Persistence failed: we cannot guarantee recovery, so restore the
            // verbatim original instead of leaving a lossy, irrecoverable result.
            *result_value = original;
            return;
        };

        inject_pointer(result_value, &stdout_path, original_len);
    }
}

/// Tracks whether distillation actually changed anything and bounds traversal.
#[derive(Default)]
struct DistillStats {
    changed: bool,
    nodes: usize,
}

/// Recursively distill a JSON value in place: clip long strings, sample large
/// arrays. Bounded by depth and node count.
fn distill_value(value: &mut Value, depth: usize, stats: &mut DistillStats) {
    if depth > MAX_DEPTH || stats.nodes >= MAX_NODES {
        return;
    }
    stats.nodes += 1;

    match value {
        Value::String(s) => {
            if s.len() > MAX_FIELD_BYTES {
                *s = distill_text(s, MAX_FIELD_BYTES);
                stats.changed = true;
            }
        }
        Value::Array(arr) => {
            // Only arrays longer than the sample size can ever be sampled, so
            // check length first and let `&&` short-circuit the O(n)
            // serialization for small arrays.
            if arr.len() > SAMPLE_ROWS
                && serde_json::to_string(arr).map(|s| s.len()).unwrap_or(0) > MAX_FIELD_BYTES
            {
                let omitted = arr.len() - SAMPLE_ROWS;
                arr.truncate(SAMPLE_ROWS);
                for el in arr.iter_mut() {
                    distill_value(el, depth + 1, stats);
                }
                arr.push(json!(format!(
                    "[… {omitted} more item(s) elided — full output via read_file …]"
                )));
                stats.changed = true;
            } else {
                for el in arr.iter_mut() {
                    distill_value(el, depth + 1, stats);
                }
            }
        }
        Value::Object(map) => {
            for (_k, v) in map.iter_mut() {
                distill_value(v, depth + 1, stats);
            }
        }
        _ => {}
    }
}

/// Produce a compact view of a long text value.
///
/// Recognizes a unified diff and collapses it to a diffstat-style summary
/// (file + hunk headers, with added/removed line counts). Otherwise keeps a
/// head+tail window with a byte-elision marker, which preserves both the start
/// and the end (unlike pure head truncation).
fn distill_text(text: &str, max_bytes: usize) -> String {
    if let Some(summary) = distill_unified_diff(text) {
        return summary;
    }
    head_tail(text, max_bytes)
}

/// If `text` looks like a unified diff, summarize it; else `None`.
fn distill_unified_diff(text: &str) -> Option<String> {
    let is_diff = text.starts_with("diff --git ")
        || text.contains("\ndiff --git ")
        || text.starts_with("--- ") && text.contains("\n+++ ") && text.contains("\n@@ ");
    if !is_diff {
        return None;
    }

    let mut kept = Vec::new();
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in text.lines() {
        if line.starts_with("diff --git ")
            || line.starts_with("+++ ")
            || line.starts_with("--- ")
            || line.starts_with("@@ ")
            || line.starts_with("rename ")
            || line.starts_with("new file")
            || line.starts_with("deleted file")
        {
            kept.push(line);
        } else if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }

    let header = format!("[diff distilled: +{added} / -{removed} lines — full diff via read_file]");
    let mut out =
        String::with_capacity(header.len() + kept.iter().map(|l| l.len() + 1).sum::<usize>());
    out.push_str(&header);
    out.push('\n');
    for line in kept {
        out.push_str(line);
        out.push('\n');
    }
    Some(out)
}

/// Keep a head+tail window of `text` within roughly `max_bytes`, on UTF-8
/// boundaries, with a marker noting how many bytes were elided.
fn head_tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let head_budget = max_bytes * 7 / 10;
    let tail_budget = max_bytes - head_budget;

    let mut head_end = head_budget.min(text.len());
    while head_end > 0 && !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = text.len().saturating_sub(tail_budget);
    while tail_start < text.len() && !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    if tail_start < head_end {
        tail_start = head_end;
    }

    let elided = tail_start.saturating_sub(head_end);
    format!(
        "{}\n… [{elided} bytes elided — full output via read_file] …\n{}",
        &text[..head_end],
        &text[tail_start..],
    )
}

/// Inject the recovery pointer fields into a distilled result value, mirroring
/// the contract of `tool_output_persistence` (`output_files` / `full_output`).
fn inject_pointer(result_value: &mut Value, stdout_path: &str, original_len: usize) {
    let note = annotate_truncated_output(
        "Tool output was distilled to fit the context window.",
        stdout_path,
        original_len,
    );
    let pointer = format!("/workspace{stdout_path}");

    if let Some(obj) = result_value.as_object_mut() {
        obj.insert("output_files".to_string(), json!([pointer]));
        obj.insert("full_output".to_string(), json!(stdout_path));
        obj.insert("distilled".to_string(), json!(true));
        obj.insert("distill_note".to_string(), json!(note));
    } else {
        // Non-object top level (array / string): wrap in an envelope so the
        // recovery pointer has somewhere to live.
        let distilled = std::mem::replace(result_value, Value::Null);
        *result_value = json!({
            "distilled_output": distilled,
            "output_files": [pointer],
            "full_output": stdout_path,
            "distilled": true,
            "distill_note": note,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = ToolOutputDistillationCapability;
        assert_eq!(cap.id(), "tool_output_distillation");
        assert!(!cap.post_tool_exec_hooks().is_empty());
        assert!(cap.dependencies().contains(&"session_file_system"));
    }

    #[test]
    fn test_head_tail_preserves_both_ends() {
        let text = format!("START{}END", "x".repeat(10_000));
        let out = head_tail(&text, 1024);
        assert!(out.starts_with("START"));
        assert!(out.ends_with("END"));
        assert!(out.contains("bytes elided"));
        assert!(out.len() < text.len());
    }

    #[test]
    fn test_head_tail_noop_when_small() {
        assert_eq!(head_tail("short", 1024), "short");
    }

    #[test]
    fn test_head_tail_utf8_safe() {
        // Multi-byte chars at the boundaries must not panic or split.
        let text = "日本語".repeat(5_000);
        let out = head_tail(&text, 1000);
        assert!(out.contains("bytes elided"));
        // Round-trips as valid UTF-8 (String guarantees it; assert non-empty ends).
        assert!(!out.is_empty());
    }

    #[test]
    fn test_distill_unified_diff_summarizes() {
        let diff = "diff --git a/x.rs b/x.rs\n--- a/x.rs\n+++ b/x.rs\n@@ -1,3 +1,4 @@\n-old line\n+new line one\n+new line two\n context\n";
        let summary = distill_unified_diff(diff).expect("recognized as diff");
        assert!(summary.contains("diff --git a/x.rs b/x.rs"));
        assert!(summary.contains("@@ -1,3 +1,4 @@"));
        assert!(summary.contains("+2 / -1 lines"));
        // Body content lines are dropped.
        assert!(!summary.contains("new line one"));
    }

    #[test]
    fn test_distill_non_diff_text_returns_none() {
        assert!(distill_unified_diff("just some prose without diff markers").is_none());
    }

    #[test]
    fn test_distill_value_samples_large_array() {
        let big: Vec<Value> = (0..1000)
            .map(|i| json!({"id": i, "name": format!("item-{i}")}))
            .collect();
        let mut value = json!({ "rows": big });
        let mut stats = DistillStats::default();
        distill_value(&mut value, 0, &mut stats);
        assert!(stats.changed);
        let rows = value["rows"].as_array().unwrap();
        // SAMPLE_ROWS kept + 1 elision marker
        assert_eq!(rows.len(), SAMPLE_ROWS + 1);
        assert!(rows.last().unwrap().as_str().unwrap().contains("more item"));
    }

    #[test]
    fn test_distill_value_clips_long_string_field() {
        let mut value = json!({ "body": "y".repeat(50_000), "ok": true });
        let mut stats = DistillStats::default();
        distill_value(&mut value, 0, &mut stats);
        assert!(stats.changed);
        assert!(value["body"].as_str().unwrap().len() < 50_000);
        // Small sibling field is untouched.
        assert_eq!(value["ok"], json!(true));
    }

    #[test]
    fn test_distill_value_noop_on_small() {
        let mut value = json!({ "a": 1, "b": "small" });
        let mut stats = DistillStats::default();
        distill_value(&mut value, 0, &mut stats);
        assert!(!stats.changed);
    }

    #[test]
    fn test_distill_value_small_array_not_sampled() {
        let mut value = json!({ "items": [1, 2, 3] });
        let mut stats = DistillStats::default();
        distill_value(&mut value, 0, &mut stats);
        assert!(!stats.changed);
        assert_eq!(value["items"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn test_inject_pointer_into_object() {
        let mut value = json!({ "content": "distilled" });
        inject_pointer(&mut value, "/outputs/abc.stdout", 50 * 1024);
        assert_eq!(
            value["output_files"],
            json!(["/workspace/outputs/abc.stdout"])
        );
        assert_eq!(value["full_output"], json!("/outputs/abc.stdout"));
        assert_eq!(value["distilled"], json!(true));
        assert!(
            value["distill_note"]
                .as_str()
                .unwrap()
                .contains("read_file")
        );
    }

    #[test]
    fn test_inject_pointer_wraps_non_object() {
        let mut value = json!(["a", "b"]);
        inject_pointer(&mut value, "/outputs/x.stdout", 1024);
        assert!(value.is_object());
        assert_eq!(value["distilled_output"], json!(["a", "b"]));
        assert_eq!(value["full_output"], json!("/outputs/x.stdout"));
    }

    #[test]
    fn test_depth_bound_does_not_panic() {
        // Build a deeply nested object beyond MAX_DEPTH.
        let mut value = json!("x".repeat(50_000));
        for _ in 0..50 {
            value = json!({ "next": value });
        }
        let mut stats = DistillStats::default();
        // Must terminate without panicking despite exceeding MAX_DEPTH.
        distill_value(&mut value, 0, &mut stats);
    }

    // --- Hook-level integration tests (after_exec end to end) ---

    use crate::error::Result;
    use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};
    use crate::traits::SessionFileSystem;
    use crate::typed_id::SessionId;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct MockFileStore {
        files: Mutex<HashMap<String, String>>,
        fail_writes: bool,
    }

    impl MockFileStore {
        fn content(&self, path: &str) -> Option<String> {
            self.files.lock().unwrap().get(path).cloned()
        }
    }

    #[async_trait]
    impl SessionFileSystem for MockFileStore {
        async fn read_file(&self, _s: SessionId, _p: &str) -> Result<Option<SessionFile>> {
            Ok(None)
        }
        async fn write_file(
            &self,
            _s: SessionId,
            path: &str,
            content: &str,
            _encoding: &str,
        ) -> Result<SessionFile> {
            if self.fail_writes {
                return Err(anyhow::anyhow!("write failed").into());
            }
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
        async fn delete_file(&self, _s: SessionId, _p: &str, _r: bool) -> Result<bool> {
            Ok(false)
        }
        async fn list_directory(&self, _s: SessionId, _p: &str) -> Result<Vec<FileInfo>> {
            Ok(vec![])
        }
        async fn stat_file(&self, _s: SessionId, _p: &str) -> Result<Option<FileStat>> {
            Ok(None)
        }
        async fn grep_files(
            &self,
            _s: SessionId,
            _pat: &str,
            _pp: Option<&str>,
        ) -> Result<Vec<GrepMatch>> {
            Ok(vec![])
        }
        async fn create_directory(&self, _s: SessionId, _p: &str) -> Result<FileInfo> {
            Err(anyhow::anyhow!("not implemented").into())
        }
    }

    fn mcp_tool_def(persist_output: bool) -> ToolDefinition {
        let mut hints = ToolHints::default();
        if persist_output {
            hints = hints.with_persist_output(true);
        }
        ToolDefinition::Builtin(BuiltinTool {
            name: "mcp__server__big_query".to_string(),
            display_name: None,
            description: "an mcp tool".to_string(),
            parameters: json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints,
            full_parameters: None,
        })
    }

    fn tool_call() -> ToolCall {
        ToolCall {
            id: "call_123".to_string(),
            name: "mcp__server__big_query".to_string(),
            arguments: json!({}),
        }
    }

    fn ctx_with_store(store: Arc<MockFileStore>) -> ToolContext {
        let mut ctx = ToolContext::new(SessionId::from(Uuid::nil()));
        ctx.file_store = Some(store);
        ctx
    }

    fn big_result() -> ToolResult {
        // A large array result, well over MIN_DISTILL_BYTES once serialized.
        let rows: Vec<Value> = (0..2000)
            .map(|i| json!({"id": i, "name": format!("row-number-{i}")}))
            .collect();
        ToolResult {
            tool_call_id: "call_123".to_string(),
            result: Some(json!({ "rows": rows })),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        }
    }

    #[tokio::test]
    async fn test_hook_distills_large_mcp_result_and_persists_original() {
        let store = Arc::new(MockFileStore::default());
        let ctx = ctx_with_store(store.clone());
        let mut result = big_result();

        DistillOutputHook
            .after_exec(&tool_call(), &mcp_tool_def(false), &mut result, &ctx)
            .await;

        let value = result.result.as_ref().unwrap();
        // Inline view is distilled: array sampled + pointer injected.
        assert_eq!(value["distilled"], json!(true));
        assert_eq!(
            value["output_files"],
            json!(["/workspace/outputs/call_123.stdout"])
        );
        let rows = value["rows"].as_array().unwrap();
        assert_eq!(rows.len(), SAMPLE_ROWS + 1);
        // Full original is recoverable from the VFS.
        let persisted = store
            .content("/outputs/call_123.stdout")
            .expect("original persisted");
        assert!(persisted.contains("row-number-1999"));
    }

    #[tokio::test]
    async fn test_hook_fallback_persists_large_unsampleable_result() {
        // A large object made entirely of sub-threshold fields: the shape walker
        // changes nothing, but the result is still large. The fallback must
        // bound it, persist the original, and inject a recovery pointer so it is
        // never head-truncated irrecoverably by the hard-limit hook.
        let store = Arc::new(MockFileStore::default());
        let ctx = ctx_with_store(store.clone());
        let mut map = serde_json::Map::new();
        for i in 0..2000 {
            map.insert(format!("field_{i}"), json!(format!("v{i}")));
        }
        let mut result = ToolResult {
            tool_call_id: "call_123".to_string(),
            result: Some(Value::Object(map)),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };

        DistillOutputHook
            .after_exec(&tool_call(), &mcp_tool_def(false), &mut result, &ctx)
            .await;

        let value = result.result.as_ref().unwrap();
        assert_eq!(value["distilled"], json!(true));
        assert_eq!(
            value["output_files"],
            json!(["/workspace/outputs/call_123.stdout"])
        );
        // Inline view is bounded (the wrapped head+tail string is small).
        assert!(value["distilled_output"].as_str().unwrap().len() < 8 * 1024);
        // Full original recoverable.
        let persisted = store
            .content("/outputs/call_123.stdout")
            .expect("original persisted");
        assert!(persisted.contains("field_1999"));
    }

    #[tokio::test]
    async fn test_hook_skips_persist_output_tools() {
        let store = Arc::new(MockFileStore::default());
        let ctx = ctx_with_store(store.clone());
        let mut result = big_result();

        DistillOutputHook
            .after_exec(&tool_call(), &mcp_tool_def(true), &mut result, &ctx)
            .await;

        // Untouched: exec/sandbox path is handled by budget + persistence.
        assert!(result.result.as_ref().unwrap().get("distilled").is_none());
        assert!(store.content("/outputs/call_123.stdout").is_none());
    }

    #[tokio::test]
    async fn test_hook_skips_small_result() {
        let store = Arc::new(MockFileStore::default());
        let ctx = ctx_with_store(store.clone());
        let mut result = ToolResult {
            tool_call_id: "call_123".to_string(),
            result: Some(json!({ "ok": true, "value": "small" })),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        };

        DistillOutputHook
            .after_exec(&tool_call(), &mcp_tool_def(false), &mut result, &ctx)
            .await;

        assert!(result.result.as_ref().unwrap().get("distilled").is_none());
        assert!(store.content("/outputs/call_123.stdout").is_none());
    }

    #[tokio::test]
    async fn test_hook_no_file_store_leaves_verbatim() {
        let mut ctx = ToolContext::new(SessionId::from(Uuid::nil()));
        ctx.file_store = None;
        let mut result = big_result();
        let before = result.result.clone();

        DistillOutputHook
            .after_exec(&tool_call(), &mcp_tool_def(false), &mut result, &ctx)
            .await;

        // No recoverability available → no lossy distillation.
        assert_eq!(result.result, before);
    }

    #[tokio::test]
    async fn test_hook_restores_original_when_persist_fails() {
        let store = Arc::new(MockFileStore {
            fail_writes: true,
            ..Default::default()
        });
        let ctx = ctx_with_store(store.clone());
        let mut result = big_result();
        let before = result.result.clone();

        DistillOutputHook
            .after_exec(&tool_call(), &mcp_tool_def(false), &mut result, &ctx)
            .await;

        // Persistence failed → verbatim original restored, never lossy-irrecoverable.
        assert_eq!(result.result, before);
        assert!(result.result.as_ref().unwrap().get("distilled").is_none());
    }

    #[tokio::test]
    async fn test_hook_skips_error_results() {
        let store = Arc::new(MockFileStore::default());
        let ctx = ctx_with_store(store.clone());
        let mut result = big_result();
        result.error = Some("boom".to_string());

        DistillOutputHook
            .after_exec(&tool_call(), &mcp_tool_def(false), &mut result, &ctx)
            .await;

        assert!(result.result.as_ref().unwrap().get("distilled").is_none());
    }
}
