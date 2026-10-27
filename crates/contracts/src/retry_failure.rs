//! What an exhausted provider recovery keeps of the failure that started it.
//!
//! A retry budget can expire on an attempt that never answered (a hung
//! request, a lost executor), and the error built at that point would
//! otherwise name only the budget. [`RetryFailure`] is the last classified
//! provider failure, kept beside retry metadata and in the durable retry
//! checkpoint so a 429 shared-pool refusal, an outage and a network timeout
//! stay distinguishable after the window closes (EVE-1318).
//!
//! THREAT[TM-LLM-003]: provider error text can echo request headers or
//! user content. Only the Everruns classification and short identifiers that
//! pass a strict character allowlist are kept, never the provider message.

use serde::{Deserialize, Serialize};

use crate::error::AgentLoopError;
use crate::llm_error::LlmErrorKind;

/// Largest identifier kept from a provider failure.
const MAX_IDENTIFIER_BYTES: usize = 64;

/// Bytes of provider text scanned for a `limit_source`; real error payloads
/// are a few hundred bytes.
const MAX_SCAN_BYTES: usize = 64 * 1024;

/// Bounded, secret-free classification of the last provider failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryFailure {
    pub kind: LlmErrorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// The provider's machine-readable error code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Which limit refused the request (OpenRouter `metadata.limit_source`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_source: Option<String>,
    /// Transport cause when no HTTP answer arrived: `stream_stall`, `timeout`
    /// or `connection`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
}

impl RetryFailure {
    /// Classify a provider failure; `None` for anything that is not one.
    pub fn from_error(error: &AgentLoopError) -> Option<Self> {
        let AgentLoopError::Llm(error) = error else {
            return None;
        };
        let scanned = prefix(&error.message, MAX_SCAN_BYTES);
        Some(
            Self {
                kind: error.kind,
                status: error.status,
                code: error.code.clone(),
                limit_source: json_string_field(scanned, "limit_source").map(str::to_owned),
                transport: error
                    .status
                    .is_none()
                    .then(|| transport_cause(scanned))
                    .flatten()
                    .map(str::to_owned),
            }
            .sanitized(),
        )
    }

    /// Re-apply the bounds to a value read back from durable state.
    #[must_use]
    pub fn sanitized(mut self) -> Self {
        for field in [&mut self.code, &mut self.limit_source, &mut self.transport] {
            *field = field.take().filter(|value| is_identifier(value));
        }
        self
    }
}

impl std::fmt::Display for RetryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(kind_label(self.kind))?;
        let mut details = Vec::new();
        if let Some(status) = self.status {
            details.push(format!("HTTP {status}"));
        }
        if let Some(code) = &self.code {
            details.push(format!("code {code}"));
        }
        if let Some(source) = &self.limit_source {
            details.push(format!("limit_source {source}"));
        }
        if let Some(transport) = &self.transport {
            details.push(transport.clone());
        }
        if !details.is_empty() {
            write!(f, " ({})", details.join(", "))?;
        }
        Ok(())
    }
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

fn prefix(text: &str, max: usize) -> &str {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// First `"name": "value"` in provider text, without parsing it as JSON: the
/// body is often embedded in a longer driver message.
fn json_string_field<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("\"{name}\"");
    let rest = &text[text.find(&key)? + key.len()..];
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
    } else if lower.contains("connection") || lower.contains("connect error") {
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
        let failure = RetryFailure::from_error(&AgentLoopError::llm_http(429, body, body)).unwrap();
        assert_eq!(
            failure.to_string(),
            "rate_limited (HTTP 429, code engine_overloaded, limit_source upstream_provider_shared_pool)"
        );
        let stored = serde_json::to_string(&failure).unwrap();
        assert!(!stored.contains("sk-secret"), "{stored}");

        let timeout = RetryFailure::from_error(&AgentLoopError::llm_kind(
            LlmErrorKind::Unavailable,
            "error sending request: operation timed out",
        ))
        .unwrap();
        assert_eq!(timeout.to_string(), "unavailable (timeout)");
        assert!(RetryFailure::from_error(&AgentLoopError::store("x")).is_none());
    }

    #[test]
    fn rejects_identifiers_outside_the_allowlist_or_bound() {
        let long = "a".repeat(MAX_IDENTIFIER_BYTES + 1);
        let failure = RetryFailure {
            kind: LlmErrorKind::RateLimited,
            status: Some(429),
            code: Some("Bearer sk-secret".into()),
            limit_source: Some(long),
            transport: Some("timeout".into()),
        }
        .sanitized();
        assert_eq!(failure.code, None);
        assert_eq!(failure.limit_source, None);
        assert_eq!(failure.transport.as_deref(), Some("timeout"));
        let text = format!(r#"{{"limit_source":"{}"}}"#, "x y");
        assert_eq!(json_string_field(&text, "limit_source"), None);
    }
}
