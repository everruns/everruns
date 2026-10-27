//! What an exhausted provider recovery keeps of the failure that started it.
//!
//! A retry budget can expire on an attempt that never answered (a hung
//! request, a lost executor), and the error built at that point would
//! otherwise name only the budget. [`retained`] keeps the last classified
//! provider failure beside retry metadata and in the durable retry
//! checkpoint, so a 429 shared-pool refusal, an outage and a network timeout
//! stay distinguishable after the window closes (EVE-1318).
//!
//! THREAT[TM-LLM-003]: provider error text can echo request headers or user
//! content. Only the Everruns classification, the status and short
//! identifiers that pass a strict character allowlist are kept, never the
//! provider message.
//!
//! Size: this runs in the Engine Worker Wasm, whose upload has a fixed
//! budget. The cause is one summary string (no new serde types), built with
//! plain pushes (no `format!`) and scanned without allocating.

use crate::error::AgentLoopError;
use crate::llm_error::LlmErrorKind;

/// Largest identifier kept from a provider failure.
const MAX_IDENTIFIER_BYTES: usize = 64;

/// Bounded, secret-free summary of a provider failure, such as
/// `rate_limited (HTTP 429, code engine_overloaded)`: its kind label, status,
/// allowlisted code and `limit_source`, nothing else. A network failure has
/// no status, which tells it apart from an HTTP outage.
/// `None` for anything that is not a provider failure.
pub fn retained(error: &AgentLoopError) -> Option<String> {
    let AgentLoopError::Llm(error) = error else {
        return None;
    };
    let mut summary = String::from(match error.kind {
        LlmErrorKind::RateLimited => "rate_limited",
        LlmErrorKind::Unavailable => "unavailable",
        _ => "other",
    });
    let mut open = false;
    let mut detail = |name: &str, value: &str| {
        summary.push_str(if open { ", " } else { " (" });
        summary.push_str(name);
        summary.push_str(value);
        open = true;
    };
    if let Some(status) = error.status {
        detail("HTTP ", &u32::from(status).to_string());
    }
    if let Some(code) = error.code.as_deref().filter(|code| is_identifier(code)) {
        detail("code ", code);
    }
    if let Some(source) = limit_source(&error.message) {
        detail("limit_source ", source);
    }
    if open {
        summary.push(')');
    }
    Some(summary)
}

/// The kind a retained summary names, for the terminal error. Only transient
/// failures reach a retry; anything but a rate limit reads back as
/// `Unavailable`, the kind this error always had before the cause was kept.
pub fn exhausted_kind(summary: &str) -> LlmErrorKind {
    if summary.starts_with("rate_limited") {
        LlmErrorKind::RateLimited
    } else {
        LlmErrorKind::Unavailable
    }
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

/// First compact `"limit_source":"value"` in provider text, without parsing
/// JSON: the body is often embedded in a longer driver message.
fn limit_source(text: &str) -> Option<&str> {
    const KEY: &str = "\"limit_source\":\"";
    let rest = &text[text.find(KEY)? + KEY.len()..];
    let value = &rest[..rest.find('"')?];
    is_identifier(value).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm_error::LlmError;

    #[test]
    fn keeps_classification_and_identifiers_but_never_provider_text() {
        let body = r#"{"error":{"code":"engine_overloaded","message":"Bearer sk-secret","metadata":{"limit_source":"upstream_provider_shared_pool"}}}"#;
        let kept = retained(&AgentLoopError::llm_http(429, body, body)).unwrap();
        assert_eq!(
            kept,
            "rate_limited (HTTP 429, code engine_overloaded, limit_source upstream_provider_shared_pool)"
        );
        assert_eq!(exhausted_kind(&kept), LlmErrorKind::RateLimited);

        let timeout = retained(&AgentLoopError::llm_kind(
            LlmErrorKind::Unavailable,
            "error sending request: operation timed out",
        ))
        .unwrap();
        assert_eq!(timeout, "unavailable");
        assert_eq!(exhausted_kind(&timeout), LlmErrorKind::Unavailable);
        assert_eq!(exhausted_kind("other"), LlmErrorKind::Unavailable);
        assert!(retained(&AgentLoopError::store("x")).is_none());
    }

    #[test]
    fn rejects_identifiers_outside_the_allowlist_or_bound() {
        let mut error = LlmError::new(
            LlmErrorKind::RateLimited,
            format!(
                r#"{{"limit_source":"{}"}}"#,
                "a".repeat(MAX_IDENTIFIER_BYTES + 1)
            ),
        );
        error.code = Some("Bearer sk-secret".into());
        let kept = retained(&AgentLoopError::Llm(error)).unwrap();
        assert_eq!(kept, "rate_limited");
        assert_eq!(limit_source(r#"{"limit_source":"x y"}"#), None);
    }
}
