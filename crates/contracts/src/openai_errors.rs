//! Status/body classifiers for OpenAI-compatible HTTP error responses.
//!
//! Shared by the Chat Completions and Responses drivers to decide which errors
//! are terminal (model missing, request too large) rather than retryable.

/// Check if the error indicates the model was not found.
///
/// OpenAI returns 404 or 400 with `"model_not_found"` code or `"does not exist"` message.
/// OpenAI can also return 403 with `"model_not_found"` for tier-gated models — these must
/// be classified as model_unavailable rather than provider_misconfigured.
/// Also handles Gemini/OpenAI-compatible endpoints with similar patterns.
pub fn is_openai_model_not_found(status: reqwest::StatusCode, error_text: &str) -> bool {
    let error_lower = error_text.to_lowercase();

    // OpenAI can return 404, 400, or 403 (tier-gated access) for nonexistent/inaccessible models
    if status == reqwest::StatusCode::NOT_FOUND
        || status == reqwest::StatusCode::BAD_REQUEST
        || status == reqwest::StatusCode::FORBIDDEN
    {
        // OpenAI: {"error":{"code":"model_not_found","message":"The model 'x' does not exist"}}
        if error_lower.contains("model_not_found") {
            return true;
        }
    }

    // 404 with generic model-not-found patterns
    if status == reqwest::StatusCode::NOT_FOUND {
        if error_lower.contains("does not exist") {
            return true;
        }
        if error_lower.contains("model") && error_lower.contains("not found") {
            return true;
        }
    }

    false
}

/// Check if an OpenAI API error indicates the request is too large.
///
/// Detects:
/// - 429 with "Request too large", or a token limit smaller than the request
/// - 400 with "context_length_exceeded" code
/// - Any message about maximum context length being exceeded
pub fn is_openai_request_too_large(status: reqwest::StatusCode, error_text: &str) -> bool {
    let error_lower = error_text.to_lowercase();

    // HTTP 429 with token-related errors
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        // "Request too large for gpt-4" pattern
        if error_lower.contains("request too large") {
            return true;
        }
        // "tokens per min (TPM): Limit X, Requested Y" is only a size rejection
        // when the single request exceeds the whole budget (Y > X). Quota 429s
        // such as OpenAI's "Limit X, Used U, Requested Y. Please try again" or
        // Azure Foundry's "Rate limit of 50000 per 60s exceeded for
        // ...InputTokens" are transient and must reach the retry path (#3740).
        if let (Some(limit), Some(requested)) = (
            number_after(&error_lower, "limit "),
            number_after(&error_lower, "requested "),
        ) {
            return requested > limit;
        }
    }

    // HTTP 400 with context length errors
    if status == reqwest::StatusCode::BAD_REQUEST {
        // "context_length_exceeded" error code
        if error_lower.contains("context_length_exceeded") {
            return true;
        }
        // "maximum context length" message
        if error_lower.contains("maximum context length") {
            return true;
        }
    }

    // Generic patterns that could appear with various status codes
    if error_lower.contains("tokens must be reduced")
        || error_lower.contains("reduce the length")
        || error_lower.contains("input is too long")
    {
        return true;
    }

    false
}

/// Parses the integer that immediately follows the first `marker` in `text`,
/// ignoring thousands separators.
fn number_after(text: &str, marker: &str) -> Option<u64> {
    let rest = &text[text.find(marker)? + marker.len()..];
    let digits: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == ',')
        .filter(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_size_classification_distinguishes_status_gates_and_generic_limits() {
        for (status, body, expected) in [
            (
                429,
                r#"{"error":{"message":"Request too large for gpt-4o in organization org-xxx on tokens per min (TPM): Limit 500000, Requested 538772."}}"#,
                true,
            ),
            (
                429,
                r#"{"error":{"message":"tokens per min (TPM): Limit 500000, Requested 600000"}}"#,
                true,
            ),
            (
                400,
                r#"{"error":{"code":"context_length_exceeded","message":"This model's maximum context length is 128000 tokens."}}"#,
                true,
            ),
            (
                400,
                r#"{"error":{"message":"This model's maximum context length is 128000 tokens"}}"#,
                true,
            ),
            (
                400,
                r#"{"error":{"message":"The input or output tokens must be reduced"}}"#,
                true,
            ),
            (
                429,
                r#"{"error":{"message":"Rate limit exceeded: too many requests per minute"}}"#,
                false,
            ),
            // Transient OpenAI TPM quota: this request fits, the minute is spent.
            (
                429,
                r#"{"error":{"message":"Rate limit reached for gpt-4o in organization org-xxx on tokens per min (TPM): Limit 30000, Used 25000, Requested 6000. Please try again in 2s."}}"#,
                false,
            ),
            // Azure AI Foundry per-minute quota (#3740).
            (
                429,
                r#"{"event_id":null,"error":{"type":"invalid_request_error","message":"{\"error\":{\"code\":\"RateLimitReached\",\"message\":\"Rate limit of 50000 per 60s exceeded for UserByModelByMinuteUncachedInputTokens. Please wait 46 seconds before retrying.\"}}"}}"#,
                false,
            ),
            (
                500,
                r#"{"error":{"message":"Internal server error"}}"#,
                false,
            ),
            (400, r#"{"error":{"message":"Invalid request"}}"#, false),
            (400, "request too large", false),
            (429, "context_length_exceeded", false),
            (500, "tokens limit", false),
            (400, "REDUCE THE LENGTH", true),
            (413, "input is too long", true),
        ] {
            assert_eq!(
                is_openai_request_too_large(reqwest::StatusCode::from_u16(status).unwrap(), body),
                expected,
                "{status}: {body}"
            );
        }
    }

    #[test]
    fn model_unavailable_classification_separates_auth_endpoint_and_model_errors() {
        for (status, body, expected) in [
            (
                404,
                r#"{"error":{"code":"model_not_found","message":"The model 'gpt-99' does not exist or you do not have access to it.","type":"invalid_request_error","param":null}}"#,
                true,
            ),
            (
                404,
                r#"{"error":{"message":"The model 'fake-model' does not exist"}}"#,
                true,
            ),
            (404, r#"{"error":{"message":"Model not found"}}"#, true),
            (
                400,
                r#"{"error":{"code":"model_not_found","message":"The requested model 'gpt-99' does not exist.","type":"invalid_request_error","param":"model"}}"#,
                true,
            ),
            (
                400,
                r#"{"error":{"code":"invalid_request","message":"Some other error"}}"#,
                false,
            ),
            (404, r#"{"error":{"message":"Endpoint not found"}}"#, false),
            (
                403,
                r#"{"error":{"code":"model_not_found","message":"The model 'gpt-5.4-mini' does not exist or you do not have access to it.","type":"invalid_request_error","param":null}}"#,
                true,
            ),
            (
                403,
                r#"{"error":{"message":"Invalid authentication credentials","type":"authentication_error"}}"#,
                false,
            ),
            (401, "model_not_found", false),
            (500, "model_not_found", false),
            (400, "model does not exist", false),
            (403, "model not found", false),
            (404, "MODEL NOT FOUND", true),
        ] {
            assert_eq!(
                is_openai_model_not_found(reqwest::StatusCode::from_u16(status).unwrap(), body),
                expected,
                "{status}: {body}"
            );
        }
    }
}
