// Shared provider/model configuration for parametrized LLM integration tests.
//
// Defines ProviderModelConfig structs and a unified DriverRegistry so test
// files can iterate over providers × models without duplicating helpers.
//
// Add new providers/models here — all test files pick them up automatically.

#![allow(dead_code)] // Not all test binaries use every constant.

use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::driver_registry::ProviderConfig;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_test_support::in_memory_loop::{InMemoryModelConfig, TurnResult};

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

    /// Build a `ModelSpec` from env, returning `None` if the key is
    /// missing or empty, or if the provider appears in
    /// `SKIP_LLM_INTEGRATION_TESTS_PROVIDERS` (comma-separated, e.g.
    /// `SKIP_LLM_INTEGRATION_TESTS_PROVIDERS=gemini,openai`).
    pub fn model(&self) -> Option<InMemoryModelConfig> {
        if let Ok(skip) = std::env::var("SKIP_LLM_INTEGRATION_TESTS_PROVIDERS") {
            let provider = self.provider_type.to_string().to_lowercase();
            if skip.split(',').any(|s| s.trim().to_lowercase() == provider) {
                self.record_outcome(CELL_SKIP_LIST);
                return None;
            }
        }
        let Some(api_key) = std::env::var(self.env_var).ok().filter(|k| !k.is_empty()) else {
            self.record_outcome(CELL_NO_KEY);
            return None;
        };
        let model = ModelSpec::on(self.provider_type.as_str(), self.model_name);
        let provider = ProviderConfig::new(self.provider_type.clone()).with_api_key(api_key);
        self.record_outcome(CELL_CONFIGURED);
        Some((model, provider).into())
    }

    /// Human-readable label for skip messages.
    pub fn label(&self) -> String {
        format!("{}:{}", self.env_var, self.model_name)
    }

    /// Record this cell as reached-but-unverified (out of quota/credits).
    ///
    /// A method rather than `record_outcome(CELL_QUOTA)` at the call site
    /// because both callers are `#[macro_export]` macros: a constant would have
    /// to resolve in each expansion's scope, which it does not in this module's
    /// own `mod tests` or in test binaries that do not glob-import the constant.
    /// Method resolution needs no import.
    pub fn record_quota(&self) {
        self.record_outcome(CELL_QUOTA);
    }

    /// Append one machine-readable coverage record for this matrix cell to the
    /// file named by `LLM_MATRIX_COVERAGE_FILE`, when that variable is set
    /// (EVE-951). A no-op otherwise, so local runs are unchanged.
    ///
    /// The human-readable `eprintln!` skip messages next to each call site stay
    /// as they are — they explain a single test to whoever is reading the log.
    /// These records answer the different question the job summary needs: which
    /// cells actually reached a provider, and which quietly did not.
    ///
    /// A file rather than stdout for two reasons. Test stdout and stderr
    /// interleave under `--nocapture` with tests running in parallel, which
    /// splices records into the middle of other lines and makes them
    /// unparseable. And this module's own `mod tests` drives the recording
    /// macros with synthetic configs, so anything written unconditionally to
    /// the log would report fake cells as real coverage; CI sets the variable
    /// only for the live matrix run.
    ///
    /// Records are appended with a single `write_all` of one short line, which
    /// `O_APPEND` keeps atomic across the parallel test threads and across the
    /// separate test binaries the step runs into the same file.
    ///
    /// `model()` is called more than once per test (the `is_none()` guard, then
    /// again inside the turn, once per `run_live_turn!` attempt), so a cell
    /// records the same outcome repeatedly. Folding duplicates is the reader's
    /// job — `scripts/report_live_matrix_coverage.py` reduces per cell — which
    /// keeps this side free of any cross-test state.
    ///
    /// Recording is best-effort: a coverage report must never be able to fail
    /// the live matrix it is only describing, so I/O errors are dropped.
    pub fn record_outcome(&self, outcome: &str) {
        use std::io::Write;

        if coverage_suppressed() {
            return;
        }
        let Ok(path) = std::env::var(COVERAGE_FILE_ENV) else {
            return;
        };
        if path.is_empty() {
            return;
        }
        let line = format!(
            "{outcome}\t{}\t{}\t{}\n",
            self.provider_type.to_string().to_lowercase(),
            self.env_var,
            self.model_name,
        );
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

/// Names the file matrix cells append their coverage records to. Unset outside
/// the `live-provider-matrix` job, which makes recording a no-op everywhere else.
pub const COVERAGE_FILE_ENV: &str = "LLM_MATRIX_COVERAGE_FILE";

thread_local! {
    static COVERAGE_SUPPRESSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn coverage_suppressed() -> bool {
    COVERAGE_SUPPRESSED.with(std::cell::Cell::get)
}

/// Suppresses coverage recording on the current thread until dropped.
///
/// This module's own unit tests drive `run_live_turn!` and `skip_if_quota!`
/// with synthetic `TurnResult`s to assert the retry and skip behaviour, and
/// they carry a *real* config (`ANTHROPIC_OPUS5`) as the label. Without this
/// guard those tests append genuine-looking records — a live matrix run would
/// then report `claude-opus-5` as quota-skipped when nothing of the sort
/// happened, which is precisely the false coverage picture EVE-951 exists to
/// remove.
///
/// Thread-local rather than an env var: the harness runs tests in parallel, so
/// mutating process environment here would race across tests. Every test that
/// needs it uses a current-thread runtime, so the flag and the code it guards
/// are on the same thread.
pub struct CoverageSuppressed(());

impl CoverageSuppressed {
    pub fn new() -> Self {
        COVERAGE_SUPPRESSED.with(|c| c.set(true));
        Self(())
    }
}

impl Default for CoverageSuppressed {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CoverageSuppressed {
    fn drop(&mut self) {
        COVERAGE_SUPPRESSED.with(|c| c.set(false));
    }
}

/// The cell had a usable API key and reached the provider.
pub const CELL_CONFIGURED: &str = "configured";

/// The cell was skipped because its API key was absent or empty.
pub const CELL_NO_KEY: &str = "no-key";

/// The cell was skipped through `SKIP_LLM_INTEGRATION_TESTS_PROVIDERS`.
pub const CELL_SKIP_LIST: &str = "skip-list";

/// The cell reached the provider but the account was out of quota/credits, so
/// its assertions never ran. A billing condition, not a code regression — but
/// it leaves the cell unverified, which is what the report exists to surface.
pub const CELL_QUOTA: &str = "quota";

impl std::fmt::Display for ProviderModelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.provider_type, self.model_name)
    }
}

