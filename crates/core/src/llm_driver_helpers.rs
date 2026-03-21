// Shared LLM Driver Helpers
//
// Common utilities extracted from individual LLM driver implementations
// (Anthropic, Gemini, OpenAI) to eliminate duplication.
//
// See specs/llm-drivers.md for driver requirements.

use reqwest::StatusCode;

/// Placeholder text for audio content in providers that don't support audio input.
pub const AUDIO_CONTENT_PLACEHOLDER: &str = "[Audio content not supported]";

// ============================================================================
// Data URL Parsing
// ============================================================================

/// Parsed data URL components (e.g., `data:image/jpeg;base64,/9j/4AAQ...`).
#[derive(Debug, Clone)]
pub struct ParsedDataUrl {
    /// MIME type (e.g., "image/jpeg", "image/png")
    pub media_type: String,
    /// Base64-encoded data (without the `data:...;base64,` prefix)
    pub data: String,
}

/// Parse a data URL into its media type and data components.
///
/// Handles formats like `data:<media_type>;base64,<data>` and `data:<media_type>,<data>`.
/// The `;base64` suffix is stripped from the media type if present, but its presence
/// is not enforced — callers should assume data may be base64-encoded.
///
/// Returns `None` if the URL doesn't start with `data:` or has no comma separator.
/// Unlike the previous per-driver implementations, this does NOT silently
/// fall back to `image/jpeg` on parse failure — callers handle fallback.
pub fn parse_data_url(url: &str) -> Option<ParsedDataUrl> {
    if !url.starts_with("data:") {
        return None;
    }

    let parts: Vec<&str> = url.splitn(2, ',').collect();
    if parts.len() != 2 {
        return None;
    }

    let media_type = parts[0]
        .trim_start_matches("data:")
        .trim_end_matches(";base64")
        .to_string();
    let data = parts[1].to_string();

    Some(ParsedDataUrl { media_type, data })
}

// ============================================================================
// Error Detection Helpers
// ============================================================================

/// Check if an HTTP error indicates the request payload is too large.
///
/// Detects common patterns across LLM providers:
/// - HTTP 413 Payload Too Large
/// - HTTP 4xx with context length / token limit errors
/// - Generic "too long" / "exceeds maximum" patterns (with token/context qualifiers)
///
/// Provider-specific patterns (must be lowercase) can be checked via `extra_patterns`.
pub fn is_request_too_large(status: StatusCode, error_text: &str, extra_patterns: &[&str]) -> bool {
    let error_lower = error_text.to_lowercase();

    // HTTP 413 Payload Too Large (universal)
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return true;
    }

    // Only check text patterns for client errors
    if status.is_client_error() {
        // Generic patterns that apply across providers
        if error_lower.contains("input is too long") || error_lower.contains("maximum context") {
            return true;
        }

        // Require a token/context qualifier with "exceeds the maximum" to avoid false positives
        if error_lower.contains("exceeds the maximum")
            && (error_lower.contains("token") || error_lower.contains("context"))
        {
            return true;
        }

        // Provider-specific patterns (already lowercase, no allocation needed)
        for pattern in extra_patterns {
            if error_lower.contains(pattern) {
                return true;
            }
        }
    }

    false
}

/// Anthropic-specific "request too large" error patterns (passed to `is_request_too_large`).
pub const ANTHROPIC_TOO_LARGE_PATTERNS: &[&str] = &[
    "prompt is too long",
    "request size exceeded",
    "context length",
    "too many tokens",
];

/// Gemini-specific "request too large" error patterns (passed to `is_request_too_large`).
pub const GEMINI_TOO_LARGE_PATTERNS: &[&str] = &[
    "request payload size exceeds",
    "content too large",
    "token limit exceeded",
];

/// Check if an HTTP error indicates the model was not found.
///
/// Only matches on 404 status. Uses provider-specific patterns (must be lowercase)
/// to avoid false positives on generic 404s (e.g., "Endpoint not found").
pub fn is_model_not_found(status: StatusCode, error_text: &str, patterns: &[&str]) -> bool {
    if status != StatusCode::NOT_FOUND {
        return false;
    }

    let error_lower = error_text.to_lowercase();

    // Provider-specific patterns (already lowercase, no allocation needed)
    for pattern in patterns {
        if error_lower.contains(pattern) {
            return true;
        }
    }

    false
}

