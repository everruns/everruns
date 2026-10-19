//! Unit tests for the matrix harness: the skip detectors, the retry macro, and
//! the live tool-call contract. No live provider traffic.

use super::{
    LiveToolCallOutcome, assert_live_tool_call_contract, classify_live_tool_call,
    is_model_unavailable, is_quota_exhausted, is_transient_transport_error,
    is_upstream_shared_pool_rate_limited, live_retry_backoff,
};
use everruns_contracts::turn::TurnStopReason;
use everruns_contracts::typed_id::TurnId;
use everruns_test_support::in_memory_loop::{LlmGenerationSummary, TurnResult};

/// Label for the synthetic retry/skip tests below. Deliberately not a real
/// matrix cell: the macros print `SKIP: <label> out of quota` for a
/// synthetic quota error, and a real model id there reads in CI logs as a
/// genuine quota skip of that model.
/// Declares no environment variable, so the synthetic cell can never
/// resolve a real credential however the environment is configured.
fn synthetic_descriptor() -> everruns_contracts::driver_registry::DriverDescriptor {
    everruns_contracts::driver_registry::DriverDescriptor {
        credential_schema: everruns_contracts::credential_schema::CredentialFormSchema::empty(),
        ..everruns_contracts::driver_registry::DriverDescriptor::chat_only(
            super::DriverId::Anthropic,
            |_| unreachable!("the synthetic cell never builds a driver"),
        )
    }
}

const SYNTHETIC: super::ProviderModelConfig = super::ProviderModelConfig::new(
    super::DriverId::Anthropic,
    "synthetic-unit-test",
    synthetic_descriptor,
);

/// A turn that failed with `error`, for driving the retry macro.
fn failed_result(error: &str) -> TurnResult {
    TurnResult {
        response: String::new(),
        iterations: 1,
        tool_calls_count: 0,
        success: false,
        error: Some(error.into()),
        stop_reason: TurnStopReason::EndTurn,
        turn_id: TurnId::new(),
        llm_generations: vec![],
    }
}

fn tool_result(
    available_tools: &[&str],
    output_tool_calls_count: usize,
    finish_reasons: &[&str],
    executed_tool_calls: usize,
) -> TurnResult {
    TurnResult {
        response: "response".into(),
        iterations: if executed_tool_calls > 0 { 2 } else { 1 },
        tool_calls_count: executed_tool_calls,
        success: true,
        error: None,
        stop_reason: TurnStopReason::EndTurn,
        turn_id: TurnId::new(),
        llm_generations: vec![LlmGenerationSummary {
            available_tools: available_tools.iter().map(|name| (*name).into()).collect(),
            output_tool_calls_count,
            finish_reasons: finish_reasons
                .iter()
                .map(|reason| (*reason).into())
                .collect(),
            success: true,
        }],
    }
}

#[test]
fn classifies_clean_no_call_as_sampling_miss() {
    let result = tool_result(&["get_current_time"], 0, &["stop"], 0);
    assert_eq!(
        classify_live_tool_call(&result, "get_current_time"),
        LiveToolCallOutcome::SamplingMiss
    );
}

#[test]
#[should_panic(expected = "did not call \"get_current_time\" after sampling retries")]
fn sampling_miss_does_not_satisfy_live_contract() {
    let result = tool_result(&["get_current_time"], 0, &["stop"], 0);
    assert_live_tool_call_contract(&result, "get_current_time", "test provider");
}

#[test]
fn classifies_missing_definition_as_contract_failure() {
    let result = tool_result(&[], 0, &["stop"], 0);
    assert_eq!(
        classify_live_tool_call(&result, "get_current_time"),
        LiveToolCallOutcome::MissingToolDefinition
    );
}

#[test]
fn classifies_dropped_tool_call_as_contract_failure() {
    let result = tool_result(&["get_current_time"], 0, &["tool_calls"], 0);
    assert_eq!(
        classify_live_tool_call(&result, "get_current_time"),
        LiveToolCallOutcome::ToolCallPipelineMismatch
    );
}

#[test]
fn classifies_executed_tool_call_as_covered() {
    let result = tool_result(&["get_current_time"], 1, &["tool_calls"], 1);
    assert_eq!(
        classify_live_tool_call(&result, "get_current_time"),
        LiveToolCallOutcome::Exercised
    );
}

