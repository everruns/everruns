// Shared provider/model configuration for parametrized LLM integration tests.
//
// Defines ProviderModelConfig structs and a unified DriverRegistry so test
// files can iterate over providers × models without duplicating helpers.
//
// Add new providers/models here — all test files pick them up automatically.

#![allow(dead_code)] // Not all test binaries use every constant.

use everruns_core::driver_registry::DriverRegistry;
use everruns_core::provider::DriverId;
use everruns_core::traits::ResolvedModel;
use everruns_test_support::in_memory_loop::TurnResult;

// ============================================================================
// Provider + Model configuration
// ============================================================================

/// One cell in the test matrix: a (provider, model, env-var) tuple.
#[derive(Clone, Debug)]
pub struct ProviderModelConfig {
    pub provider_type: DriverId,
    pub model_name: &'static str,
    pub env_var: &'static str,
    /// Whether the model surfaces extended reasoning on the private `thinking`
    /// field. Anthropic (and OpenAI o-series encrypted reasoning) do. OpenAI
    /// GPT-5.x instead surfaces its readable reasoning *summary* as public
    /// text, leaving `thinking` empty — see the `ReasoningSummaryDelta`
    /// mapping in `openresponses_protocol`.
    pub reasoning_on_thinking_field: bool,
}

impl ProviderModelConfig {
    pub const fn new(
        provider_type: DriverId,
        model_name: &'static str,
        env_var: &'static str,
    ) -> Self {
        Self {
            provider_type,
            model_name,
            env_var,
            reasoning_on_thinking_field: true,
        }
    }

    /// Mark this model as surfacing its reasoning summary as public text rather
    /// than on the private `thinking` field (OpenAI GPT-5.x behavior).
    pub const fn reasoning_as_text(mut self) -> Self {
        self.reasoning_on_thinking_field = false;
        self
    }

    /// Build a `ResolvedModel` from env, returning `None` if the key is
    /// missing or empty, or if the provider appears in
    /// `SKIP_LLM_INTEGRATION_TESTS_PROVIDERS` (comma-separated, e.g.
    /// `SKIP_LLM_INTEGRATION_TESTS_PROVIDERS=gemini,openai`).
    pub fn model(&self) -> Option<ResolvedModel> {
        if let Ok(skip) = std::env::var("SKIP_LLM_INTEGRATION_TESTS_PROVIDERS") {
            let provider = self.provider_type.to_string().to_lowercase();
            if skip.split(',').any(|s| s.trim().to_lowercase() == provider) {
                return None;
            }
        }
        let api_key = std::env::var(self.env_var).ok().filter(|k| !k.is_empty())?;
        Some(ResolvedModel {
            model: self.model_name.to_string(),
            provider_type: self.provider_type.clone(),
            api_key: Some(api_key),
            base_url: None,
            provider_metadata: None,
        })
    }

    /// Human-readable label for skip messages.
    pub fn label(&self) -> String {
        format!("{}:{}", self.env_var, self.model_name)
    }
}

impl std::fmt::Display for ProviderModelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.provider_type, self.model_name)
    }
}

// ============================================================================
// Provider catalogue — add new providers/models here
// ============================================================================

// Not wired into the live matrix: `claude-fable-5` is listed by the Anthropic
// `/models` endpoint but returns "Model not available" on inference from the
// CI key. Re-add the `#[case]` entries once it's usable; Anthropic stays
// covered via haiku/opus/sonnet below.
pub const ANTHROPIC_FABLE: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::Anthropic, "claude-fable-5", "ANTHROPIC_API_KEY");

pub const ANTHROPIC_HAIKU: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::Anthropic,
    "claude-haiku-4-5-20251001",
    "ANTHROPIC_API_KEY",
);

// Bare alias — the API does not serve a dated id for Opus 4.7.
pub const ANTHROPIC_OPUS: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::Anthropic, "claude-opus-4-7", "ANTHROPIC_API_KEY");

pub const ANTHROPIC_OPUS5: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::Anthropic, "claude-opus-5", "ANTHROPIC_API_KEY");

pub const ANTHROPIC_SONNET: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::Anthropic,
    "claude-sonnet-4-6",
    "ANTHROPIC_API_KEY",
);

pub const OPENAI_GPT56_LUNA: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-5.6-luna", "OPENAI_API_KEY")
        .reasoning_as_text();

pub const OPENAI_GPT52: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-5.2", "OPENAI_API_KEY").reasoning_as_text();

pub const OPENAI_GPT54: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-5.4", "OPENAI_API_KEY").reasoning_as_text();

pub const OPENAI_GPT55: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-5.5", "OPENAI_API_KEY").reasoning_as_text();

