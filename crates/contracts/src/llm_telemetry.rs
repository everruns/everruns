//! Structured logs for LLM edge cases we want to *measure* before changing.
//!
//! Why: several provider edge cases were silent. A driver discarded tool calls
//! from a response cut off at the output limit, a truncated call could still run
//! with `{}` arguments, a context-overflow error the classifier did not recognize
//! surfaced as a generic failure, and a retry's `Retry-After` value was never
//! logged. Production ships logs, not traces, so each case is one log line on the
//! `everruns::llm_telemetry` target with stable field names an operator can
//! count and alert on.
//!
//! Decision: observation only. Nothing here changes what a driver does with a
//! truncated response or which errors are retried. The fix for truncated tool
//! calls landed separately: drivers now drop a cut-off call instead of running
//! it with `{}`, and the engine's output-truncation gate decides what happens
//! next; `warn_truncation_gate` records each time it acts.

use crate::error::AgentLoopError;

/// Lower-cased fragments of provider messages that mean "the request did not
/// fit the model's context". Deliberately short and specific: this list only
/// *flags* errors the real classifiers missed (`overflow_suspected_unclassified`)
/// so a false positive costs one log line, but a broad phrase such as
/// "exceeds the maximum" would bury the signal under tool-count and file-size
/// errors.
const OVERFLOW_WORDING: &[&str] = &[
    "context length",
    "context_length",
    "context window",
    "maximum context",
    "prompt is too long",
    "prompt too long",
    "input is too long",
    "too many tokens",
    "token limit",
    "input tokens exceed",
    "request too large",
    "reduce the length",
];

/// Whether an error message reads like a context overflow.
pub fn looks_like_context_overflow(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    OVERFLOW_WORDING.iter().any(|needle| lower.contains(needle))
}

/// The provider's own error code, when the message embeds its JSON error body
/// (`{"error":{"code":"context_length_exceeded"}}`, Anthropic's
/// `{"error":{"type":"invalid_request_error"}}`, Gemini's
/// `{"error":{"status":"INVALID_ARGUMENT"}}`). Only string codes are reported:
/// a bare HTTP status number repeats the `http_status` field.
pub fn provider_error_code(text: &str) -> Option<String> {
    const POINTERS: &[&str] = &[
        "/error/code",
        "/error/status",
        "/error/type",
        "/code",
        "/type",
    ];
    // THREAT[TM-DOS-038]: the text carries a provider-controlled body. Same
    // bounds as `llm_error::provider_error_code_in`: no parse of an oversized
    // body, and a retained code is length-capped.
    if text.len() > 64 * 1024 {
        return None;
    }
    text.match_indices('{').take(4).find_map(|(start, _)| {
        let value: serde_json::Value = serde_json::Deserializer::from_str(&text[start..])
            .into_iter()
            .next()?
            .ok()?;
        POINTERS.iter().find_map(|pointer| {
            value
                .pointer(pointer)
                .and_then(serde_json::Value::as_str)
                .filter(|code| !code.is_empty() && code.len() <= 128 && *code != "error")
                .map(str::to_owned)
        })
    })
}

/// The HTTP status a driver wrote into its error message as
/// `API error (400 Bad Request): ...`. Request-too-large errors carry no
/// structured status, only this text.
pub fn http_status_in(text: &str) -> Option<u16> {
    text.match_indices('(').find_map(|(start, _)| {
        let rest = text.get(start + 1..)?;
        let digits = rest.get(..3)?;
        let after = rest[3..].chars().next();
        (digits.bytes().all(|b| b.is_ascii_digit()) && matches!(after, Some(' ' | ')')))
            .then(|| digits.parse().ok())
            .flatten()
            .filter(|status| (100..600).contains(status))
    })
}

/// How a terminal provider error relates to context overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverflowObservation {
    /// The classifier recognized it as request-too-large.
    Classified,
    /// Not classified, but its wording reads like an overflow.
    SuspectedUnclassified,
    /// Unrelated to overflow.
    None,
}

impl OverflowObservation {
    pub fn of(error: &AgentLoopError) -> Self {
        if error.is_request_too_large() {
            Self::Classified
        } else if looks_like_context_overflow(&error.to_string()) {
            Self::SuspectedUnclassified
        } else {
            Self::None
        }
    }
}

/// Log a terminal provider error that is, or reads like, a context overflow.
///
/// `provider` is the driver or provider label the caller has (drivers pass
/// their name; the engine passes the provider type), `model` when known.
/// Returns what was observed so callers and tests can act on it.
pub fn observe_request_error(
    provider: &str,
    model: Option<&str>,
    http_status: Option<u16>,
    error: &AgentLoopError,
) -> OverflowObservation {
    let observation = OverflowObservation::of(error);
    if observation == OverflowObservation::None {
        return observation;
    }
    // Prefer the structured fields a driver already parsed over re-reading
    // the message text.
    let structured = match error {
        AgentLoopError::Llm(llm) => Some(llm),
        _ => None,
    };
    let message = error.to_string();
    let http_status = http_status
        .or(structured.and_then(|llm| llm.status))
        .or_else(|| http_status_in(&message));
    let code = structured
        .and_then(|llm| llm.code.clone())
        .or_else(|| provider_error_code(&message));
    match observation {
        OverflowObservation::Classified => tracing::warn!(
            target: "everruns::llm_telemetry",
            provider,
            model,
            http_status,
            provider_error_code = code.as_deref(),
            classified = true,
            "LLM request rejected as too large (context overflow)"
        ),
        OverflowObservation::SuspectedUnclassified => tracing::warn!(
            target: "everruns::llm_telemetry",
            provider,
            model,
            http_status,
            provider_error_code = code.as_deref(),
            classified = false,
            overflow_suspected_unclassified = true,
            "LLM error reads like a context overflow but was not classified as one"
        ),
        OverflowObservation::None => {}
    }
    observation
}