#[test]
fn matches_provider_quota_signatures() {
    // OpenAI / OpenRouter
    assert!(is_quota_exhausted(
        "LLM error: insufficient_quota: You exceeded your current quota, please check your plan and billing details."
    ));
    assert!(is_quota_exhausted("Error: insufficient_quota"));
    // Generic "exceeded ... quota" phrasing without the machine code.
    assert!(is_quota_exhausted(
        "You have exceeded your current quota for this month"
    ));
    // Anthropic real-world 400 credit-balance exhaustion (no "quota" word,
    // not a 429). This is the exact message returned when the account is
    // out of credits and must skip rather than fail the matrix.
    assert!(is_quota_exhausted(
        "LLM error: Anthropic API error (400 Bad Request): {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"Your credit balance is too low to access the Anthropic API. Please go to Plans & Billing to upgrade or purchase credits.\"}}"
    ));
    // Anthropic / Gemini style billing/credit exhaustion.
    assert!(is_quota_exhausted(
        "Your account has run out of credit; quota exhausted"
    ));
    assert!(is_quota_exhausted(
        "Request failed: billing quota has been reached"
    ));
    // Vercel AI Gateway with no card on file: a 403, which the auth guard
    // would otherwise reject, carrying an unambiguous billing machine code.
    assert!(is_quota_exhausted(
        "LLM error: provider 'vercel': OpenAI Responses API error (403 Forbidden): {\"error\":{\"message\":\"AI Gateway requires a valid credit card on file to service requests.\",\"type\":\"customer_verification_required\"}}"
    ));
    // Cloudflare with an unfunded account: a 402 with neither a quota word
    // nor "credit"/"billing", so only the explicit balance phrase matches.
    assert!(is_quota_exhausted(
        "LLM error: provider 'cloudflare': OpenAI API error (402 Payment Required): {\"errors\":[{\"message\":\"Insufficient balance; add money to your gateway or use BYOK\",\"code\":2021}]}"
    ));
    // HTTP 429 paired with a quota signal.
    assert!(is_quota_exhausted(
        "HTTP 429 Too Many Requests: insufficient_quota"
    ));
    assert!(is_quota_exhausted("429: quota exceeded for project"));
    // OpenAI real-world out-of-credits: 429 with `credit_balance_exhausted`
    // machine code and "You have no credits remaining." (no "quota" word,
    // no exhaustion phrase like "too low"). This is the exact message that
    // red main CI until matched here.
    assert!(is_quota_exhausted(
        "LLM error: credit_balance_exhausted: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/."
    ));
    // OpenRouter may reject an otherwise valid request when the requested
    // output ceiling exceeds the key's remaining credit limit.
    assert!(is_quota_exhausted(
        "OpenAI Responses API error (402 Payment Required): This request requires more credits, or fewer max_tokens. You requested up to 65536 tokens, but can only afford 24564."
    ));
    // Case-insensitive.
    assert!(is_quota_exhausted("INSUFFICIENT_QUOTA"));
}

#[test]
fn does_not_match_ordinary_or_auth_errors() {
    // Genuine functional/contract breaks must still fail the test.
    assert!(!is_quota_exhausted(
        "Model not available: gpt-99-nonexistent"
    ));
    assert!(!is_quota_exhausted("Bad request: invalid schema for tool"));
    assert!(!is_quota_exhausted("Internal server error"));
    assert!(!is_quota_exhausted(""));
    // Plain rate limiting (no billing/quota signal) should NOT be skipped.
    assert!(!is_quota_exhausted("HTTP 429 Too Many Requests: slow down"));
    assert!(!is_quota_exhausted(
        "HTTP 429 Too Many Requests: rate limit exceeded"
    ));
    assert!(!is_quota_exhausted(
        "HTTP 429 Too Many Requests: requests per minute exceeded"
    ));
    assert!(!is_quota_exhausted("rate_limit_exceeded"));
    // Billing-related prose without an exhaustion phrase must NOT be
    // swallowed (e.g. a declined card or a generic billing notice).
    assert!(!is_quota_exhausted("Please update your billing details"));
    assert!(!is_quota_exhausted("Your credit card was declined"));
    assert!(!is_quota_exhausted(
        "402 Payment Required: payment method was declined"
    ));
    assert!(!is_quota_exhausted(
        "Bad request: max_tokens exceeds the model limit"
    ));
    // Auth/permission failures must never be swallowed as quota.
    assert!(!is_quota_exhausted("401 Unauthorized: invalid api key"));
    assert!(!is_quota_exhausted("403 Forbidden"));
    assert!(!is_quota_exhausted(
        "insufficient_quota but actually invalid api key"
    ));
    assert!(!is_quota_exhausted("permission denied for this model"));
}