pub const GEMINI_FLASH: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::Gemini, "gemini-2.5-flash", "GEMINI_API_KEY");

// Use Meta's lower-cost Contributor tier for the live matrix. Its data-use
// terms are acceptable for these synthetic test prompts.
pub const META_MUSE_SPARK_CONTRIBUTOR: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::Meta,
    "muse-spark-1.2-contributor",
    "MODEL_API_KEY",
)
.reasoning_as_text();

// OpenRouter routes to upstream providers; Luna is the fast, cost-efficient
// GPT-5.6 tier. Exercises the Open Responses streaming path (incl. the `[DONE]`
// terminator) and the OpenRouter request decoration (session_id, routing).
pub const OPENROUTER_GPT56_LUNA: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::OpenRouter,
    "openai/gpt-5.6-luna",
    "OPENROUTER_API_KEY",
)
.reasoning_as_text();

// Fireworks AI serves open models via an OpenAI-compatible Chat Completions
// API. The point of this case is to exercise our OpenAI-protocol driver's
// streaming + tool-calling path against a third (non-OpenAI/Azure) host — not
// to probe a model's intelligence — so the model must call tools reliably for
// the `test_tool_call` assertion to be deterministic. Kimi K2 is purpose-built
// for agentic tool use and calls tools deterministically. Fireworks has
// churned its serverless catalog repeatedly: `gpt-oss-120b` flaked the "must
// call tool" assert (#2550/#2556), and `llama-v3p3-70b-instruct` (#2597) was
// then de-listed from the account entirely ("Model not available"). Kimi K2 is
// a current, dependable tool-caller in the served catalog.
pub const FIREWORKS_KIMI_K2: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::Fireworks,
    "accounts/fireworks/models/kimi-k2p6",
    "FIREWORKS_API_KEY",
);

// Bedrock: credentials are JSON in the env var, not a plain API key.
// Set AWS_BEDROCK_CREDENTIALS to the JSON credential object.
pub const BEDROCK_HAIKU: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::Bedrock,
    "anthropic.claude-3-5-haiku-20241022-v1:0",
    "AWS_BEDROCK_CREDENTIALS",
);

pub const BEDROCK_SONNET: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::Bedrock,
    "anthropic.claude-3-5-sonnet-20241022-v2:0",
    "AWS_BEDROCK_CREDENTIALS",
);

// ============================================================================
// Provider quota / billing exhaustion handling
// ============================================================================

// The live LLM matrix is best-effort: it already skips providers whose API key
// is absent (see `ProviderModelConfig::model`). When a provider account is
// merely out of credits, a turn fails with a billing/quota error from the live
// API rather than a code regression. We treat that the same way as an absent
// key: skip the case with a loud warning so `main` stays green, while a genuine
// API/contract break still fails the test.
//
// Detection is kept specific so ordinary failures (auth, permission, model
// availability, schema, rate limits unrelated to billing) still fail loudly:
//   - `insufficient_quota` (OpenAI / OpenRouter)
//   - `quota` together with `exceeded`/`billing`/`credit` (Gemini, Anthropic,
//     generic phrasings like "exceeded your current quota")
//   - OpenRouter's explicit "requires more credits" / "can only afford" 402
//   - HTTP 429 carrying a quota/billing signature (not a bare rate-limit)
//
// Authn/authz signals (`unauthorized`, `forbidden`, `invalid api key`,
// `permission`, 401/403) are deliberately NOT matched, so a broken credential
// is never silently swallowed.
pub fn is_quota_exhausted(err: &str) -> bool {
    let e = err.to_lowercase();

    // Never treat auth/permission failures as quota exhaustion.
    let auth_failure = e.contains("unauthorized")
        || e.contains("forbidden")
        || e.contains("invalid api key")
        || e.contains("invalid_api_key")
        || e.contains("permission")
        || e.contains("authentication")
        || e.contains(" 401")
        || e.contains("401 ")
        || e.contains(" 403")
        || e.contains("403 ");
    if auth_failure {
        return false;
    }

    if e.contains("insufficient_quota") {
        return true;
    }

    // OpenAI's explicit out-of-credits machine code / phrasing. Returned as a
    // 429 with body `credit_balance_exhausted: You have no credits remaining.`
    // — no "quota" word, and "no credits remaining" isn't an exhaustion phrase
    // below, so match it directly here.
    if e.contains("credit_balance_exhausted") {
        return true;
    }

    // OpenRouter reports an account/key credit ceiling as HTTP 402 without a
    // quota machine code. Require both halves of its affordability message so
    // a generic payment error or max_tokens validation failure stays fatal.
    if e.contains("requires more credits") && e.contains("can only afford") {
        return true;
    }

    let quota_signal = e.contains("quota") || e.contains("out of quota");
    let billing_signal = e.contains("billing") || e.contains("credit");
    let exceeded_signal = e.contains("exceeded");
    if quota_signal && (billing_signal || exceeded_signal) {
        return true;
    }

    // Explicit credit/balance exhaustion that never uses the word "quota".
    // Anthropic returns this as a 400 `invalid_request_error`, not a 429:
    //   "Your credit balance is too low to access the Anthropic API. Please go
    //    to Plans & Billing to upgrade or purchase credits."
    // Require the billing/credit signal to be paired with an exhaustion phrase
    // so ordinary billing-related prose isn't swallowed.
    let exhaustion_phrase = e.contains("too low")
        || e.contains("run out")
        || e.contains("ran out")
        || e.contains("out of credit")
        || e.contains("no credit") // "you have no credits remaining" (OpenAI)
        || e.contains("purchase credit")
        || e.contains("depleted");
    if billing_signal && exhaustion_phrase {
        return true;
    }

    // HTTP 429 only counts when paired with an explicit quota/billing signal,
    // so plain rate limiting (which should be retried/flagged, not skipped)
    // still fails even if the provider says a rate limit was "exceeded".
    if e.contains("429") && (quota_signal || billing_signal) {
        return true;
    }

    false
}

