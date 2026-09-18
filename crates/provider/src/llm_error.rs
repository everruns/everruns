//! How a provider failure is classified, and what is preserved of it.
//!
//! [`LlmErrorKind`] is Everruns' taxonomy — what runtime policy retries, what
//! it surfaces, what it refuses. [`LlmError`] carries it alongside the
//! provider's own answer (status, error code, requested retry delay), recorded
//! at the boundary where the HTTP response was still structured. Without those,
//! anything downstream that has to re-express the failure is left scraping the
//! display string, which is not a contract.
//!
//! Split out of [`error`](crate::error) when that file outgrew what anyone can
//! hold in their head; everything here is re-exported from there, so existing
//! paths keep working.

use serde::{Deserialize, Serialize};

use crate::user_facing_error::{
    is_attestation_required_message, is_provider_quota_message, is_usage_limit_message,
};

/// Machine-readable reason for provider billing pressure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingPressureReason {
    /// Existing requests temporarily consume the account's available budget.
    InFlightBudgetExhausted,
    /// The provider account does not have enough credits for the request.
    InsufficientCredits,
}

/// Semantic classification of an LLM provider error, assigned by the driver
/// at the provider boundary where the HTTP status and response body are still
/// available. Downstream consumers prefer this over re-parsing error strings;
/// `LlmErrorKind::Other` falls back to string classification
/// (`classify_runtime_error_message`) so untyped errors keep working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LlmErrorKind {
    /// Invalid or missing credentials, or access denied (401/403, bad API key).
    Authentication,
    /// Provider account is out of credits/quota (billing). Non-transient:
    /// needs operator action, unlike a regular rate limit.
    QuotaExhausted,
    /// Provider billing pressure with the stable machine-readable reason and
    /// the provider's suggested retry delay. This is non-transient at the
    /// runtime layer: consumers decide whether to wait or add credits.
    BillingPressure {
        /// Stable provider reason for the billing refusal.
        reason: BillingPressureReason,
        /// Provider-requested delay before another attempt, in seconds.
        retry_after_secs: Option<u64>,
    },
    /// Transient rate limit (429).
    RateLimited,
    /// Provider outage or unreachable (5xx, 529, network failure).
    Unavailable,
    /// Provider account has not completed a confirmation the model requires
    /// (OpenRouter's 18+ age gate). Non-transient and not a credential
    /// problem: it clears when the account holder completes the confirmation,
    /// so it is kept apart from `Authentication` even though it arrives as a
    /// 403.
    AttestationRequired,
    /// Provider rejected the request shape (4xx that is not auth/quota/429).
    InvalidRequest,
    /// The provider answered, but the answer could not be used as a turn: the
    /// stream ended before its terminal event, or it exceeded a limit the
    /// caller set.
    ///
    /// Distinct from [`Unavailable`](Self::Unavailable) because retrying does
    /// not help by itself, and from [`Other`](Self::Other) because the
    /// classification is certain — the failure was decided on Everruns' side
    /// of the wire, not guessed from provider prose.
    MalformedResponse,
    /// Unclassified; downstream falls back to string classification.
    Other,
}

impl LlmErrorKind {
    /// Classify a provider's stable machine-readable error code.
    pub fn from_provider_code(code: &str) -> Option<Self> {
        let code = code.trim().to_ascii_lowercase();
        match code.as_str() {
            "insufficient_quota"
            | "billing_hard_limit_reached"
            | "credit_balance_too_low"
            | "credit_balance_exhausted" => Some(Self::QuotaExhausted),
            "authentication_error" | "invalid_api_key" | "permission_denied" => {
                Some(Self::Authentication)
            }
            "rate_limit_exceeded" | "rate_limit_error" | "overloaded_error" => {
                Some(Self::RateLimited)
            }
            "server_error"
            | "internal_error"
            | "processing_error"
            | "service_unavailable"
            | "timeout" => Some(Self::Unavailable),
            "invalid_request_error" | "model_not_found" => Some(Self::InvalidRequest),
            _ => None,
        }
    }

    /// Classify a provider HTTP error from status code + response body.
    ///
    /// Quota/billing patterns are checked before the status code because
    /// providers surface exhausted billing under different statuses
    /// (OpenAI: 429 `insufficient_quota`, Anthropic: 400 "credit balance is
    /// too low").
    pub fn from_provider_status(status: u16, body: &str) -> Self {
        if is_provider_quota_message(body) || is_usage_limit_message(body) {
            return LlmErrorKind::QuotaExhausted;
        }
        // Body-driven for the same reason as quota: the 403 this arrives under
        // is indistinguishable from a bad-key 403 by status alone, and the
        // gate is worth naming only when the body actually reports one.
        if is_attestation_required_message(body) {
            return LlmErrorKind::AttestationRequired;
        }
        match status {
            401 | 403 => LlmErrorKind::Authentication,
            429 => LlmErrorKind::RateLimited,
            408 | 409 => LlmErrorKind::Unavailable,
            501 => LlmErrorKind::Other,
            500..=599 => LlmErrorKind::Unavailable,
            400..=499 => LlmErrorKind::InvalidRequest,
            _ => LlmErrorKind::Other,
        }
    }