#[test]
fn matches_provider_overload_as_transient() {
    // The exact Anthropic payload that failed the Live Provider Matrix on
    // abda5cc — a capacity rejection, not a regression in our code.
    assert!(is_transient_transport_error(
        "LLM error: Anthropic stream error: {\"type\":\"error\",\"error\":{\"details\":null,\"type\":\"overloaded_error\",\"message\":\"Overloaded\"},\"request_id\":\"req_011CeggG6uTebixJra5EYWEx\"}"
    ));
    // Matched on the overload itself, not on the "stream error" wrapper, so
    // a driver reporting the same rejection differently still retries.
    assert!(is_transient_transport_error("overloaded_error"));
    assert!(is_transient_transport_error(
        "Anthropic API error (529): Overloaded"
    ));
    assert!(is_transient_transport_error("HTTP 503 Service Unavailable"));
    // Pre-existing transport signatures still match.
    assert!(is_transient_transport_error(
        "LLM error: Stream error: Transport error: error decoding response body"
    ));
}

#[test]
fn does_not_treat_real_failures_as_transient() {
    // A capacity rejection is transient; a functional break is not.
    assert!(!is_transient_transport_error(
        "Model not available: gpt-99-nonexistent"
    ));
    assert!(!is_transient_transport_error(
        "Bad request: invalid schema for tool"
    ));
    assert!(!is_transient_transport_error("401 Unauthorized"));
    assert!(!is_transient_transport_error(""));
    // Bare status-code digits must not match: they collide with token
    // counts and request ids, which is why 503/529 are not substrings.
    assert!(!is_transient_transport_error(
        "Bad request: max_tokens 65529 exceeds the model limit"
    ));
    assert!(!is_transient_transport_error(
        "Invalid request id req_503_abc: unknown tool"
    ));
}

#[test]
fn backoff_grows_exponentially_and_caps() {
    use std::time::Duration;
    assert_eq!(live_retry_backoff(1), Duration::from_secs(5));
    assert_eq!(live_retry_backoff(2), Duration::from_secs(10));
    assert_eq!(live_retry_backoff(3), Duration::from_secs(20));
    // Capped so a long retry budget cannot unbound the job.
    assert_eq!(live_retry_backoff(4), Duration::from_secs(30));
    assert_eq!(live_retry_backoff(50), Duration::from_secs(30));
}

/// Drive the macro itself with a synthetic turn so the retry/backoff path is
/// covered without live provider traffic. `start_paused` auto-advances
/// tokio's clock while the runtime is idle, so the real 15s of backoff costs
/// no wall-clock time but is still observable via `Instant::now()`.
#[tokio::test(start_paused = true)]
async fn transient_failure_retries_with_backoff() {
    use std::cell::Cell;
    use std::time::Duration;
    use tokio::time::Instant;

    // Synthetic results against a real config: must not record coverage.
    let _no_coverage = super::CoverageSuppressed::new();
    let config = SYNTHETIC;
    let attempts = Cell::new(0usize);
    let started = Instant::now();

    let outcome = run_live_turn!(config, 3, |r: &TurnResult| r.success, {
        attempts.set(attempts.get() + 1);
        failed_result("LLM error: Anthropic stream error: overloaded_error")
    });

    assert_eq!(attempts.get(), 3, "every attempt should be spent");
    assert!(
        outcome.is_some_and(|r| !r.success),
        "the last failure is returned so the caller's assertion reports it"
    );
    // 5s after attempt 1 + 10s after attempt 2; none after the last.
    assert_eq!(started.elapsed(), Duration::from_secs(15));
}

/// A model that cleanly declines the tool is not a transport problem, so the
/// retries must fire back to back with no delay.
#[tokio::test(start_paused = true)]
async fn sampling_miss_retries_without_backoff() {
    use std::cell::Cell;
    use std::time::Duration;
    use tokio::time::Instant;

    // Synthetic results against a real config: must not record coverage.
    let _no_coverage = super::CoverageSuppressed::new();
    let config = SYNTHETIC;
    let attempts = Cell::new(0usize);
    let started = Instant::now();

    let outcome = run_live_turn!(config, 3, |r: &TurnResult| r.tool_calls_count > 0, {
        attempts.set(attempts.get() + 1);
        tool_result(&["get_current_time"], 0, &["stop"], 0)
    });

    assert_eq!(attempts.get(), 3);
    assert!(outcome.is_some());
    assert_eq!(started.elapsed(), Duration::ZERO);
}

