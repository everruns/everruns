// Reading-tool output truncation contract (EVE-339)
//
// Shared envelope for every tool in the reading-tool class (file readers,
// sandbox readers, DB query, web fetch, search). The contract is documented
// in `specs/tool-execution.md` under "Reading-tool output contract".
//
// Design decisions:
// - One struct (`TruncationInfo`) covers the three interesting cases:
//   not-truncated, truncated-without-resume, truncated-with-resume.
// - `reason` is a stable machine-readable enum so LLMs can branch without
//   string-matching human-readable markers.
// - `next_offset` is populated ONLY when the owning tool supports in-place
//   resume. Tools without resume (e.g. browserless without ?range, exec
//   output after priority-aware truncation) set it to `None`.
// - Attached to tool responses as a `"truncation": { ... }` object via
//   `TruncationInfo::attach`. Existing flat fields (e.g. `truncated: bool`,
//   `total_lines`) stay in place for back-compat; the envelope is additive.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Machine-readable reason for a truncation. Stable wire values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruncationReason {
    /// Source exceeded a byte cap (e.g. `read_file` 50 KB hard cap,
    /// `browserless_content` 100 KB cap).
    SizeCap,
    /// Source exceeded a line cap (e.g. `read_file` default 2000 lines,
    /// `grep_files` match-count limit).
    LineCap,
    /// Source exceeded a row cap (e.g. `sqldb_query` 1000-row cap).
    RowCap,
    /// Source exceeded an exec verbosity budget
    /// (`silent`/`concise`/`normal`/`verbose`).
    ExecBudget,
    /// Source exceeded a listing/item-count cap (e.g. `list_directory`).
    ItemCap,
}

impl TruncationReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SizeCap => "size_cap",
            Self::LineCap => "line_cap",
            Self::RowCap => "row_cap",
            Self::ExecBudget => "exec_budget",
            Self::ItemCap => "item_cap",
        }
    }
}

/// Structured truncation metadata for reading-tool responses.
///
/// Every reading tool (see `specs/tool-execution.md`) must attach this block
/// to its response so LLM callers can:
/// 1. Detect partial output without regex-matching human markers.
/// 2. Know *why* the cut happened (size / line / row / budget / item cap).
/// 3. Resume from the next offset when the tool supports in-place resume, or
///    discover the documented fallback otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TruncationInfo {
    pub truncated: bool,

    /// Bytes of the response's primary content returned to the caller.
    /// "Primary content" means the field a caller actually consumes
    /// (e.g. `content` for `read_file` / `browserless_content`, `rows` for
    /// `sql_query`, `entries` for `list_directory`, `matches` for
    /// `grep_files`). It is not the serialized size of the wrapping object.
    pub bytes_returned: usize,

    /// Total bytes of the untruncated source. `None` when unknown (e.g.
    /// streaming/search results).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_total: Option<usize>,

    /// Offset the caller should pass back to the same tool to continue
    /// reading. Populated only when the tool supports in-place resume.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,

    /// Human-readable nudge for the LLM describing how to resume. Always
    /// paired with `next_offset` when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_hint: Option<String>,

    /// Machine-readable reason code. Always present so callers can branch
    /// on it even when `truncated` is `false` (the reason is unused in
    /// that case but kept for schema stability).
    pub reason: TruncationReason,
}

impl TruncationInfo {
    /// Response fully contains the source; no cut was made.
    pub fn not_truncated(bytes_returned: usize) -> Self {
        Self {
            truncated: false,
            bytes_returned,
            bytes_total: Some(bytes_returned),
            next_offset: None,
            resume_hint: None,
            reason: TruncationReason::SizeCap,
        }
    }

    /// Response was cut and the caller can resume by passing `next_offset`
    /// back to the same tool.
    pub fn with_resume(
        bytes_returned: usize,
        bytes_total: Option<usize>,
        next_offset: u64,
        resume_hint: impl Into<String>,
        reason: TruncationReason,
    ) -> Self {
        Self {
            truncated: true,
            bytes_returned,
            bytes_total,
            next_offset: Some(next_offset),
            resume_hint: Some(resume_hint.into()),
            reason,
        }
    }

    /// Response was cut and in-place resume is not supported. The caller
    /// must fall back to the documented per-tool strategy (e.g. narrower
    /// `WHERE` for SQL, VFS file reads for exec output).
    pub fn without_resume(
        bytes_returned: usize,
        bytes_total: Option<usize>,
        reason: TruncationReason,
    ) -> Self {
        Self {
            truncated: true,
            bytes_returned,
            bytes_total,
            next_offset: None,
            resume_hint: None,
            reason,
        }
    }

    /// Attach this block to a JSON object under the `truncation` key.
    /// No-op if `target` is not an object.
    pub fn attach(&self, target: &mut Value) {
        if let Some(obj) = target.as_object_mut() {
            obj.insert(
                "truncation".to_string(),
                serde_json::to_value(self).expect("TruncationInfo serializes"),
            );
        }
    }