    /// Keyword-based classification for drivers without an HTTP status at the
    /// error site (e.g. Bedrock SDK errors).
    pub fn from_error_text(text: &str) -> Self {
        if is_provider_quota_message(text) || is_usage_limit_message(text) {
            return LlmErrorKind::QuotaExhausted;
        }
        let lower = text.to_ascii_lowercase();
        if lower.contains("throttlingexception")
            || lower.contains("toomanyrequestsexception")
            || lower.contains("rate limit")
            || lower.contains("too many requests")
        {
            return LlmErrorKind::RateLimited;
        }
        if lower.contains("accessdeniedexception")
            || lower.contains("unrecognizedclientexception")
            || lower.contains("expiredtokenexception")
            || lower.contains("invalidsignatureexception")
            || lower.contains("unauthorized")
        {
            return LlmErrorKind::Authentication;
        }
        if lower.contains("serviceunavailable")
            || lower.contains("service unavailable")
            || lower.contains("internalserverexception")
            || lower.contains("modelnotreadyexception")
        {
            return LlmErrorKind::Unavailable;
        }
        LlmErrorKind::Other
    }
}

/// LLM provider error with a semantic kind attached by the driver.
///
/// [`kind`](Self::kind) is what runtime policy keys on. The transport fields
/// beside it — [`status`](Self::status), [`code`](Self::code),
/// [`retry_after_secs`](Self::retry_after_secs) — are the provider's own
/// answer, recorded at the boundary where it was still structured. Without
/// them an embedder that has to re-express a failure (an HTTP API in front of
/// Everruns, a retry budget of its own) can only scrape them back out of
/// [`message`](Self::message), which is display text and not a contract.
///
/// `#[non_exhaustive]`: the boundary keeps learning to preserve more, and a
/// field addition must not break construction downstream. Build with
/// [`LlmError::new`] and the `with_*` setters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LlmError {
    pub kind: LlmErrorKind,
    pub message: String,
    /// HTTP status the provider answered with, when the failure arrived as an
    /// HTTP response. `None` for SDK, transport, and protocol failures that
    /// never carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    /// The provider's own machine-readable error code, verbatim (OpenAI
    /// `error.code`, Anthropic `error.type`). Kept beside `kind` rather than
    /// folded into it: `kind` is Everruns' taxonomy, this is the provider's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Delay the provider asked for before another attempt, in seconds
    /// (`Retry-After` or an equivalent rate-limit header).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_secs: Option<u64>,
    /// Retries already consumed below the turn loop.
    #[serde(default)]
    pub retry_attempts: u32,
    /// Backoff time already consumed below the turn loop.
    #[serde(default)]
    pub retry_wait_ms: u64,
    /// Whether a lower provider layer already made the terminal retry decision.
    #[serde(default)]
    pub retry_handled: bool,
}

impl LlmError {
    /// A provider failure with its semantic kind and nothing else known.
    pub fn new(kind: LlmErrorKind, message: impl Into<String>) -> Self {
        LlmError {
            kind,
            message: message.into(),
            status: None,
            code: None,
            retry_after_secs: None,
            retry_attempts: 0,
            retry_wait_ms: 0,
            retry_handled: false,
        }
    }

    /// Record the HTTP status the provider answered with.
    #[must_use]
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = Some(status);
        self
    }

    /// Record the provider's machine-readable error code.
    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    /// Record the delay the provider asked for before another attempt.
    #[must_use]
    pub fn with_retry_after_secs(mut self, secs: u64) -> Self {
        self.retry_after_secs = Some(secs);
        self
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Read the provider's machine-readable error code out of a JSON error body.
///
/// Both shapes in circulation are accepted: OpenAI-style `error.code` and
/// Anthropic-style `error.type`. A body that is not JSON, or carries neither,
/// yields `None` — this never guesses a code out of prose.
pub(crate) fn provider_error_code_in(body: &str) -> Option<String> {
    let body = body.trim();
    if !body.starts_with('{') {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    let error = parsed.get("error")?;
    let code = error
        .get("code")
        .and_then(|value| value.as_str())
        .or_else(|| error.get("type").and_then(|value| value.as_str()))?;
    let code = code.trim();
    (!code.is_empty()).then(|| code.to_owned())
}
