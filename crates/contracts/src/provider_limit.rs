//! Structured fields for provider limit refusals (rate limits, billing).
//!
//! The classifier in [`error`](crate::error) picks the user-facing code; this
//! module adds what the reader and runtime policy need on top of it: why the
//! provider refused (`reason`), when it clears (`retry_after`), and, for a
//! credit shortfall, how many output tokens the balance still covers.

use std::sync::OnceLock;

use regex::Regex;

use crate::llm_error::{BillingPressureReason, LlmError, LlmErrorKind};
use crate::user_facing_error::{UserFacingError, codes};

/// `reason` field value for OpenRouter's temporary in-flight spending cap.
pub const IN_FLIGHT_BUDGET_EXHAUSTED_REASON: &str = "in_flight_budget_exhausted";

/// Attach limit details to an already-classified provider error.
pub(crate) fn apply_provider_limit_fields(
    error: UserFacingError,
    err: &LlmError,
    context_retry_after: Option<u64>,
) -> UserFacingError {
    match error.code.as_str() {
        codes::PROVIDER_RATE_LIMITED => match err.kind {
            LlmErrorKind::BillingPressure {
                reason: BillingPressureReason::InFlightBudgetExhausted,
                retry_after_secs,
            } => error
                .with_field("reason", IN_FLIGHT_BUDGET_EXHAUSTED_REASON)
                .with_optional_field("retry_after", retry_after_secs),
            _ => error.with_optional_field("retry_after", context_retry_after),
        },
        codes::PROVIDER_QUOTA_EXHAUSTED => apply_affordability_fields(error, &err.message),
        _ => error,
    }
}

/// Add `requested_max_tokens` / `affordable_tokens` when the provider said how
/// far the balance falls short (OpenRouter: "You requested up to X tokens, but
/// can only afford Y").
pub(crate) fn apply_affordability_fields(error: UserFacingError, message: &str) -> UserFacingError {
    match parse_affordability(message) {
        Some((requested, affordable)) => error
            .with_field("requested_max_tokens", requested)
            .with_field("affordable_tokens", affordable),
        None => error,
    }
}

#[expect(
    clippy::expect_used,
    reason = "Built-in constant regexes must compile; invalid syntax is a programming error"
)]
fn parse_affordability(message: &str) -> Option<(u64, u64)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"(?i)requested up to (\d+) tokens, but can only afford (\d+)")
            .expect("valid affordability regex")
    });
    let captures = re.captures(message)?;
    Some((captures[1].parse().ok()?, captures[2].parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AgentLoopError;
    use crate::user_facing_error::UserFacingErrorContext;
    use serde_json::json;

    const AFFORD_BODY: &str = r#"{"error":{"message":"This request requires more credits, or fewer max_tokens. You requested up to 32000 tokens, but can only afford 1234. To increase, visit https://openrouter.ai/settings/credits and upgrade to a paid account","code":402}}"#;

    fn classify(err: LlmError) -> UserFacingError {
        AgentLoopError::Llm(err).user_facing_error(UserFacingErrorContext::default())
    }

    #[test]
    fn in_flight_budget_keeps_rate_limited_code_and_carries_reason_and_retry_after() {
        let error = classify(LlmError::new(
            LlmErrorKind::BillingPressure {
                reason: BillingPressureReason::InFlightBudgetExhausted,
                retry_after_secs: Some(120),
            },
            "OpenRouter API error (402): in-flight budget exhausted",
        ));
        assert_eq!(error.code, codes::PROVIDER_RATE_LIMITED);
        assert_eq!(
            error.fields.get("reason"),
            Some(&json!("in_flight_budget_exhausted"))
        );
        assert_eq!(error.fields.get("retry_after"), Some(&json!(120)));
        assert!(
            error
                .fallback_message()
                .contains("in-flight spending limit"),
            "{}",
            error.fallback_message()
        );
    }

    #[test]
    fn plain_rate_limit_has_no_reason() {
        let error = AgentLoopError::Llm(LlmError::new(LlmErrorKind::RateLimited, "429"))
            .user_facing_error(UserFacingErrorContext::default().with_retry_after(9));
        assert_eq!(error.code, codes::PROVIDER_RATE_LIMITED);
        assert_eq!(error.fields.get("reason"), None);
        assert_eq!(error.fields.get("retry_after"), Some(&json!(9)));
        assert_eq!(
            error.fallback_message(),
            "Rate limited by the AI provider. Please wait a moment."
        );
    }

    #[test]
    fn credit_shortfall_carries_requested_and_affordable_tokens() {
        let error = classify(LlmError::new(
            LlmErrorKind::BillingPressure {
                reason: BillingPressureReason::InsufficientCredits,
                retry_after_secs: None,
            },
            format!("OpenRouter API error (402): {AFFORD_BODY}"),
        ));
        assert_eq!(error.code, codes::PROVIDER_QUOTA_EXHAUSTED);
        assert_eq!(
            error.fields.get("requested_max_tokens"),
            Some(&json!(32000))
        );
        assert_eq!(error.fields.get("affordable_tokens"), Some(&json!(1234)));
        let message = error.fallback_message();
        assert!(message.contains("lower the max output tokens"), "{message}");
        assert!(message.contains("1234"), "{message}");
    }

    #[test]
    fn untyped_affordability_message_is_classified_as_quota() {
        let error = AgentLoopError::Llm(LlmError::new(
            LlmErrorKind::from_provider_status(402, AFFORD_BODY),
            AFFORD_BODY,
        ))
        .user_facing_error(UserFacingErrorContext::default());
        assert_eq!(error.code, codes::PROVIDER_QUOTA_EXHAUSTED);
        assert_eq!(error.fields.get("affordable_tokens"), Some(&json!(1234)));

        let from_text = crate::user_facing_error::classify_runtime_error_message(
            AFFORD_BODY,
            &UserFacingErrorContext::default(),
        );
        assert_eq!(from_text.code, codes::PROVIDER_QUOTA_EXHAUSTED);
        assert_eq!(
            from_text.fields.get("requested_max_tokens"),
            Some(&json!(32000))
        );
    }

    #[test]
    fn any_402_is_quota_exhausted() {
        assert_eq!(
            LlmErrorKind::from_provider_status(402, "Payment Required"),
            LlmErrorKind::QuotaExhausted
        );
        assert_eq!(
            LlmErrorKind::from_provider_status(400, "bad"),
            LlmErrorKind::InvalidRequest
        );
    }

    #[test]
    fn quota_without_affordability_keeps_general_copy() {
        let error = classify(LlmError::new(
            LlmErrorKind::QuotaExhausted,
            "insufficient_quota",
        ));
        assert_eq!(error.fields.get("affordable_tokens"), None);
        assert_eq!(
            error.fallback_message(),
            "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue."
        );
    }
}
