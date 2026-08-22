// Public-endpoint error sanitization.
//
// "Public" endpoints serve unauthenticated callers (see
// `knowledge/execution/public-endpoints.md`). They MUST NOT surface internal information to
// clients: provider names, model IDs, HTTP status codes from upstream calls,
// quota state, stack traces, or any other implementation detail. They also
// MUST NOT instruct the user to "contact admin" or "contact support" — public
// users have no relationship to either.
//
// This module provides the canonical translation from internal error codes
// (`everruns_provider::user_facing_error::codes`) to a small, stable set of
// public-facing codes and generic messages. AG-UI is currently the only
// public endpoint; future public endpoints MUST route their errors through
// `PublicError` so the contract stays uniform.

use everruns_provider::user_facing_error::codes as user_facing_error_codes;

/// Stable public error codes. Clients of public endpoints can rely on these
/// values; internal codes from `user_facing_error_codes` are NOT part of the
/// public contract and may change at any time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicErrorCode {
    /// The service is busy or upstream-rate-limited; the caller should retry.
    RateLimited,
    /// The service cannot fulfill the request right now (provider outage,
    /// misconfiguration, budget exhaustion, model unavailable, etc.). The
    /// caller cannot fix this themselves.
    ServiceUnavailable,
    /// The caller's input is too large for the configured agent/model.
    RequestTooLarge,
    /// Universal fallback for anything else — including unknown internal
    /// codes, missing classification, and unexpected runtime errors. This is
    /// the default any public endpoint MUST fall back to when in doubt.
    InternalError,
}

impl PublicErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RateLimited => "rate_limited",
            Self::ServiceUnavailable => "service_unavailable",
            Self::RequestTooLarge => "request_too_large",
            Self::InternalError => "internal_error",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::RateLimited => "The service is busy right now. Please try again in a moment.",
            Self::ServiceUnavailable => {
                "The service is temporarily unavailable. Please try again shortly."
            }
            // Covers both an oversized last message and accumulated context that no
            // longer fits — the public message therefore points at the conversation,
            // not just the latest message.
            Self::RequestTooLarge => {
                "The conversation has become too long to process. Please start a new conversation."
            }
            Self::InternalError => "Something went wrong. Please try again.",
        }
    }
}

impl Default for PublicErrorCode {
    /// Universal fallback. Any code path that cannot classify an error MUST
    /// use this default rather than emitting an internal string.
    fn default() -> Self {
        Self::InternalError
    }
}

/// Public-facing error payload. Both fields are safe to expose to
/// unauthenticated callers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicError {
    pub code: PublicErrorCode,
    pub message: &'static str,
}

impl PublicError {
    pub fn new(code: PublicErrorCode) -> Self {
        Self {
            code,
            message: code.message(),
        }
    }

    /// The universal fallback. Use this whenever there is no internal code to
    /// classify, or when classification is uncertain.
    pub fn fallback() -> Self {
        Self::new(PublicErrorCode::default())
    }

    /// Map an internal `user_facing_error_codes` code to its public
    /// equivalent. Unknown / unclassifiable input falls back to
    /// `InternalError` — this is the contract for public endpoints.
    pub fn from_internal_code(internal_code: Option<&str>) -> Self {
        let code = match internal_code {
            Some(user_facing_error_codes::PROVIDER_RATE_LIMITED) => PublicErrorCode::RateLimited,
            Some(user_facing_error_codes::REQUEST_TOO_LARGE) => PublicErrorCode::RequestTooLarge,
            Some(
                user_facing_error_codes::PROVIDER_UNAVAILABLE
                | user_facing_error_codes::DEPENDENCY_UNAVAILABLE
                | user_facing_error_codes::PROVIDER_MISCONFIGURED
                | user_facing_error_codes::PROVIDER_QUOTA_EXHAUSTED
                | user_facing_error_codes::MODEL_UNAVAILABLE
                | user_facing_error_codes::MODEL_NOT_CONFIGURED
                | user_facing_error_codes::BUDGET_EXHAUSTED
                | user_facing_error_codes::BUDGET_PAUSED
                | user_facing_error_codes::SOFT_LIMIT_REACHED,
            ) => PublicErrorCode::ServiceUnavailable,
            _ => PublicErrorCode::default(),
        };
        Self::new(code)
    }
}

impl Default for PublicError {
    fn default() -> Self {
        Self::fallback()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_is_internal_error() {
        let err = PublicError::fallback();
        assert_eq!(err.code, PublicErrorCode::InternalError);
        assert_eq!(err.code.as_str(), "internal_error");
        assert_eq!(err.message, "Something went wrong. Please try again.");
    }

    #[test]
    fn default_matches_fallback() {
        assert_eq!(PublicError::default(), PublicError::fallback());
        assert_eq!(PublicErrorCode::default(), PublicErrorCode::InternalError);
    }

    #[test]
    fn unknown_internal_code_falls_back_to_internal_error() {
        for input in [
            None,
            Some(""),
            Some("some_future_code"),
            Some("processing_error"),
        ] {
            assert_eq!(
                PublicError::from_internal_code(input).code,
                PublicErrorCode::InternalError,
                "expected fallback for {input:?}",
            );
        }
    }

    #[test]
    fn rate_limit_classification() {
        assert_eq!(
            PublicError::from_internal_code(Some(user_facing_error_codes::PROVIDER_RATE_LIMITED))
                .code,
            PublicErrorCode::RateLimited,
        );
    }

    #[test]
    fn unavailable_classification_covers_billing_and_misconfig() {
        for code in [
            user_facing_error_codes::PROVIDER_UNAVAILABLE,
            user_facing_error_codes::DEPENDENCY_UNAVAILABLE,
            user_facing_error_codes::PROVIDER_MISCONFIGURED,
            user_facing_error_codes::PROVIDER_QUOTA_EXHAUSTED,
            user_facing_error_codes::MODEL_UNAVAILABLE,
            user_facing_error_codes::MODEL_NOT_CONFIGURED,
            user_facing_error_codes::BUDGET_EXHAUSTED,
            user_facing_error_codes::BUDGET_PAUSED,
            user_facing_error_codes::SOFT_LIMIT_REACHED,
        ] {
            assert_eq!(
                PublicError::from_internal_code(Some(code)).code,
                PublicErrorCode::ServiceUnavailable,
                "expected ServiceUnavailable for {code}",
            );
        }
    }

    #[test]
    fn messages_never_mention_internal_concepts() {
        for code in [
            PublicErrorCode::RateLimited,
            PublicErrorCode::ServiceUnavailable,
            PublicErrorCode::RequestTooLarge,
            PublicErrorCode::InternalError,
        ] {
            let lower = code.message().to_lowercase();
            for forbidden in [
                "admin",
                "support",
                "billing",
                "provider",
                "openai",
                "anthropic",
                "api key",
                "quota",
                "model",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "{code:?} message leaks `{forbidden}`: {}",
                    code.message(),
                );
            }
        }
    }
}
