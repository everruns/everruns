//! Shared scaffolding for capability tools.
//!
//! Capability tools historically hand-rolled argument extraction, typed-ID
//! parsing, store lookup, and error mapping. Because tool `execute*` methods
//! return `ToolExecutionResult` (not `Result<_, ToolExecutionResult>`), every
//! extracted argument forced a verbose
//! `let x = match f() { Ok(v) => v, Err(e) => return e };` block.
//!
//! These helpers return `Result<_, ToolExecutionResult>` so they compose with
//! `?` inside an `_impl` function that returns
//! `Result<ToolExecutionResult, ToolExecutionResult>`; the trait method then
//! calls `_impl(..).await.unwrap_or_else(|e| e)`.
//!
//! ## String extraction variants
//!
//! Call sites historically diverged on two axes: whether the returned `&str`
//! is trimmed, and whether emptiness is judged on the raw or trimmed value.
//! To unify without changing any caller's observable behavior we expose three
//! variants. Pick the one that matches the original call site:
//!
//! - [`require_str`] / [`get_str`]: reject *truly* empty (`""`); return the raw
//!   value, no trimming. Whitespace-only values pass through.
//! - [`require_str_trimmed`]: trim, reject empty-after-trim, return the trimmed
//!   slice. The strictest, default-preferred variant for new code.
//! - [`require_str_nonblank`]: reject blank (empty-after-trim) but return the
//!   raw, untrimmed value. For callers that validate non-blank yet preserve the
//!   original text (e.g. free-form instructions).

use crate::subagent_delegation::SubagentSessionDelegate;
use crate::tools::ToolExecutionResult;
use crate::{session_files::SessionFileSystem, tool_context::ToolContext};
use serde_json::Value;
use std::str::FromStr;

/// Optional string argument: returns the raw value if present and non-empty.
///
/// Filters out truly-empty strings (`""`) but does **not** trim; a
/// whitespace-only value is returned as-is.
pub fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Required string argument (raw, reject truly-empty).
///
/// Returns the raw value without trimming. Errors with
/// `Missing required parameter: {key}` when absent or empty.
pub fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolExecutionResult> {
    get_str(args, key).ok_or_else(|| {
        ToolExecutionResult::tool_error(format!("Missing required parameter: {key}"))
    })
}

/// Required string argument, trimmed.
///
/// Trims surrounding whitespace, rejects values that are empty after trimming,
/// and returns the trimmed slice. Preferred for new code.
pub fn require_str_trimmed<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolExecutionResult> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {key}"))
        })
}

/// Required string argument, validated non-blank but returned untrimmed.
///
/// Rejects values that are empty after trimming, but returns the original
/// (untrimmed) value. For callers that must preserve the exact text while still
/// rejecting blank input.
pub fn require_str_nonblank<'a>(
    args: &'a Value,
    key: &str,
) -> Result<&'a str, ToolExecutionResult> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(format!("Missing required parameter: {key}"))
        })
}

/// Parse an already-extracted string into a typed ID, mapping failure to a
/// tool error labeled `Invalid {label}: {raw}`.
pub fn parse_id<T: FromStr>(raw: &str, label: &str) -> Result<T, ToolExecutionResult> {
    raw.parse::<T>()
        .map_err(|_| ToolExecutionResult::tool_error(format!("Invalid {label}: {raw}")))
}

/// Extract a required string argument and parse it into a typed ID.
///
/// Combines [`require_str`] and [`parse_id`]: missing/empty errors with
/// `Missing required parameter: {key}`; a parse failure errors with
/// `Invalid {key}: {raw}`.
pub fn require_id<T: FromStr>(args: &Value, key: &str) -> Result<T, ToolExecutionResult> {
    let raw = require_str(args, key)?;
    parse_id(raw, key)
}

