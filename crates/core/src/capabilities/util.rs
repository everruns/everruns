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
    fn get_str_filters_empty_but_not_whitespace() {
        let args = json!({ "a": "", "b": "  ", "c": "x" });
        assert_eq!(get_str(&args, "a"), None);
        assert_eq!(get_str(&args, "b"), Some("  "));
        assert_eq!(get_str(&args, "c"), Some("x"));
        assert_eq!(get_str(&args, "missing"), None);
    }

    #[test]
    fn require_str_rejects_truly_empty_keeps_raw() {
        let args = json!({ "a": "", "b": "  x  ", "c": "x" });
        assert!(require_str(&args, "a").is_err());
        assert!(require_str(&args, "missing").is_err());
        // No trimming: whitespace preserved.
        assert_eq!(require_str(&args, "b").unwrap(), "  x  ");
        assert_eq!(require_str(&args, "c").unwrap(), "x");
    }

    #[test]
    fn require_str_trimmed_trims_and_rejects_blank() {
        let args = json!({ "blank": "   ", "padded": "  x  ", "ok": "y" });
        assert!(require_str_trimmed(&args, "blank").is_err());
        assert!(require_str_trimmed(&args, "missing").is_err());
        assert_eq!(require_str_trimmed(&args, "padded").unwrap(), "x");
        assert_eq!(require_str_trimmed(&args, "ok").unwrap(), "y");
    }

    #[test]
    fn require_str_nonblank_rejects_blank_keeps_raw() {
        let args = json!({ "blank": "   ", "padded": "  x  ", "ok": "y" });
        assert!(require_str_nonblank(&args, "blank").is_err());
        assert!(require_str_nonblank(&args, "missing").is_err());
        // Validates non-blank but returns the untrimmed value.
        assert_eq!(require_str_nonblank(&args, "padded").unwrap(), "  x  ");
        assert_eq!(require_str_nonblank(&args, "ok").unwrap(), "y");
    }

    #[test]
    fn require_id_parse_failure_maps_to_invalid_key() {
        let args = json!({ "session_id": "not-a-valid-id" });
        let err = require_id::<SessionId>(&args, "session_id").unwrap_err();
        match err {
            ToolExecutionResult::ToolError(msg) => {
                assert_eq!(msg, "Invalid session_id: not-a-valid-id");
            }
            other => panic!("expected ToolError, got {other:?}"),
        }
    }

    #[test]
    fn require_id_missing_maps_to_missing_parameter() {
        let args = json!({});
        let err = require_id::<SessionId>(&args, "session_id").unwrap_err();
        match err {
            ToolExecutionResult::ToolError(msg) => {
                assert_eq!(msg, "Missing required parameter: session_id");
            }
            other => panic!("expected ToolError, got {other:?}"),
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