/// Quota exhaustion still short-circuits to a skip on the first attempt,
/// without spending retries or backoff on an account that cannot recover.
#[tokio::test(start_paused = true)]
async fn quota_exhaustion_skips_immediately() {
    use std::cell::Cell;
    use std::time::Duration;
    use tokio::time::Instant;

    // Synthetic results against a real config: must not record coverage.
    let _no_coverage = super::CoverageSuppressed::new();
    let config = SYNTHETIC;
    let attempts = Cell::new(0usize);
    let started = Instant::now();

    let outcome = run_live_turn!(config, 3, |r: &TurnResult| r.success, {
        attempts.set(attempts.get() + 1);
        failed_result("LLM error: insufficient_quota: You exceeded your current quota")
    });

    assert_eq!(attempts.get(), 1, "quota is terminal, not worth retrying");
    assert!(outcome.is_none(), "caller skips on None");
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[test]
fn matches_the_driver_retired_model_mapping() {
    // The exact string `AgentLoopError::ModelNotAvailable` renders, as it
    // reached CI from Fireworks in #3851.
    assert!(is_model_unavailable(
        "Model not available: accounts/fireworks/models/kimi-k2p6"
    ));
    // Wrapped by the atom/activity error chain, which is how a worker turn
    // surfaces it.
    assert!(is_model_unavailable(
        "ReasonAtom execution failed: Model not available: retired-model"
    ));
}

#[test]
fn does_not_treat_auth_or_ordinary_failures_as_a_retired_model() {
    // A revoked or wrong key must fail loudly, never skip — even when the
    // provider phrases it in terms of model access.
    assert!(!is_model_unavailable(
        "403 Forbidden: Model not available: kimi-k3"
    ));
    assert!(!is_model_unavailable(
        "Invalid API key: Model not available: kimi-k3"
    ));
    assert!(!is_model_unavailable("401 authentication_error"));
    // Unrelated failures stay fatal.
    assert!(!is_model_unavailable("Stream error: connection reset"));
    assert!(!is_model_unavailable("insufficient_quota"));
    assert!(!is_model_unavailable("Request too large"));
}

#[tokio::test(start_paused = true)]
async fn retired_model_skips_immediately() {
    use std::cell::Cell;
    use std::time::Duration;
    use tokio::time::Instant;

    // Synthetic results against a real config: must not record coverage.
    let _no_coverage = super::CoverageSuppressed::new();
    let config = SYNTHETIC;
    let attempts = Cell::new(0usize);
    let started = Instant::now();

    let outcome = run_live_turn!(config, 3, |r: &TurnResult| r.success, {
        attempts.set(attempts.get() + 1);
        failed_result("LLM error: Model not available: accounts/fireworks/models/gone")
    });

    assert_eq!(
        attempts.get(),
        1,
        "a retired model id is deterministic, so re-asking cannot help"
    );
    assert!(outcome.is_none(), "caller skips on None");
    assert_eq!(started.elapsed(), Duration::ZERO);
}

/// The exact error the OpenRouter Mistral Large 4 cell failed with on main
/// (7717d560), as the driver rendered it into the turn error.
const OPENROUTER_SHARED_POOL_429: &str = r#"LLM error: provider 'openrouter': OpenAI Responses API error (429 Too Many Requests): {"error":{"message":"Provider returned error","code":429,"metadata":{"raw":"mistralai/mistral-large-4-0 is temporarily rate-limited upstream. Please retry shortly, or add your own key to accumulate your rate limits: https://openrouter.ai/settings/integrations","provider_name":"Mistral","is_byok":false,"limit_source":"upstream_provider_shared_pool","remedy_hint":"Retry shortly, add your own provider key (https://openrouter.ai/settings/integrations), or route to another provider with provider routing: https://openrouter.ai/docs/features/provider-routing","previous_errors":[{"code":429,"message":"Provider returned error","provider_name":"Mistral","raw":"mistralai/mistral-large-4-0 is temporarily rate-limited upstream. Please retry shortly, or add your own key to accumulate your rate limits: https://openrouter.ai/settings/integrations"}]}},"user_id":"user_x"}"#;

#[test]
fn matches_openrouter_shared_pool_rate_limit() {
    assert!(is_upstream_shared_pool_rate_limited(
        OPENROUTER_SHARED_POOL_429
    ));
    // Re-escaped by a `Debug`-formatted error chain, as the CI log shows it.
    assert!(is_upstream_shared_pool_rate_limited(&format!(
        "{OPENROUTER_SHARED_POOL_429:?}"
    )));
    // Pretty-printed JSON still matches.
    assert!(is_upstream_shared_pool_rate_limited(
        "HTTP 429: {\"is_byok\": false, \"limit_source\": \"upstream_provider_shared_pool\"}"
    ));
}

#[test]
fn shared_pool_detector_keeps_our_own_rate_limits_fatal() {
    // Not a quota condition either: the existing detector must stay narrow.
    assert!(!is_quota_exhausted(OPENROUTER_SHARED_POOL_429));
    // With our own provider key, the limit is ours.
    assert!(!is_upstream_shared_pool_rate_limited(
        &OPENROUTER_SHARED_POOL_429.replace("\"is_byok\":false", "\"is_byok\":true")
    ));
    // A limit sourced from our key/account is ours.
    assert!(!is_upstream_shared_pool_rate_limited(
        &OPENROUTER_SHARED_POOL_429
            .replace("upstream_provider_shared_pool", "openrouter_key_limit")
    ));
    // Both fields are required, not either.
    assert!(!is_upstream_shared_pool_rate_limited(
        "429: {\"limit_source\":\"upstream_provider_shared_pool\"}"
    ));
    assert!(!is_upstream_shared_pool_rate_limited(
        "429: {\"is_byok\":false}"
    ));
    // Plain rate limiting and the prose alone stay fatal.
    assert!(!is_upstream_shared_pool_rate_limited(
        "HTTP 429 Too Many Requests: rate limit exceeded"
    ));
    assert!(!is_upstream_shared_pool_rate_limited(
        "mistralai/mistral-large-4-0 is temporarily rate-limited upstream"
    ));
    assert!(!is_upstream_shared_pool_rate_limited(""));
}

/// A shared-pool limit is waited out with the transient backoff first, and
/// only becomes a skip when it outlasts every attempt.
#[tokio::test(start_paused = true)]
async fn shared_pool_rate_limit_retries_then_skips() {
    use std::cell::Cell;
    use std::time::Duration;
    use tokio::time::Instant;

    // Synthetic results against a real config: must not record coverage.
    let _no_coverage = super::CoverageSuppressed::new();
    let config = SYNTHETIC;
    let attempts = Cell::new(0usize);
    let started = Instant::now();

    let outcome = run_live_turn!(config, 3, |r: &TurnResult| r.success, {
        attempts.set(attempts.get() + 1);
        failed_result(OPENROUTER_SHARED_POOL_429)
    });

    assert_eq!(attempts.get(), 3, "every attempt is spent before skipping");
    assert!(outcome.is_none(), "caller skips on None");
    assert_eq!(started.elapsed(), Duration::from_secs(15));
}

/// A pool that drains mid-run still verifies the cell.
#[tokio::test(start_paused = true)]
async fn shared_pool_rate_limit_recovers_on_retry() {
    use std::cell::Cell;

    let _no_coverage = super::CoverageSuppressed::new();
    let config = SYNTHETIC;
    let attempts = Cell::new(0usize);

    let outcome = run_live_turn!(config, 3, |r: &TurnResult| r.success, {
        attempts.set(attempts.get() + 1);
        if attempts.get() == 1 {
            failed_result(OPENROUTER_SHARED_POOL_429)
        } else {
            tool_result(&[], 0, &["stop"], 0)
        }
    });

    assert_eq!(attempts.get(), 2);
    assert!(outcome.is_some_and(|r| r.success));
}

/// A plain 429 is not skipped: the last failure reaches the caller's assert.
#[tokio::test(start_paused = true)]
async fn plain_rate_limit_is_not_skipped() {
    let _no_coverage = super::CoverageSuppressed::new();
    let config = SYNTHETIC;

    let outcome = run_live_turn!(config, 3, |r: &TurnResult| r.success, {
        failed_result("LLM error: 429 Too Many Requests: rate limit exceeded")
    });

    assert!(outcome.is_some_and(|r| !r.success));
}
