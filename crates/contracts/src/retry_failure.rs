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
//! provider message. The retained value is one short summary string, read back
//! by [`exhausted_kind`] and [`exhausted_status`]: a string adds no serde code
//! to the Engine Worker Wasm, whose upload has a fixed budget.

use crate::error::AgentLoopError;
#[cfg(test)]
use crate::llm_error::LlmError;
use crate::llm_error::LlmErrorKind;

/// Largest identifier kept from a provider failure.
const MAX_IDENTIFIER_BYTES: usize = 64;

/// Bytes of provider text scanned for a `limit_source`; real error payloads
/// are a few hundred bytes.
const MAX_SCAN_BYTES: usize = 64 * 1024;

/// Bounded, secret-free summary of a provider failure, such as
/// `rate_limited (HTTP 429, code engine_overloaded)`: its kind label, status,
/// allowlisted code, `limit_source` and transport cause, nothing else.
/// `None` for anything that is not a provider failure.
pub fn retained(error: &AgentLoopError) -> Option<String> {
    let AgentLoopError::Llm(error) = error else {
        return None;
    };
    let mut end = error.message.len().min(MAX_SCAN_BYTES);
    while !error.message.is_char_boundary(end) {
        end -= 1;
    }
    let scanned = &error.message[..end];
    let mut summary = String::from(
        KINDS
            .iter()
            .find(|(_, kind)| *kind == error.kind)
            .map_or("other", |(label, _)| label),
    );
    // Plain pushes rather than `format!`: this runs in the size-budgeted Wasm.
    let mut details = String::new();
    let mut detail = |name: &str, value: &str| {
        details.push_str(if details.is_empty() { " (" } else { ", " });
        details.push_str(name);
        details.push_str(value);
    };
    if let Some(status) = error.status {
        detail("HTTP ", &status.to_string());
    }
    if let Some(code) = error.code.as_deref().filter(|code| is_identifier(code)) {
        detail("code ", code);
    }
    if let Some(source) = limit_source(scanned) {
        detail("limit_source ", source);
    }
    if error.status.is_none()
        && let Some(cause) = transport_cause(scanned)
    {
        detail(cause, "");
    }
    if !details.is_empty() {
        summary.push_str(&details);
        summary.push(')');
    }
    Some(summary)
}

/// Kinds a retained summary names. Only transient failures reach a retry;
/// any other kind reads back as `Unavailable`, as before this was kept.
const KINDS: [(&str, LlmErrorKind); 4] = [
    ("rate_limited", LlmErrorKind::RateLimited),
    ("unavailable", LlmErrorKind::Unavailable),
    ("malformed_response", LlmErrorKind::MalformedResponse),
    ("other", LlmErrorKind::Other),
];

/// The kind a retained summary names, for the terminal error.
pub fn exhausted_kind(summary: &str) -> LlmErrorKind {
    let label = summary.split(' ').next().unwrap_or_default();
    KINDS
        .iter()
        .find(|(name, _)| *name == label)
        .map_or(LlmErrorKind::Unavailable, |(_, kind)| *kind)
}

/// The HTTP status a retained summary names, for the terminal error.
pub fn exhausted_status(summary: &str) -> Option<u16> {
    let rest = &summary.as_bytes()[summary.find("(HTTP ")? + 6..];
    let digits = rest.get(..3).filter(|d| d.iter().all(u8::is_ascii_digit))?;
    Some(digits.iter().fold(0, |n, d| n * 10 + u16::from(d - b'0')))
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
}

/// First `"limit_source": "value"` in provider text, without parsing JSON:
/// the body is often embedded in a longer driver message.
fn limit_source(text: &str) -> Option<&str> {
    const KEY: &str = "\"limit_source\"";
    let rest = &text[text.find(KEY)? + KEY.len()..];
    let rest = rest.trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let value = &rest[..rest.find('"')?];
    is_identifier(value).then_some(value)
}

fn transport_cause(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("stream stall") {
        Some("stream_stall")
    } else if lower.contains("timed out") || lower.contains("timeout") {
        Some("timeout")
    } else if lower.contains("connection") {
        Some("connection")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_classification_and_identifiers_but_never_provider_text() {
        let body = r#"{"error":{"code":"engine_overloaded","message":"Bearer sk-secret","metadata":{"limit_source": "upstream_provider_shared_pool"}}}"#;
        let kept = retained(&AgentLoopError::llm_http(429, body, body)).unwrap();
        assert_eq!(
            kept,
            "rate_limited (HTTP 429, code engine_overloaded, limit_source upstream_provider_shared_pool)"
        );
        assert_eq!(exhausted_kind(&kept), LlmErrorKind::RateLimited);
        assert_eq!(exhausted_status(&kept), Some(429));

        let timeout = retained(&AgentLoopError::llm_kind(
            LlmErrorKind::Unavailable,
            "error sending request: operation timed out",
        ))
        .unwrap();
        assert_eq!(timeout, "unavailable (timeout)");
        assert_eq!(exhausted_kind(&timeout), LlmErrorKind::Unavailable);
        assert_eq!(exhausted_status(&timeout), None);
        assert_eq!(exhausted_kind("authentication"), LlmErrorKind::Unavailable);
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