/// Skip the current test (with a loud stderr warning) if `result` failed due to
/// the provider being out of quota/credits. Otherwise the test proceeds and its
/// normal assertions run. `result` must expose `success: bool` and
/// `error: Option<String>`.
///
/// Defined inside this shared module and re-exported via `use
/// llm_test_matrix::*` so every test binary that includes the module can use it.
/// `is_quota_exhausted` is called unqualified, resolved through the same glob
/// import the test files already rely on.
#[macro_export]
macro_rules! skip_if_quota {
    ($result:expr, $label:expr) => {{
        // Bind once by reference so the expression isn't evaluated twice (no
        // duplicated side effects / moves if a caller passes a non-trivial expr).
        let __result = &$result;
        if !__result.success {
            if let Some(err) = __result.error.as_deref() {
                if is_quota_exhausted(err) {
                    eprintln!("SKIP: provider {} out of quota: {}", $label, err);
                    return;
                }
            }
        }
    }};
}

/// Substrings that mark a *transient* live-transport failure (network blip,
/// streaming-decode hiccup, timeout) rather than a real, reproducible error.
/// These tests hit real provider endpoints, so a single transport hiccup should
/// be retried, not reported as a regression — e.g. the observed flake
/// `LLM error: Stream error: Transport error: error decoding response body`.
pub fn is_transient_transport_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    [
        "transport error",
        "stream error",
        "error decoding response body",
        "connection reset",
        "connection closed",
        "connection error",
        "connection refused",
        "timed out",
        "timeout",
        "broken pipe",
        "incomplete message",
        "unexpected eof",
        "tls",
    ]
    .iter()
    .any(|s| e.contains(s))
}

/// Outcome of the request/response contract checks for a live tool-call turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveToolCallOutcome {
    /// The expected tool was advertised and the turn exercised it end-to-end.
    Exercised,
    /// The expected tool was advertised, but the model cleanly chose text instead.
    SamplingMiss,
    /// A successful provider generation omitted the expected tool.
    MissingToolDefinition,
    /// The provider signalled or emitted tool calls, but the turn executed none.
    ToolCallPipelineMismatch,
    /// The harness captured no provider-generation evidence for a successful turn.
    MissingGenerationEvidence,
}

/// Classify a live tool-call result using request-side and response-side evidence.
///
/// A clean `stop` after the expected tool was advertised is model sampling, not a
/// product regression. Missing request evidence, a missing tool definition, or a
/// provider/tool-loop count mismatch is a contract failure and must remain fatal.
pub fn classify_live_tool_call(result: &TurnResult, expected_tool: &str) -> LiveToolCallOutcome {
    if result.llm_generations.is_empty() {
        return LiveToolCallOutcome::MissingGenerationEvidence;
    }

    if result
        .llm_generations
        .iter()
        .filter(|generation| generation.success)
        .any(|generation| {
            !generation
                .available_tools
                .iter()
                .any(|tool| tool == expected_tool)
        })
    {
        return LiveToolCallOutcome::MissingToolDefinition;
    }

    let provider_reported_tool_calls = result.llm_generations.iter().any(|generation| {
        generation.output_tool_calls_count > 0
            || generation
                .finish_reasons
                .iter()
                .any(|reason| reason == "tool_calls" || reason == "tool_use")
    });
    if provider_reported_tool_calls && result.tool_calls_count == 0 {
        return LiveToolCallOutcome::ToolCallPipelineMismatch;
    }

    if result.tool_calls_count > 0 {
        LiveToolCallOutcome::Exercised
    } else {
        LiveToolCallOutcome::SamplingMiss
    }
}