    /// Serialize as a JSON `Value` for manual insertion.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("TruncationInfo serializes")
    }
}

/// Assert that a reading-tool response carries a well-formed `truncation`
/// block per the contract. Intended for use in tool-specific tests and the
/// cross-tool conformance harness.
///
/// Panics with a descriptive message on violation.
pub fn assert_conforms(tool_name: &str, response: &Value) {
    let obj = response
        .as_object()
        .unwrap_or_else(|| panic!("{tool_name}: response is not a JSON object"));

    let block = obj
        .get("truncation")
        .unwrap_or_else(|| panic!("{tool_name}: response missing required `truncation` block"));

    let parsed: TruncationInfo = serde_json::from_value(block.clone())
        .unwrap_or_else(|e| panic!("{tool_name}: `truncation` block malformed: {e}"));

    if parsed.truncated {
        if parsed.next_offset.is_some() {
            assert!(
                parsed.resume_hint.is_some(),
                "{tool_name}: `next_offset` present but `resume_hint` missing"
            );
        }
        if parsed.resume_hint.is_some() {
            assert!(
                parsed.next_offset.is_some(),
                "{tool_name}: `resume_hint` present but `next_offset` missing"
            );
        }
    } else {
        assert!(
            parsed.next_offset.is_none(),
            "{tool_name}: `next_offset` set on a non-truncated response"
        );
        assert!(
            parsed.resume_hint.is_none(),
            "{tool_name}: `resume_hint` set on a non-truncated response"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn not_truncated_shape() {
        let info = TruncationInfo::not_truncated(128);
        let v = info.to_json();
        assert_eq!(v["truncated"], json!(false));
        assert_eq!(v["bytes_returned"], json!(128));
        assert_eq!(v["bytes_total"], json!(128));
        assert!(v.get("next_offset").is_none());
        assert!(v.get("resume_hint").is_none());
        assert_eq!(v["reason"], json!("size_cap"));
    }

    #[test]
    fn with_resume_shape() {
        let info = TruncationInfo::with_resume(
            49_512,
            Some(184_221),
            49_512,
            "call read_file with offset=49512",
            TruncationReason::SizeCap,
        );
        let v = info.to_json();
        assert_eq!(v["truncated"], json!(true));
        assert_eq!(v["bytes_returned"], json!(49_512));
        assert_eq!(v["bytes_total"], json!(184_221));
        assert_eq!(v["next_offset"], json!(49_512));
        assert!(v["resume_hint"].as_str().unwrap().contains("offset=49512"));
        assert_eq!(v["reason"], json!("size_cap"));
    }

    #[test]
    fn without_resume_shape() {
        let info = TruncationInfo::without_resume(1_000, None, TruncationReason::RowCap);
        let v = info.to_json();
        assert_eq!(v["truncated"], json!(true));
        assert!(v.get("bytes_total").is_none());
        assert!(v.get("next_offset").is_none());
        assert!(v.get("resume_hint").is_none());
        assert_eq!(v["reason"], json!("row_cap"));
    }

    #[test]
    fn attach_inserts_under_truncation_key() {
        let mut target = json!({ "content": "abc", "total_lines": 3 });
        TruncationInfo::not_truncated(3).attach(&mut target);
        assert_eq!(target["truncation"]["truncated"], json!(false));
        assert_eq!(target["content"], json!("abc"));
        assert_eq!(target["total_lines"], json!(3));
    }

    #[test]
    fn reason_wire_values_are_stable() {
        assert_eq!(TruncationReason::SizeCap.as_str(), "size_cap");
        assert_eq!(TruncationReason::LineCap.as_str(), "line_cap");
        assert_eq!(TruncationReason::RowCap.as_str(), "row_cap");
        assert_eq!(TruncationReason::ExecBudget.as_str(), "exec_budget");
        assert_eq!(TruncationReason::ItemCap.as_str(), "item_cap");
    }

    #[test]
    fn assert_conforms_accepts_valid() {
        let mut response = json!({});
        TruncationInfo::with_resume(100, Some(500), 100, "resume", TruncationReason::LineCap)
            .attach(&mut response);
        assert_conforms("fake_tool", &response);
    }

    #[test]
    #[should_panic(expected = "missing required `truncation` block")]
    fn assert_conforms_rejects_missing() {
        let response = json!({"content": "hi"});
        assert_conforms("fake_tool", &response);
    }

    #[test]
    #[should_panic(expected = "`next_offset` set on a non-truncated response")]
    fn assert_conforms_rejects_offset_without_truncation() {
        let response = json!({
            "truncation": {
                "truncated": false,
                "bytes_returned": 10,
                "bytes_total": 10,
                "next_offset": 10,
                "reason": "size_cap",
            }
        });
        assert_conforms("fake_tool", &response);
    }
}
