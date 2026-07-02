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

pub const ANTHROPIC_SONNET: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::Anthropic,
    "claude-sonnet-4-6",
    "ANTHROPIC_API_KEY",
);

pub const OPENAI_GPT4O_MINI: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-4o-mini", "OPENAI_API_KEY");

pub const OPENAI_GPT52: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-5.2", "OPENAI_API_KEY").reasoning_as_text();

pub const OPENAI_GPT54: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-5.4", "OPENAI_API_KEY").reasoning_as_text();

pub const OPENAI_GPT55: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-5.5", "OPENAI_API_KEY").reasoning_as_text();

pub const GEMINI_FLASH: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::Gemini, "gemini-2.5-flash", "GEMINI_API_KEY");

// OpenRouter routes to upstream providers; gpt-4o-mini is cheap and reliably
// available. Exercises the Open Responses streaming path (incl. the `[DONE]`
// terminator) and the OpenRouter request decoration (session_id, routing).
pub const OPENROUTER_GPT4O_MINI: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::OpenRouter,
    "openai/gpt-4o-mini",
    "OPENROUTER_API_KEY",
);

// Fireworks AI serves open models via an OpenAI-compatible Chat Completions
// API. kimi-k2p5 is a chat + tool-calling model. Exercises the Chat Completions
// streaming path against a third (non-OpenAI/Azure) host.
pub const FIREWORKS_KIMI: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::Fireworks,
    "accounts/fireworks/models/kimi-k2p5",
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

// ============================================================================
// Transient transport / network flakiness
// ============================================================================

// The live LLM matrix talks to real providers over the network, so a turn can
// fail because the HTTP/SSE connection was interrupted mid-stream rather than
// because of any code regression. Historically these surface on `main` as
// `LLM error: Stream error: Transport error: error decoding response body`
// (reqwest/hyper failing to read the response body) and vary test-to-test,
// which is the signature of infrastructure flakiness, not a contract break.
//
// We treat these the same way as quota exhaustion: skip the case with a loud
// warning so `main` stays green, while genuine functional failures (auth,
// schema, model availability, assertion mismatches) still fail loudly.
//
// Detection is kept specific to transport-layer signatures so ordinary API
// errors are never swallowed. Auth/permission signals are excluded first, so a
// broken credential can never be misread as a transient hiccup.
pub fn is_transient_transport(err: &str) -> bool {
    let e = err.to_lowercase();

    // Never treat auth/permission failures as transient transport noise.
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

    // Transport-layer / connection-teardown signatures from reqwest/hyper and
    // the drivers' own error wrapping. These indicate the network stream was
    // interrupted, not that the provider rejected the request on its merits.
    e.contains("transport error")
        || e.contains("error decoding response body")
        || e.contains("connection reset")
        || e.contains("connection closed")
        || e.contains("connection aborted")
        || e.contains("connection refused")
        || e.contains("broken pipe")
        || e.contains("incomplete message")
        || e.contains("unexpected end of file")
        || e.contains("unexpected eof")
        || e.contains("error trying to connect")
        || e.contains("tcp connect error")
        || e.contains("dns error")
        || e.contains("timed out")
        || e.contains("timeout")
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
                if is_transient_transport(err) {
                    eprintln!(
                        "SKIP: provider {} transient transport error: {}",
                        $label, err
                    );
                    return;
                }
            }
        }
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
    registry
}

#[cfg(test)]
mod quota_detector_tests {
    use super::is_quota_exhausted;

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
        // Auth/permission failures must never be swallowed as quota.
        assert!(!is_quota_exhausted("401 Unauthorized: invalid api key"));
        assert!(!is_quota_exhausted("403 Forbidden"));
        assert!(!is_quota_exhausted(
            "insufficient_quota but actually invalid api key"
        ));
        assert!(!is_quota_exhausted("permission denied for this model"));
    }
}

#[cfg(test)]
mod transient_transport_tests {
    use super::is_transient_transport;

    #[test]
    fn matches_transport_flakes() {
        // The exact message observed flaking `main` from live OpenAI streaming.
        assert!(is_transient_transport(
            "LLM error: Stream error: Transport error: error decoding response body"
        ));
        assert!(is_transient_transport("error decoding response body"));
        assert!(is_transient_transport("connection reset by peer"));
        assert!(is_transient_transport(
            "connection closed before message completed"
        ));
        assert!(is_transient_transport("hyper: incomplete message"));
        assert!(is_transient_transport(
            "error trying to connect: tcp connect error"
        ));
        assert!(is_transient_transport("operation timed out"));
        assert!(is_transient_transport("request timeout"));
        // Case-insensitive.
        assert!(is_transient_transport("TRANSPORT ERROR: broken pipe"));
    }

    #[test]
    fn does_not_match_functional_or_auth_errors() {
        // Genuine functional/contract breaks must still fail the test.
        assert!(!is_transient_transport(
            "Model not available: gpt-99-nonexistent"
        ));
        assert!(!is_transient_transport(
            "Bad request: invalid schema for tool"
        ));
        assert!(!is_transient_transport("Internal server error"));
        assert!(!is_transient_transport(""));
        assert!(!is_transient_transport("rate_limit_exceeded"));
        // Auth/permission failures must never be swallowed as transport noise,
        // even when the message mentions a connection.
        assert!(!is_transient_transport("401 Unauthorized: invalid api key"));
        assert!(!is_transient_transport("403 Forbidden"));
        assert!(!is_transient_transport(
            "connection reset but actually 401 unauthorized"
        ));
        assert!(!is_transient_transport("permission denied for this model"));
    }
}