// ============================================================================
// Provider catalogue — add new providers/models here
// ============================================================================

// Fable 5.1 is in the live matrix (top tier, $10/$50 per MTok — keep its
// cases to the basic/thinking suites). Fable 5 stays out: same tier, same
// surface, superseded. The earlier "Model not available" on Fable dates from
// the period when Anthropic had it disabled; both ids serve inference on the
// Doppler `ANTHROPIC_API_KEY` (verified 2026-09-02).
pub const ANTHROPIC_FABLE_5_1: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::Anthropic, "claude-fable-5-1", "ANTHROPIC_API_KEY");
pub const ANTHROPIC_FABLE: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::Anthropic, "claude-fable-5", "ANTHROPIC_API_KEY");

pub const ANTHROPIC_HAIKU: ProviderModelConfig = ProviderModelConfig::new(
    DriverId::Anthropic,
    "claude-haiku-4-5-20251001",
    "ANTHROPIC_API_KEY",
);

// Current Anthropic tiers only; superseded Opus 4.7 / Sonnet 4.6 entries were
// dropped when Opus 5 / Sonnet 5 took their matrix rows.
pub const ANTHROPIC_OPUS5: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::Anthropic, "claude-opus-5", "ANTHROPIC_API_KEY");

pub const ANTHROPIC_SONNET5: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::Anthropic, "claude-sonnet-5", "ANTHROPIC_API_KEY");

pub const OPENAI_GPT56_LUNA: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-5.6-luna", "OPENAI_API_KEY")
        .reasoning_as_text();