/// Anthropic-specific model-not-found patterns.
/// Matches `not_found_error` (Anthropic's error type) or `model` + `not found` together.
pub const ANTHROPIC_NOT_FOUND_PATTERNS: &[&str] = &["not_found_error"];

/// Gemini-specific model-not-found patterns.
/// Gemini returns 404 with `"NOT_FOUND"` status or `"model"` in the message.
pub const GEMINI_NOT_FOUND_PATTERNS: &[&str] = &["not_found", "model"];

// ============================================================================
// Thinking Budget Constants
// ============================================================================

/// Thinking token budgets for Anthropic's extended thinking feature.
/// Maps reasoning effort levels to token budgets.
pub mod thinking_budget {
    pub const LOW: u32 = 1024;
    pub const MEDIUM: u32 = 4096;
    pub const HIGH: u32 = 16384;
    pub const XHIGH: u32 = 32768;

    /// Map a reasoning effort string to a thinking budget.
    pub fn from_effort(effort: &str) -> Option<u32> {
        match effort.to_lowercase().as_str() {
            "low" => Some(LOW),
            "medium" => Some(MEDIUM),
            "high" => Some(HIGH),
            "xhigh" => Some(XHIGH),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_data_url_valid() {
        let result = parse_data_url("data:image/png;base64,iVBOR").unwrap();
        assert_eq!(result.media_type, "image/png");
        assert_eq!(result.data, "iVBOR");
    }

    #[test]
    fn test_parse_data_url_jpeg() {
        let result = parse_data_url("data:image/jpeg;base64,/9j/4AAQ").unwrap();
        assert_eq!(result.media_type, "image/jpeg");
        assert_eq!(result.data, "/9j/4AAQ");
    }

    #[test]
    fn test_parse_data_url_not_data() {
        assert!(parse_data_url("https://example.com/image.png").is_none());
    }

    #[test]
    fn test_parse_data_url_no_comma() {
        assert!(parse_data_url("data:image/jpeg;base64").is_none());
    }

    #[test]
    fn test_is_request_too_large_413() {
        assert!(is_request_too_large(StatusCode::PAYLOAD_TOO_LARGE, "", &[]));
    }

    #[test]
    fn test_is_request_too_large_generic() {
        assert!(is_request_too_large(
            StatusCode::BAD_REQUEST,
            "input is too long",
            &[]
        ));
    }

    #[test]
    fn test_is_request_too_large_anthropic() {
        assert!(is_request_too_large(
            StatusCode::BAD_REQUEST,
            "prompt is too long: 100000 tokens",
            ANTHROPIC_TOO_LARGE_PATTERNS
        ));
    }

    #[test]
    fn test_is_request_too_large_gemini() {
        assert!(is_request_too_large(
            StatusCode::BAD_REQUEST,
            "request payload size exceeds limit",
            GEMINI_TOO_LARGE_PATTERNS
        ));
    }

    #[test]
    fn test_is_model_not_found_with_pattern() {
        assert!(is_model_not_found(
            StatusCode::NOT_FOUND,
            r#"{"error":{"type":"not_found_error"}}"#,
            ANTHROPIC_NOT_FOUND_PATTERNS
        ));
    }

    #[test]
    fn test_is_model_not_found_no_match_without_pattern() {
        // Generic "not found" without matching patterns should NOT match
        assert!(!is_model_not_found(
            StatusCode::NOT_FOUND,
            "Endpoint not found",
            ANTHROPIC_NOT_FOUND_PATTERNS
        ));
    }

    #[test]
    fn test_is_model_not_found_not_404() {
        assert!(!is_model_not_found(
            StatusCode::BAD_REQUEST,
            "model not found",
            &[]
        ));
    }

    #[test]
    fn test_is_model_not_found_gemini() {
        assert!(is_model_not_found(
            StatusCode::NOT_FOUND,
            r#"{"error":{"status":"NOT_FOUND","message":"model foo"}}"#,
            GEMINI_NOT_FOUND_PATTERNS
        ));
    }

    #[test]
    fn test_thinking_budget_from_effort() {
        assert_eq!(thinking_budget::from_effort("low"), Some(1024));
        assert_eq!(thinking_budget::from_effort("medium"), Some(4096));
        assert_eq!(thinking_budget::from_effort("HIGH"), Some(16384));
        assert_eq!(thinking_budget::from_effort("xhigh"), Some(32768));
        assert_eq!(thinking_budget::from_effort("unknown"), None);
    }
}