/// Log tool calls a driver discarded because the response was cut off or
/// rejected. No-op for a zero count.
pub fn warn_tool_calls_dropped(provider: &str, model: &str, count: u32, stop_reason: &str) {
    if count == 0 {
        return;
    }
    tracing::warn!(
        target: "everruns::llm_telemetry",
        provider,
        model,
        tool_calls_dropped = count,
        stop_reason,
        "LLM response ended without completing its tool calls; calls discarded"
    );
}

/// Log tool calls handed on for execution from a truncated response. Their own
/// arguments are complete (a cut-off call is dropped instead). No-op for a zero
/// count.
pub fn warn_tool_calls_truncated_executed(
    provider: &str,
    model: &str,
    count: u32,
    stop_reason: &str,
) {
    if count == 0 {
        return;
    }
    tracing::warn!(
        target: "everruns::llm_telemetry",
        provider,
        model,
        tool_calls_truncated_executed = count,
        stop_reason,
        "LLM tool calls from a truncated response will run; the response was cut off after them"
    );
}

/// Log that the engine's output-truncation gate acted on a generation that lost
/// tool calls: `action` is `retried` (another generation was scheduled and the
/// model told why) or `failed` (the turn ends with an error). `consecutive` is
/// how many generations in a row, this one included, lost calls.
pub fn warn_truncation_gate(
    provider: &str,
    model: &str,
    action: &str,
    policy: &str,
    tool_calls_lost: u32,
    consecutive: u32,
    finish_reason: &str,
) {
    tracing::warn!(
        target: "everruns::llm_telemetry",
        provider,
        model,
        truncation_gate = action,
        policy,
        tool_calls_lost,
        consecutive,
        finish_reason,
        "LLM response lost tool calls; output-truncation gate acted"
    );
}

/// Log that a retry loop gave up: attempt budget or time budget spent.
pub fn warn_retry_exhausted(provider: &str, attempts: u32, max_retries: u32, budget: &str) {
    tracing::warn!(
        target: "everruns::llm_telemetry",
        provider,
        attempts,
        max_retries,
        retry_budget = budget,
        retry_exhausted = true,
        "LLM retry budget exhausted"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_wording_flags_known_phrases_and_ignores_unrelated_limits() {
        for text in [
            "prompt is too long: 210000 tokens > 200000 maximum",
            "This model's maximum context length is 128000 tokens",
            "Input tokens exceed the configured limit of 272000 tokens",
            "The input token count exceeds the context window",
        ] {
            assert!(looks_like_context_overflow(text), "{text}");
        }
        for text in [
            "rate limit exceeded",
            "Number of tools exceeds the maximum of 128",
            "invalid api key",
        ] {
            assert!(!looks_like_context_overflow(text), "{text}");
        }
    }

    #[test]
    fn provider_error_code_reads_embedded_json_bodies() {
        assert_eq!(
            provider_error_code(
                r#"OpenAI API error (400): {"error":{"code":"context_length_exceeded","message":"x"}}"#
            )
            .as_deref(),
            Some("context_length_exceeded")
        );
        assert_eq!(
            provider_error_code(
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long"}}"#
            )
            .as_deref(),
            Some("invalid_request_error")
        );
        assert_eq!(
            provider_error_code(r#"{"error":{"code":400,"status":"INVALID_ARGUMENT"}}"#).as_deref(),
            Some("INVALID_ARGUMENT")
        );
        assert_eq!(provider_error_code("plain text, no body"), None);
        assert_eq!(provider_error_code("{not json"), None);
    }

    #[test]
    fn http_status_is_read_from_driver_error_text() {
        assert_eq!(
            http_status_in("Anthropic API error (400 Bad Request): {}"),
            Some(400)
        );
        assert_eq!(http_status_in("Gemini API error (413): too big"), Some(413));
        assert_eq!(http_status_in("limit (12345 tokens) exceeded"), None);
        assert_eq!(http_status_in("(999) nope, (é)"), None);
        assert_eq!(http_status_in("no status here"), None);
    }

    #[test]
    fn observation_separates_classified_suspected_and_unrelated_errors() {
        assert_eq!(
            observe_request_error(
                "anthropic",
                Some("m"),
                Some(400),
                &AgentLoopError::request_too_large("prompt is too long")
            ),
            OverflowObservation::Classified
        );
        assert_eq!(
            observe_request_error(
                "openai",
                None,
                Some(400),
                &AgentLoopError::llm("upstream: input tokens exceed the context window")
            ),
            OverflowObservation::SuspectedUnclassified
        );
        assert_eq!(
            observe_request_error("openai", None, Some(500), &AgentLoopError::llm("boom")),
            OverflowObservation::None
        );
    }
}
