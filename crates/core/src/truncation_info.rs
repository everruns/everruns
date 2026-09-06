// Reading-tool output truncation contract (EVE-339)
//
// Shared envelope for every tool in the reading-tool class (file readers,
// sandbox readers, DB query, web fetch, search). The contract is documented
// in `knowledge/execution/tool-execution.md` under "Reading-tool output contract".
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
/// Every reading tool (see `knowledge/execution/tool-execution.md`) must attach this block
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
    fn constructors_pin_complete_wire_shapes_and_pass_conformance() {
        for (info, wire) in [
            (
                TruncationInfo::not_truncated(128),
                json!({"truncated":false,"bytes_returned":128,"bytes_total":128,"reason":"size_cap"}),
            ),
            (
                TruncationInfo::not_truncated(0),
                json!({"truncated":false,"bytes_returned":0,"bytes_total":0,"reason":"size_cap"}),
            ),
            (
                TruncationInfo::with_resume(2, Some(8), 2, "offset=2", TruncationReason::LineCap),
                json!({"truncated":true,"bytes_returned":2,"bytes_total":8,"next_offset":2,"resume_hint":"offset=2","reason":"line_cap"}),
            ),
            (
                TruncationInfo::with_resume(3, None, 9, "offset=9", TruncationReason::ItemCap),
                json!({"truncated":true,"bytes_returned":3,"next_offset":9,"resume_hint":"offset=9","reason":"item_cap"}),
            ),
            (
                TruncationInfo::without_resume(4, None, TruncationReason::RowCap),
                json!({"truncated":true,"bytes_returned":4,"reason":"row_cap"}),
            ),
            (
                TruncationInfo::without_resume(5, Some(10), TruncationReason::ExecBudget),
                json!({"truncated":true,"bytes_returned":5,"bytes_total":10,"reason":"exec_budget"}),
            ),
        ] {
            assert_eq!(info.to_json(), wire);
            assert_eq!(
                serde_json::from_value::<TruncationInfo>(wire.clone()).unwrap(),
                info
            );
            assert_conforms("reader", &json!({"truncation":wire}));
        }
    }

    #[test]
    fn attach_inserts_under_truncation_key() {
        let mut target = json!({"content":"α","total_lines":1,"truncation":{"old":true}});
        TruncationInfo::not_truncated(2).attach(&mut target);
        assert_eq!(
            target,
            json!({"content":"α","total_lines":1,"truncation":{"truncated":false,"bytes_returned":2,"bytes_total":2,"reason":"size_cap"}})
        );
        for mut value in [
            json!(null),
            json!(false),
            json!(42),
            json!("text"),
            json!([1, 2]),
        ] {
            let original = value.clone();
            TruncationInfo::not_truncated(2).attach(&mut value);
            assert_eq!(value, original);
        }
    }

    #[test]
    fn reason_wire_values_are_stable() {
        for (reason, wire) in [
            (TruncationReason::SizeCap, "size_cap"),
            (TruncationReason::LineCap, "line_cap"),
            (TruncationReason::RowCap, "row_cap"),
            (TruncationReason::ExecBudget, "exec_budget"),
            (TruncationReason::ItemCap, "item_cap"),
        ] {
            assert_eq!(reason.as_str(), wire);
            assert_eq!(serde_json::to_value(reason).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<TruncationReason>(json!(wire)).unwrap(),
                reason
            );
        }
        assert!(serde_json::from_value::<TruncationReason>(json!("other")).is_err());
    }

    #[test]
    fn conformance_rejects_malformed_and_inconsistent_resume_metadata() {
        let block = |extra: Value| {
            let mut b = json!({"truncated":true,"bytes_returned":2,"reason":"size_cap"});
            b.as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            json!({"truncation":b})
        };
        for (response, expected) in [
            (json!([]), "response is not a JSON object"),
            (
                json!({"content":"x"}),
                "response missing required `truncation` block",
            ),
            (json!({"truncation":{}}), "`truncation` block malformed"),
            (
                block(json!({"reason":"unknown"})),
                "`truncation` block malformed",
            ),
            (
                block(json!({"next_offset":2})),
                "`next_offset` present but `resume_hint` missing",
            ),
            (
                block(json!({"resume_hint":"resume"})),
                "`resume_hint` present but `next_offset` missing",
            ),
            (
                block(json!({"truncated":false,"next_offset":2})),
                "`next_offset` set on a non-truncated response",
            ),
            (
                block(json!({"truncated":false,"resume_hint":"resume"})),
                "`resume_hint` set on a non-truncated response",
            ),
        ] {
            let panic = std::panic::catch_unwind(|| assert_conforms("reader", &response))
                .expect_err("invalid envelope must fail");
            let message = panic
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| panic.downcast_ref::<&str>().copied())
                .expect("text panic");
            assert!(message.starts_with("reader:"), "{message}");
            assert!(message.contains(expected), "{message}");
        }
    }
}