// GPT-6 Astra is the current OpenAI flagship (verified serving inference on
// the Doppler `OPENAI_API_KEY` on 2026-09-04). Unlike every other OpenAI case
// here, its reasoning item never carries readable summary text (`content: []`
// even with `summary: "auto"`) — only opaque `encrypted_content` — so it is
// NOT covered by `test_extended_thinking` in agent_run_with_thinking.rs,
// which asserts on readable reasoning text. It is covered by the basic/
// tool-call suites and by `test_thinking_with_tool_call`, none of which
// require readable reasoning.
pub const OPENAI_GPT6_ASTRA: ProviderModelConfig =
    ProviderModelConfig::new(DriverId::OpenAI, "gpt-6-astra", "OPENAI_API_KEY");

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
    "muse-spark-1.3-contributor",
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
    // `cell = <ProviderModelConfig>` — a real matrix cell. Records a `quota`
    // coverage marker (EVE-951) so the job summary reports the cell as
    // unverified instead of letting a green test result imply it was checked.
    ($result:expr, cell = $config:expr) => {{
        // Bind once by reference so the expression isn't evaluated twice (no
        // duplicated side effects / moves if a caller passes a non-trivial expr).
        let __result = &$result;
        let __config = &$config;
        if !__result.success {
            if let Some(err) = __result.error.as_deref() {
                if is_quota_exhausted(err) {
                    eprintln!("SKIP: provider {} out of quota: {}", __config.label(), err);
                    __config.record_quota();
                    return;
                }
            }
        }
    }};
    // `label = <&str>` — not a matrix cell. The nonexistent-model negative test
    // builds its own `ModelSpec` rather than a `ProviderModelConfig`, so it has
    // no cell to report coverage for.
    ($result:expr, label = $label:expr) => {{
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
/// streaming-decode hiccup, timeout, provider-side overload) rather than a real,
/// reproducible error. These tests hit real provider endpoints, so a single
/// hiccup should be retried, not reported as a regression — e.g. the observed
/// flake `LLM error: Stream error: Transport error: error decoding response body`.
///
/// `overloaded` and `service unavailable` are matched explicitly: a provider
/// capacity rejection (Anthropic `overloaded_error` / HTTP 529, or a 503) is
/// transient in exactly the same way, and is the condition that turned main CI
/// red on `abda5cc`. It previously matched only by accident, because Anthropic
/// wraps the payload in the string `Anthropic stream error: …` — a driver that
/// reported the same rejection without the words "stream error" would have been
/// treated as a hard regression. Bare status codes are deliberately *not*
/// matched: `503`/`529` as substrings collide with token counts and request ids.
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
        "overloaded",
        "service unavailable",
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

/// Enforce live tool-call contracts after the caller exhausts its sampling retries.
/// A clean sampling miss is still useful diagnostic evidence, but cannot prove
/// that the provider serialized the tool definition onto the wire.
pub fn assert_live_tool_call_contract(result: &TurnResult, expected_tool: &str, label: &str) {
    let outcome = classify_live_tool_call(result, expected_tool);
    match outcome {
        LiveToolCallOutcome::Exercised => {}
        LiveToolCallOutcome::SamplingMiss => panic!(
            "TOOL CONTRACT FAILURE: {label} did not call {expected_tool:?} after sampling retries; \
             the in-memory advertised-tool summary is not wire-level request evidence; \
             generations={:?}",
            result.llm_generations,
        ),
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
/// A transient failure backs off before the next attempt (5s, then 10s);
/// a sampling miss retries immediately. Without this the three attempts were
/// issued back to back over a couple of seconds, so a provider overload lasting
/// tens of seconds — the common shape — exhausted every attempt while still
/// overloaded and failed the matrix. Only the transient path pays the delay,
/// because re-sampling a model that cleanly declined a tool has nothing to wait
/// for.
///
/// Exported via `#[macro_export]` so every test binary that includes this
/// shared module can use it. `is_quota_exhausted`, `is_transient_transport_error`,
/// `live_retry_backoff`, and `TurnResult` are referenced unqualified and resolve
/// at each call site (the test files already glob-import this module and
/// `TurnResult`).
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
                        // The cell reached the provider but never got to assert
                        // anything (EVE-951): record it as unverified, not as run.
                        $config.record_quota();
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
            if transient && attempt < $max {
                let backoff = live_retry_backoff(attempt);
                eprintln!(
                    "{}: backing off {:?} before attempt {}/{}",
                    $config.label(),
                    backoff,
                    attempt + 1,
                    $max,
                );
                ::tokio::time::sleep(backoff).await;
            }
        }
        outcome
    }};
}

/// Delay before retrying a live turn that failed transiently, after `attempt`
/// (1-based) attempts. Exponential from a 5s base and capped, so a provider
/// overload gets tens of seconds to clear without unbounding the job.
///
/// Free function rather than inline arithmetic so the schedule is unit-testable
/// without issuing live provider traffic.
pub fn live_retry_backoff(attempt: u32) -> std::time::Duration {
    const BASE_SECS: u64 = 5;
    const MAX_SECS: u64 = 30;
    let exponent = attempt.saturating_sub(1).min(8);
    std::time::Duration::from_secs((BASE_SECS << exponent).min(MAX_SECS))
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
    use super::{
        LiveToolCallOutcome, assert_live_tool_call_contract, classify_live_tool_call,
        is_quota_exhausted, is_transient_transport_error, live_retry_backoff,
    };
    use everruns_core::turn::TurnStopReason;
    use everruns_provider::typed_id::TurnId;
    use everruns_test_support::in_memory_loop::{LlmGenerationSummary, TurnResult};

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
        let config = super::ANTHROPIC_OPUS5;
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
        let config = super::ANTHROPIC_OPUS5;
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
        let config = super::ANTHROPIC_OPUS5;
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
}