/// Extract the narrow subagent-session delegate from context or return a tool error.
pub fn get_subagent_delegate(
    context: &ToolContext,
) -> Result<&dyn SubagentSessionDelegate, ToolExecutionResult> {
    context
        .subagent_delegate
        .as_ref()
        .map(|store| store.as_ref())
        .ok_or_else(|| {
            ToolExecutionResult::tool_error(
                "Platform management not available: platform_store context is missing. Ensure the platform_management capability is enabled.",
            )
        })
}

/// Extract the session file store from context or return a tool error.
pub fn require_file_store(
    context: &ToolContext,
) -> Result<&dyn SessionFileSystem, ToolExecutionResult> {
    context
        .file_store
        .as_ref()
        .map(|store| store.as_ref())
        .ok_or_else(|| ToolExecutionResult::tool_error("File system not available"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_id::SessionId;
    use serde_json::json;

    #[test]
    fn string_extractors_preserve_their_distinct_whitespace_contracts() {
        for (args, raw, trimmed, nonblank) in [
            (json!({}), None, None, None),
            (json!({"value":null}), None, None, None),
            (json!({"value":42}), None, None, None),
            (json!({"value":true}), None, None, None),
            (json!({"value":[]}), None, None, None),
            (json!({"value":{}}), None, None, None),
            (json!({"value":""}), None, None, None),
            (json!({"value":" \t\n"}), Some(" \t\n"), None, None),
            (
                json!({"value":"\u{2003}α\u{2003}"}),
                Some("\u{2003}α\u{2003}"),
                Some("α"),
                Some("\u{2003}α\u{2003}"),
            ),
            (json!({"value":"x"}), Some("x"), Some("x"), Some("x")),
        ] {
            assert_eq!(get_str(&args, "value"), raw, "{args}");
            for (actual, expected) in [
                (require_str(&args, "value"), raw),
                (require_str_trimmed(&args, "value"), trimmed),
                (require_str_nonblank(&args, "value"), nonblank),
            ] {
                match (actual, expected) {
                    (Ok(value), Some(expected)) => assert_eq!(value, expected, "{args}"),
                    (Err(ToolExecutionResult::ToolError(message)), None) => {
                        assert_eq!(message, "Missing required parameter: value")
                    }
                    (actual, expected) => panic!("{args}: expected {expected:?}, got {actual:?}"),
                }
            }
        }
    }

    #[test]
    fn required_id_preserves_identity_and_distinguishes_missing_from_invalid() {
        let wire = "session_12345678123456789abc123456789abc";
        let expected = uuid::Uuid::parse_str("12345678-1234-5678-9abc-123456789abc").unwrap();
        assert_eq!(
            require_id::<SessionId>(&json!({"session_id":wire}), "session_id")
                .unwrap()
                .uuid(),
            expected
        );
        assert_eq!(
            parse_id::<SessionId>(wire, "session").unwrap().uuid(),
            expected
        );
        for args in [
            json!({}),
            json!({"session_id":""}),
            json!({"session_id":null}),
            json!({"session_id":42}),
        ] {
            assert!(
                matches!(require_id::<SessionId>(&args,"session_id"),Err(ToolExecutionResult::ToolError(message)) if message == "Missing required parameter: session_id")
            );
        }
        for raw in [
            "not-a-valid-id",
            "agent_12345678123456789abc123456789abc",
            " ",
            "session_1234",
        ] {
            assert!(
                matches!(require_id::<SessionId>(&json!({"session_id":raw}),"session_id"),Err(ToolExecutionResult::ToolError(message)) if message == format!("Invalid session_id: {raw}"))
            );
        }
    }

    #[test]
    fn parse_id_uses_custom_label() {
        let err = parse_id::<SessionId>("garbage", "harness id").unwrap_err();
        match err {
            ToolExecutionResult::ToolError(msg) => {
                assert_eq!(msg, "Invalid harness id: garbage");
            }
            other => panic!("expected ToolError, got {other:?}"),
        }
    }
}