/// Enforce live tool-call contracts while keeping clean model sampling misses
/// out of the merge gate. Returns whether the tool path was exercised.
pub fn assert_live_tool_call_contract(
    result: &TurnResult,
    expected_tool: &str,
    label: &str,
) -> bool {
    let outcome = classify_live_tool_call(result, expected_tool);
    match outcome {
        LiveToolCallOutcome::Exercised => true,
        LiveToolCallOutcome::SamplingMiss => {
            eprintln!(
                "::warning title=Live LLM sampling miss::{label} advertised {expected_tool} \
                 but the model cleanly returned without calling it"
            );
            eprintln!("SAMPLING MISS: generations={:?}", result.llm_generations,);
            false
        }
        LiveToolCallOutcome::MissingToolDefinition => panic!(
            "TOOL CONTRACT FAILURE: {label} did not advertise {expected_tool:?} on every \
             successful generation; generations={:?}",
            result.llm_generations,
        ),
        LiveToolCallOutcome::ToolCallPipelineMismatch => panic!(
            "TOOL CONTRACT FAILURE: {label} provider output reported tool calls but the \
             turn executed none; generations={:?}",
            result.llm_generations,
        ),
        LiveToolCallOutcome::MissingGenerationEvidence => {
            panic!("TOOL CONTRACT FAILURE: {label} completed without llm.generation evidence")
        }
    }
}

/// Run a live turn with bounded retries against real-model non-determinism and
/// transient transport failures. `$run` is a block that builds a runner and
/// awaits a turn, evaluating to a `TurnResult`; it is re-run up to `$max` times.
///
/// Returns `None` when the provider is out of quota (caller should skip), or
/// `Some(result)` for the first attempt that satisfies the `$ok` predicate —
/// otherwise the last attempt after exhausting retries, so the caller's own
/// assertions still produce a precise failure message. Retries fire when a turn
/// hit a transient transport error or `$ok` was not yet met.
///
/// Exported via `#[macro_export]` so every test binary that includes this
/// shared module can use it. `is_quota_exhausted`, `is_transient_transport_error`,
/// and `TurnResult` are referenced unqualified and resolve at each call site
/// (the test files already glob-import this module and `TurnResult`).
#[macro_export]
macro_rules! run_live_turn {
    ($config:expr, $max:expr, $ok:expr, $run:block) => {{
        let ok_fn = $ok;
        let mut outcome: Option<TurnResult> = None;
        for attempt in 1..=$max {
            let result: TurnResult = $run;
            if !result.success {
                if let Some(err) = result.error.as_deref() {
                    if is_quota_exhausted(err) {
                        eprintln!("SKIP: {} out of quota: {}", $config.label(), err);
                        outcome = None;
                        break;
                    }
                }
            }
            if ok_fn(&result) {
                outcome = Some(result);
                break;
            }
            let transient = !result.success
                && result
                    .error
                    .as_deref()
                    .is_some_and(is_transient_transport_error);
            eprintln!(
                "{}: attempt {attempt}/{} not acceptable \
                 (success={}, transient_transport={}, tool_calls={}, iterations={}, error={:?}, \
                 generations={:?}); {}",
                $config.label(),
                $max,
                result.success,
                transient,
                result.tool_calls_count,
                result.iterations,
                result.error,
                result.llm_generations,
                if attempt < $max {
                    "retrying"
                } else {
                    "giving up"
                },
            );
            outcome = Some(result);
        }
        outcome
    }};
}

// ============================================================================
// Unified driver registry
// ============================================================================

/// Registry with all real providers registered.
pub fn all_providers_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut registry);
    everruns_openai::register_driver(&mut registry);
    everruns_openrouter::register_driver(&mut registry);
    everruns_fireworks::register_driver(&mut registry);
    everruns_gemini::register_driver(&mut registry);
    everruns_bedrock::register_driver(&mut registry);
    everruns_meta::register_driver(&mut registry);
    registry
}

#[cfg(test)]
mod quota_detector_tests {
    use super::{LiveToolCallOutcome, classify_live_tool_call, is_quota_exhausted};
    use everruns_core::turn::TurnStopReason;
    use everruns_core::typed_id::TurnId;
    use everruns_test_support::in_memory_loop::{LlmGenerationSummary, TurnResult};

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
}
