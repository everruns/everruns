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
//! provider message. The retained value reuses [`LlmError`] (boxed, so retry
//! metadata held across awaits stays small) instead of a new serialized type,
//! which keeps the Engine Worker Wasm upload within budget.

use crate::error::AgentLoopError;
use crate::llm_error::{LlmError, LlmErrorKind};

/// Largest identifier kept from a provider failure.
const MAX_IDENTIFIER_BYTES: usize = 64;

/// Bytes of provider text scanned for a `limit_source`; real error payloads
/// are a few hundred bytes.
const MAX_SCAN_BYTES: usize = 64 * 1024;

/// Bounded, secret-free copy of a provider failure: its kind, status and an
/// allowlisted code, with a message that names only those and the safe
/// `limit_source` / transport cause. `None` for anything that is not one.
pub fn retained(error: &AgentLoopError) -> Option<Box<LlmError>> {
    let AgentLoopError::Llm(error) = error else {
        return None;
    };
    let mut end = error.message.len().min(MAX_SCAN_BYTES);
    while !error.message.is_char_boundary(end) {
        end -= 1;
    }
    let scanned = &error.message[..end];
    let code = error.code.as_deref().filter(|code| is_identifier(code));
    let mut details: Vec<String> = Vec::new();
    if let Some(status) = error.status {
        details.push(format!("HTTP {status}"));
    }
    if let Some(code) = code {
        details.push(format!("code {code}"));
    }
    if let Some(source) = limit_source(scanned) {
        details.push(format!("limit_source {source}"));
    }
    if error.status.is_none()
        && let Some(cause) = transport_cause(scanned)
    {
        details.push(cause.to_owned());
    }
    let mut summary = String::from(kind_label(error.kind));
    if !details.is_empty() {
        summary.push_str(" (");
        summary.push_str(&details.join(", "));
        summary.push(')');
    }
    let mut kept = LlmError::new(error.kind, summary);
    kept.status = error.status;
    kept.code = code.map(str::to_owned);
    Some(Box::new(kept))
}

fn kind_label(kind: LlmErrorKind) -> &'static str {
    match kind {
        LlmErrorKind::Authentication => "authentication",
        LlmErrorKind::QuotaExhausted => "quota_exhausted",
        LlmErrorKind::BillingPressure { .. } => "billing_pressure",
        LlmErrorKind::RateLimited => "rate_limited",
        LlmErrorKind::Unavailable => "unavailable",
        LlmErrorKind::AttestationRequired => "attestation_required",
        LlmErrorKind::InvalidRequest => "invalid_request",
        LlmErrorKind::MalformedResponse => "malformed_response",
        LlmErrorKind::Other => "other",
    }
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
        assert_eq!(kept.kind, LlmErrorKind::RateLimited);
        assert_eq!(kept.status, Some(429));
        assert_eq!(
            kept.message,
            "rate_limited (HTTP 429, code engine_overloaded, limit_source upstream_provider_shared_pool)"
        );
        let stored = serde_json::to_string(&kept).unwrap();
        assert!(!stored.contains("sk-secret"), "{stored}");

        let timeout = retained(&AgentLoopError::llm_kind(
            LlmErrorKind::Unavailable,
            "error sending request: operation timed out",
        ))
        .unwrap();
        assert_eq!(timeout.message, "unavailable (timeout)");
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
        assert_eq!(kept.code, None);
        assert_eq!(kept.message, "rate_limited");
        assert_eq!(limit_source(r#"{"limit_source":"x y"}"#), None);
    }
}
