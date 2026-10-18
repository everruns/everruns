#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Integration tests for tool_search (deferred tool loading).
//
// Tests the full pipeline for provider-backed and generic client-side tool_search
// capability wiring through RuntimeAgent and LlmCallConfig.
//
// Includes real GPT-5.4, GPT-5.5, and GPT-5.6 Terra integration tests that
// exercise hosted tool_search end-to-end against the OpenAI API.
//
// Run all:
//   cargo test -p everruns-llm-tests --test tool_search_test --features llm-tests
//
// CI: Live Provider Matrix (`live-provider-matrix` in ci.yml) runs this target
// under `doppler run` on push when provider_live paths change.
//
// Required env vars (tests skip gracefully if missing):
//   OPENAI_API_KEY / ANTHROPIC_API_KEY (per case)
//
// Every live turn goes through `live_turn` (built on `run_live_turn!`): out-of-quota and
// retired-model cells skip and are reported as unverified, and transient transport/overload
// errors retry on a fresh runner, matching the other live-matrix targets.
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

#[path = "tool_search_test/gemini_tool_removal_wire.rs"]
mod gemini_tool_removal_wire;

use everruns_test_support::{TestMathCapability, TestWeatherCapability};
use llm_test_matrix::*;

use everruns_capabilities::capabilities::SessionCapability;
use everruns_core::builtins::{
    AutoToolSearchCapability, ClaudeToolSearchCapability, CurrentTimeCapability,
    OpenAiToolSearchCapability, PromptCachingCapability, StatelessTodoListCapability,
    TOOL_SEARCH_TOOL_NAME, ToolSearchCapability,
};
use everruns_core::events::{EventData, LLM_GENERATION};
use everruns_integrations::bashkit::BashkitShellCapability;
use everruns_integrations::filesystem::FileSystemCapability;
use everruns_test_support::in_memory_loop::{InMemoryAgenticLoop, InMemoryModelConfig, TurnResult};
use rstest::rstest;

async fn assert_hosted_tool_search_was_enabled(runner: &InMemoryAgenticLoop) {
    let generations = runner.events_by_type(LLM_GENERATION).await;
    assert!(
        !generations.is_empty(),
        "expected at least one llm.generation event"
    );
    assert!(
        generations.iter().any(|event| {
            let EventData::LlmGeneration(data) = &event.data else {
                return false;
            };
            data.metadata
                .request_options
                .as_ref()
                .and_then(|options| options.tool_search.as_ref())
                .is_some_and(|tool_search| tool_search.enabled)
        }),
        "expected an llm.generation event with hosted tool_search enabled"
    );
}

async fn generation_called_tool(runner: &InMemoryAgenticLoop, name: &str) -> bool {
    runner
        .events_by_type(LLM_GENERATION)
        .await
        .iter()
        .any(|event| {
            let EventData::LlmGeneration(data) = &event.data else {
                return false;
            };
            data.output.tool_calls.iter().any(|call| call.name == name)
        })
}

/// Attempts per live turn when the failure is a transient transport/overload
/// error (e.g. `error decoding response body`), which failed main CI on a
/// single Opus 5 hiccup. Separate from each test's own sampling-retry loop.
const TRANSPORT_ATTEMPTS: u32 = 3;

/// Runs one live turn for `config` on a runner from `build`.
///
/// Returns `None` when the test should skip: the cell has no credentials, is
/// out of quota/credits, or its model was retired (the latter two are recorded
/// as unverified coverage by `run_live_turn!`). Transient transport errors are
/// retried with backoff; each attempt builds a fresh runner so a retry never
/// inherits the failed attempt's session history or events. Any other failure
/// is returned as-is for the caller's `result.success` assertion to report.
async fn live_turn<B, F>(
    config: &ProviderModelConfig,
    prompt: &str,
    build: B,
) -> Option<(InMemoryAgenticLoop, TurnResult)>
where
    B: Fn(InMemoryModelConfig) -> F,
    F: std::future::Future<Output = InMemoryAgenticLoop>,
{
    if config.model().is_none() {
        eprintln!("Skipping: {} not set", config.label());
        return None;
    }
    let mut runner = None;
    let result = run_live_turn!(
        config,
        TRANSPORT_ATTEMPTS,
        |r: &TurnResult| r.success || !r.error.as_deref().is_some_and(is_transient_transport_error),
        {
            let model = config.model().expect("checked above");
            let built = build(model).await;
            let result = built.run_turn(prompt).await.unwrap();
            runner = Some(built);
            result
        }
    )?;
    Some((runner.expect("at least one attempt ran"), result))
}

// ============================================================================
// Scenario: hosted OpenAI tool_search (deferred loading)
// ============================================================================

/// Tests tool_search end-to-end with GPT-5.4 and GPT-5.6 Terra:
/// - Adds enough capabilities to exceed the threshold (16 tools > 15)
/// - Adds OpenAiToolSearchCapability
/// - Verifies the model can still call tools correctly with deferred schemas
///
/// Both models must discover tool purposes hidden behind generic categories.
#[rstest]
#[case::gpt54(OPENAI_GPT54)]
#[case::gpt56_terra(OPENAI_GPT56_TERRA)]
#[tokio::test]
async fn test_openai_tool_search_with_many_capabilities(#[case] config: ProviderModelConfig) {
    const MAX_ATTEMPTS: usize = 5;
    let mut called_get_current_time = false;

    for attempt in 1..=MAX_ATTEMPTS {
        let Some((runner, result)) = live_turn(&config, TIME_TOOL_QUESTION, |model| async move {
            InMemoryAgenticLoop::builder()
                .agent_name("Tool Search Agent")
                .system_prompt(TIME_TOOL_SYSTEM_PROMPT)
                .model(model)
                .driver_registry(all_providers_registry())
                // Add multiple capabilities to exceed the 15-tool threshold (16 total)
                .capability(CurrentTimeCapability) // 1 tool
                .capability(TestMathCapability) // 4 tools
                .capability(TestWeatherCapability) // 2 tools
                .capability(FileSystemCapability) // 6 tools
                .capability(SessionCapability) // 2 tools
                .capability(StatelessTodoListCapability) // 1 tool
                // Enable tool_search
                .capability(OpenAiToolSearchCapability::new())
                .max_iterations(5)
                .build()
                .await
                .unwrap()
        })
        .await
        else {
            return;
        };
        assert!(result.success, "Turn should succeed: {:?}", result.error);
        if result.tool_calls_count > 0 && generation_called_tool(&runner, "get_current_time").await
        {
            called_get_current_time = true;
            break;
        }
        eprintln!("attempt {attempt}/{MAX_ATTEMPTS}: no tool call yet; retrying");
    }

    assert!(
        called_get_current_time,
        "Model should call get_current_time even with deferred tool loading \
         within {MAX_ATTEMPTS} attempts"
    );
}

/// Tests tool_search with a lower custom threshold so it activates with few tools.
///
/// GPT-5.4 remains covered alongside the production GPT-5.6 Terra model.
#[rstest]
#[case::gpt54(OPENAI_GPT54)]
#[case::gpt56_terra(OPENAI_GPT56_TERRA)]
#[tokio::test]
async fn test_openai_tool_search_low_threshold(#[case] config: ProviderModelConfig) {
    const MAX_ATTEMPTS: usize = 5;
    let mut called_add = false;

    for attempt in 1..=MAX_ATTEMPTS {
        let Some((runner, result)) = live_turn(&config, "What is 7 + 3?", |model| async move {
            InMemoryAgenticLoop::builder()
                .agent_name("Low Threshold Agent")
                .system_prompt("When asked to add numbers, use the add tool.")
                .model(model)
                .driver_registry(all_providers_registry())
                .capability(TestMathCapability)
                .capability(CurrentTimeCapability)
                // Low threshold: tool_search activates even with few tools (5 > 3)
                .capability(OpenAiToolSearchCapability::with_threshold(3))
                .max_iterations(5)
                .build()
                .await
                .unwrap()
        })
        .await
        else {
            return;
        };
        assert!(result.success, "Turn should succeed: {:?}", result.error);
        assert_hosted_tool_search_was_enabled(&runner).await;
        if result.tool_calls_count > 0 && generation_called_tool(&runner, "add").await {
            called_add = true;
            break;
        }
        eprintln!("attempt {attempt}/{MAX_ATTEMPTS}: no add call yet; retrying");
    }

    assert!(
        called_add,
        "Model should call add tool even with deferred schemas within {MAX_ATTEMPTS} attempts"
    );
}

/// EVE-1164: hosted search over the filesystem category must complete a
/// `list_directory` call. Before the fix, OpenAI returned `server_error` when
/// the wire namespace was the human label `File Operations` (whitespace).
#[tokio::test]
async fn test_gpt55_tool_search_file_operations_namespace() {
    const MAX_ATTEMPTS: usize = 3;
    let mut called_list_directory = false;

    for attempt in 1..=MAX_ATTEMPTS {
        let Some((runner, result)) = live_turn(
            &OPENAI_GPT55,
            "List the files in the current directory using list_directory.",
            |model| async move {
                InMemoryAgenticLoop::builder()
                    .agent_name("File Operations Namespace Agent")
                    .system_prompt(
                        "You have file tools. When asked to list a directory, call list_directory \
                         with path \".\". Do not answer without calling the tool.",
                    )
                    .model(model)
                    .driver_registry(all_providers_registry())
                    .capability(FileSystemCapability)
                    // Force hosted search with a single namespace derived from the
                    // filesystem category (`File Operations` → `File_Operations`).
                    .capability(OpenAiToolSearchCapability::with_threshold(1))
                    .max_iterations(5)
                    .build()
                    .await
                    .unwrap()
            },
        )
        .await
        else {
            return;
        };
        assert!(
            result.success,
            "hosted search over File Operations must not fail: {:?}",
            result.error
        );
        assert_hosted_tool_search_was_enabled(&runner).await;

        let generations = runner.events_by_type(LLM_GENERATION).await;
        for event in &generations {
            let EventData::LlmGeneration(data) = &event.data else {
                continue;
            };
            if data
                .output
                .tool_calls
                .iter()
                .any(|call| call.name == "list_directory")
            {
                called_list_directory = true;
            }
        }

        if called_list_directory {
            break;
        }
        eprintln!("attempt {attempt}/{MAX_ATTEMPTS}: no list_directory call yet; retrying");
    }

    assert!(
        called_list_directory,
        "hosted tool search must load the File Operations namespace and emit list_directory"
    );
}

/// Tests hosted tool_search with GPT-5.5 and a lower custom threshold.
///
/// This covers the current default OpenAI model family against the real API and
/// verifies both halves of the contract: hosted tool_search is present on the
/// request, and the deferred tool can still be called and completed by the loop.
#[tokio::test]
async fn test_gpt55_tool_search_low_threshold() {
    let Some((runner, result)) = live_turn(&OPENAI_GPT55, "What is 7 + 3?", |model| async move {
        InMemoryAgenticLoop::builder()
            .agent_name("GPT-5.5 Tool Search Agent")
            .system_prompt("When asked to add numbers, use the add tool.")
            .model(model)
            .driver_registry(all_providers_registry())
            .capability(TestMathCapability)
            .capability(CurrentTimeCapability)
            // Low threshold: tool_search activates even with few tools (5 > 3).
            .capability(OpenAiToolSearchCapability::with_threshold(3))
            .max_iterations(5)
            .build()
            .await
            .unwrap()
    })
    .await
    else {
        return;
    };
    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "GPT-5.5 should call add tool even with deferred schemas"
    );
    assert_hosted_tool_search_was_enabled(&runner).await;
}

/// Tests the model-adaptive `auto_tool_search` capability end-to-end on GPT-5.4.
///
/// On a native model, `auto_tool_search` must resolve (at capability-collection
/// time, via `Capability::resolve_for_model`) to the hosted OpenAI mechanism —
/// not the client-side fallback. The test checks two things from the emitted
/// `llm.generation` events:
///
/// 1. **Hosted was selected (deterministic).** The hosted mechanism offers the
///    real (deferred) tools and adds *no* client-side `tool_search` tool, whereas
///    the generic fallback *would* add one. So the absence of a `tool_search`
///    tool in the model's tool list proves hosted resolution — independent of
///    whether the model chose to call a tool on a given turn.
/// 2. **The hosted round-trip executes (live).** The model actually calls
///    `get_current_time` (deferred load → server-side search → tool call).
///
/// Supported models occasionally answer "what time is it" from priors without
/// calling a tool, so the round-trip half is retried; the hosted-resolution
/// half is asserted on every attempt's generation events.
///
/// GPT-5.4 remains covered alongside the production GPT-5.6 Terra model.
#[rstest]
#[case::gpt54(OPENAI_GPT54)]
#[case::gpt56_terra(OPENAI_GPT56_TERRA)]
#[tokio::test]
async fn test_openai_auto_tool_search_resolves_to_hosted(#[case] config: ProviderModelConfig) {
    const MAX_ATTEMPTS: usize = 5;
    let mut called_get_current_time = false;

    for attempt in 1..=MAX_ATTEMPTS {
        let Some((runner, result)) = live_turn(&config, TIME_TOOL_QUESTION, |model| async move {
            InMemoryAgenticLoop::builder()
                .agent_name("Auto Tool Search Agent")
                .system_prompt(TIME_TOOL_SYSTEM_PROMPT)
                .model(model)
                .driver_registry(all_providers_registry())
                // Multiple capabilities to exceed the 15-tool threshold (16 total).
                .capability(CurrentTimeCapability) // 1 tool
                .capability(TestMathCapability) // 4 tools
                .capability(TestWeatherCapability) // 2 tools
                .capability(FileSystemCapability) // 6 tools
                .capability(SessionCapability) // 2 tools
                .capability(StatelessTodoListCapability) // 1 tool
                // Model-adaptive: on this OpenAI model this must resolve to hosted.
                .capability(AutoToolSearchCapability::new())
                .max_iterations(5)
                .build()
                .await
                .unwrap()
        })
        .await
        else {
            return;
        };
        assert!(result.success, "Turn should succeed: {:?}", result.error);

        let generations = runner.events_by_type(LLM_GENERATION).await;
        assert!(
            !generations.is_empty(),
            "expected at least one llm.generation event"
        );
        assert_hosted_tool_search_was_enabled(&runner).await;
        for event in &generations {
            let EventData::LlmGeneration(data) = &event.data else {
                continue;
            };
            // (1) Hosted resolution: the client-side `tool_search` tool must not
            // be offered to the model. Its presence would mean auto_tool_search
            // fell back to the generic mechanism.
            assert!(
                !data.tools.iter().any(|t| t.name == TOOL_SEARCH_TOOL_NAME),
                "auto_tool_search on this OpenAI model must resolve to hosted: the client-side \
                 `{TOOL_SEARCH_TOOL_NAME}` tool must not be offered to the model"
            );
            // (2) Round-trip: the model executed the deferred get_current_time tool.
            if data
                .output
                .tool_calls
                .iter()
                .any(|call| call.name == "get_current_time")
            {
                called_get_current_time = true;
            }
        }

        if called_get_current_time {
            break;
        }
        eprintln!("attempt {attempt}/{MAX_ATTEMPTS}: no get_current_time call yet; retrying");
    }

    assert!(
        called_get_current_time,
        "auto_tool_search → hosted deferred loading should drive a get_current_time \
         call within {MAX_ATTEMPTS} attempts"
    );
}

/// Tests model-adaptive hosted resolution on GPT-5.5.
#[tokio::test]
async fn test_gpt55_auto_tool_search_resolves_to_hosted() {
    let Some((runner, result)) = live_turn(&OPENAI_GPT55, "What is 7 + 3?", |model| async move {
        InMemoryAgenticLoop::builder()
            .agent_name("GPT-5.5 Auto Tool Search Agent")
            .system_prompt("When asked to add numbers, use the add tool.")
            .model(model)
            .driver_registry(all_providers_registry())
            .capability(TestMathCapability)
            .capability(CurrentTimeCapability)
            .capability(AutoToolSearchCapability::with_threshold(3))
            .max_iterations(5)
            .build()
            .await
            .unwrap()
    })
    .await
    else {
        return;
    };
    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "GPT-5.5 auto_tool_search should call add through hosted deferred loading"
    );
    assert_hosted_tool_search_was_enabled(&runner).await;

    let generations = runner.events_by_type(LLM_GENERATION).await;
    for event in &generations {
        let EventData::LlmGeneration(data) = &event.data else {
            continue;
        };
        assert!(
            !data.tools.iter().any(|t| t.name == TOOL_SEARCH_TOOL_NAME),
            "auto_tool_search on GPT-5.5 must resolve to hosted: the client-side \
             `{TOOL_SEARCH_TOOL_NAME}` tool must not be offered to the model"
        );
    }
}

// ============================================================================
// Scenario: generic (provider-agnostic) tool_search with a non-GPT model
// ============================================================================

/// Tests the generic `tool_search` capability end-to-end with Anthropic Haiku
/// (no native tool_search). Schemas are deferred client-side and the model
/// loads them via the `tool_search` tool before calling the real tool.
#[tokio::test]
async fn test_anthropic_generic_tool_search() {
    let Some((_, result)) = live_turn(&ANTHROPIC_HAIKU, "What is 21 + 21?", |model| async move {
        InMemoryAgenticLoop::builder()
            .agent_name("Generic Tool Search Agent")
            .system_prompt(
                "You are a helpful assistant. When asked to add numbers, use the add tool.",
            )
            .model(model)
            .driver_registry(all_providers_registry())
            // Many tools to exceed the default threshold (16 total).
            .capability(CurrentTimeCapability) // 1 tool
            .capability(TestMathCapability) // 4 tools
            .capability(TestWeatherCapability) // 2 tools
            .capability(FileSystemCapability) // 6 tools
            .capability(SessionCapability) // 2 tools
            .capability(StatelessTodoListCapability) // 1 tool
            // Generic, provider-agnostic deferred loading (works on Anthropic).
            .capability(ToolSearchCapability::new())
            .max_iterations(6)
            .build()
            .await
            .unwrap()
    })
    .await
    else {
        return;
    };
    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Model should load and call the add tool via generic tool_search"
    );
}

/// Tests generic tool_search with a low threshold so it activates with few tools.
#[tokio::test]
async fn test_anthropic_generic_tool_search_low_threshold() {
    let Some((_, result)) = live_turn(&ANTHROPIC_HAIKU, "What is 7 + 3?", |model| async move {
        InMemoryAgenticLoop::builder()
            .agent_name("Generic Low Threshold Agent")
            .system_prompt("When asked to add numbers, use the add tool.")
            .model(model)
            .driver_registry(all_providers_registry())
            .capability(TestMathCapability)
            .capability(CurrentTimeCapability)
            // Low threshold: deferral activates even with few tools (6 > 3).
            .capability(ToolSearchCapability::with_threshold(3))
            .max_iterations(6)
            .build()
            .await
            .unwrap()
    })
    .await
    else {
        return;
    };
    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Model should call add tool even with deferred schemas"
    );
}

// ============================================================================
// Scenario: hosted Anthropic (Claude) tool_search (deferred loading)
// ============================================================================

/// Tests Anthropic's hosted tool_search end-to-end with Claude Haiku 4.5.
///
/// Verifies both halves of the contract against the live API: the hosted
/// `ToolSearchConfig` is present on the request (deferred-load wire format), and
/// the deferred tool can still be discovered, loaded, and called by the loop.
#[tokio::test]
async fn test_anthropic_claude_tool_search_low_threshold() {
    // Haiku sometimes answers "7 + 3" without the tool; retry fresh sessions
    // like the GPT cases, but check the hosted wire contract on every attempt.
    const MAX_ATTEMPTS: usize = 5;
    let mut called_add = false;

    for attempt in 1..=MAX_ATTEMPTS {
        let Some((runner, result)) =
            live_turn(&ANTHROPIC_HAIKU, "What is 7 + 3?", |model| async move {
                InMemoryAgenticLoop::builder()
                    .agent_name("Claude Tool Search Agent")
                    .system_prompt("When asked to add numbers, use the add tool.")
                    .model(model)
                    .driver_registry(all_providers_registry())
                    .capability(TestMathCapability)
                    .capability(CurrentTimeCapability)
                    // Low threshold: hosted tool_search activates even with few tools (5 > 3).
                    .capability(ClaudeToolSearchCapability::with_threshold(3))
                    .max_iterations(6)
                    .build()
                    .await
                    .unwrap()
            })
            .await
        else {
            return;
        };
        assert!(result.success, "Turn should succeed: {:?}", result.error);
        assert_hosted_tool_search_was_enabled(&runner).await;

        // The hosted path must not also offer the client-side `tool_search` tool —
        // its presence would mean the generic fallback was selected instead.
        let generations = runner.events_by_type(LLM_GENERATION).await;
        for event in &generations {
            let EventData::LlmGeneration(data) = &event.data else {
                continue;
            };
            assert!(
                !data.tools.iter().any(|t| t.name == TOOL_SEARCH_TOOL_NAME),
                "hosted claude_tool_search must not offer the client-side \
                 `{TOOL_SEARCH_TOOL_NAME}` tool"
            );
        }

        if result.tool_calls_count > 0 {
            called_add = true;
            break;
        }
        eprintln!("attempt {attempt}/{MAX_ATTEMPTS}: no tool call yet; retrying");
    }

    assert!(
        called_add,
        "Claude should call the add tool even with deferred schemas \
         within {MAX_ATTEMPTS} attempts"
    );
}

/// Tests model-adaptive hosted resolution on Claude Haiku 4.5 and on Opus 5.5,
/// the recommended Opus.
///
/// On a native Claude model, `auto_tool_search` must resolve (at
/// capability-collection time, via `Capability::resolve_for_model`) to the hosted
/// Anthropic mechanism — not the client-side fallback. The hosted mechanism adds
/// *no* client-side `tool_search` tool, so its absence from the model's tool list
/// proves hosted resolution, independent of whether a tool was called on a turn.
#[rstest]
#[case::anthropic_haiku(ANTHROPIC_HAIKU)]
#[case::anthropic_opus5_5(ANTHROPIC_OPUS55)]
#[tokio::test]
async fn test_anthropic_auto_tool_search_resolves_to_hosted(#[case] config: ProviderModelConfig) {
    let Some((runner, result)) = live_turn(&config, "What is 7 + 3?", |model| async move {
        InMemoryAgenticLoop::builder()
            .agent_name("Claude Auto Tool Search Agent")
            .system_prompt("When asked to add numbers, use the add tool.")
            .model(model)
            .driver_registry(all_providers_registry())
            .capability(TestMathCapability)
            .capability(CurrentTimeCapability)
            // Model-adaptive: on native Claude this must resolve to the hosted mechanism.
            .capability(AutoToolSearchCapability::with_threshold(3))
            .max_iterations(6)
            .build()
            .await
            .unwrap()
    })
    .await
    else {
        return;
    };
    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Claude auto_tool_search should call add through hosted deferred loading"
    );
    assert_hosted_tool_search_was_enabled(&runner).await;

    let generations = runner.events_by_type(LLM_GENERATION).await;
    for event in &generations {
        let EventData::LlmGeneration(data) = &event.data else {
            continue;
        };
        assert!(
            !data.tools.iter().any(|t| t.name == TOOL_SEARCH_TOOL_NAME),
            "auto_tool_search on {config} must resolve to hosted: the client-side \
             `{TOOL_SEARCH_TOOL_NAME}` tool must not be offered to the model"
        );
    }
}

/// Reproduces the production surface where Opus 5 invented `bash_run` after
/// Bash's schema was deferred. Bash is a hot-path exception now, so hosted
/// search remains active for the rest of the tool set while the model always
/// receives Bash's exact `bash` / `commands` contract. Also pinned on Opus 5.5,
/// which replaced Opus 5 as the recommended Opus.
#[rstest]
#[case::anthropic_opus5(ANTHROPIC_OPUS5)]
#[case::anthropic_opus5_5(ANTHROPIC_OPUS55)]
#[tokio::test]
async fn test_anthropic_opus_hosted_search_calls_bash_contract(
    #[case] config: ProviderModelConfig,
) {
    let Some((runner, result)) = live_turn(
        &config,
        "Use Bash to run `printf everruns-bash-contract`.",
        |model| async move {
            InMemoryAgenticLoop::builder()
                .agent_name("Claude Bash Tool Search Agent")
                .system_prompt("Use the bash tool for shell commands.")
                .model(model)
                .driver_registry(all_providers_registry())
                .capability(BashkitShellCapability)
                .capability(FileSystemCapability)
                .capability(TestMathCapability)
                .capability(AutoToolSearchCapability::with_threshold(3))
                .max_iterations(6)
                .build()
                .await
                .unwrap()
        },
    )
    .await
    else {
        return;
    };
    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert_hosted_tool_search_was_enabled(&runner).await;

    let generations = runner.events_by_type(LLM_GENERATION).await;
    let bash_calls: Vec<_> = generations
        .iter()
        .filter_map(|event| {
            let EventData::LlmGeneration(data) = &event.data else {
                return None;
            };
            Some(data.output.tool_calls.iter())
        })
        .flatten()
        .filter(|call| call.name == "bash")
        .collect();

    assert!(
        !bash_calls.is_empty(),
        "{config} should call the `bash` tool"
    );
    assert!(bash_calls.iter().all(|call| {
        call.arguments.get("commands").is_some() && call.arguments.get("command").is_none()
    }));
}

/// Tests that tool_search gracefully works when below threshold
/// (falls back to standard tool format — no namespaces, no defer_loading)
#[tokio::test]
async fn test_gpt54_tool_search_below_threshold_fallback() {
    let Some((_, result)) = live_turn(&OPENAI_GPT54, TIME_TOOL_QUESTION, |model| async move {
        InMemoryAgenticLoop::builder()
            .agent_name("Below Threshold Agent")
            .system_prompt(TIME_TOOL_SYSTEM_PROMPT)
            .model(model)
            .driver_registry(all_providers_registry())
            // Only 1 tool — well below default threshold of 15
            .capability(CurrentTimeCapability)
            .capability(OpenAiToolSearchCapability::new())
            .max_iterations(5)
            .build()
            .await
            .unwrap()
    })
    .await
    else {
        return;
    };
    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Should still work with standard tool format (below threshold)"
    );
}

// ============================================================================
// Scenario: a tool revealed mid-session keeps the prompt cache
// (knowledge/execution/cache-stable-tool-changes.md)
// ============================================================================

/// A system prompt well above every Claude model's minimum cacheable prefix
/// (4096 tokens on Opus and Haiku), so a cache hit on the call after a reveal
/// is unmistakable.
fn long_cacheable_system_prompt() -> String {
    let mut prompt = String::from(
        "You are a helpful assistant. When asked to add numbers, find the add \
         tool with tool_search and call it. The reference notes below are \
         background only; never quote them.\n\n",
    );
    for i in 0..70 {
        prompt.push_str(&format!(
            "Reference note {i:04}: shelf {i} in warehouse W-{:03} holds crate \
             K-{i:04}, inspected on day {} of the audit cycle.\n",
            i % 37,
            i % 29 + 1
        ));
    }
    prompt
}

/// The call right after `tool_search` revealed a tool sends that change inline
/// on models with `inline_tool_changes`, so it still reads the cached prefix
/// (system prompt and tool list) instead of writing it again. Before inline
/// changes, the rewritten `tools` array made this call's cache read zero.
///
/// Fable 5.1 and Opus 4.8 also passed on 2026-10-12; they stay out of the
/// matrix for cost and because Opus 4.8 has no matrix row.
#[rstest]
#[case::anthropic_opus5_5(ANTHROPIC_OPUS55)]
#[case::anthropic_sonnet5_5(ANTHROPIC_SONNET55)]
#[case::anthropic_opus5(ANTHROPIC_OPUS5)]
#[case::anthropic_haiku5_5(ANTHROPIC_HAIKU55)]
#[tokio::test]
async fn test_anthropic_tool_reveal_keeps_prompt_cache(#[case] config: ProviderModelConfig) {
    assert_tool_reveal_keeps_prompt_cache(config).await;
}

/// The OpenAI counterpart: the reveal rides in an `additional_tools` item, so
/// the request after it still reads the cached prefix.
#[rstest]
#[case::openai_gpt6_1_sol(OPENAI_GPT61_SOL)]
#[case::openai_gpt6_luna(OPENAI_GPT6_LUNA)]
#[case::gpt56_terra(OPENAI_GPT56_TERRA)]
#[tokio::test]
async fn test_openai_tool_reveal_keeps_prompt_cache(#[case] config: ProviderModelConfig) {
    assert_tool_reveal_keeps_prompt_cache(config).await;
}

async fn assert_tool_reveal_keeps_prompt_cache(config: ProviderModelConfig) {
    let system_prompt = long_cacheable_system_prompt();
    let Some((runner, result)) = live_turn(&config, "What is 21 + 21?", |model| {
        let system_prompt = system_prompt.clone();
        async move {
            InMemoryAgenticLoop::builder()
                .agent_name("Tool Reveal Cache Agent")
                .system_prompt(system_prompt)
                .model(model)
                .driver_registry(all_providers_registry())
                .capability(TestMathCapability)
                .capability(CurrentTimeCapability)
                .capability(ToolSearchCapability::with_threshold(3))
                .capability(PromptCachingCapability::new())
                .max_iterations(6)
                .build()
                .await
                .unwrap()
        }
    })
    .await
    else {
        return;
    };
    assert!(result.success, "Turn should succeed: {:?}", result.error);

    let generations: Vec<_> = runner
        .events_by_type(LLM_GENERATION)
        .await
        .into_iter()
        .filter_map(|event| match event.data {
            EventData::LlmGeneration(data) => Some(data),
            _ => None,
        })
        .collect();
    for (index, data) in generations.iter().enumerate() {
        let usage = data.metadata.usage.as_ref();
        eprintln!(
            "{} call {index}: input={:?} cache_read={:?} cache_write={:?} tool_changes={:?}",
            config.label(),
            usage.map(|u| u.input_tokens),
            usage.and_then(|u| u.cache_read_tokens),
            usage.and_then(|u| u.cache_creation_tokens),
            data.metadata.tool_changes.as_ref().map(|c| (
                c.baseline.as_ref().map(Vec::len),
                c.added.iter().map(|t| t.name()).collect::<Vec<_>>(),
                c.redefined.iter().map(|t| t.name()).collect::<Vec<_>>(),
            )),
        );
    }

    // Skip the first call: it starts the baseline. Any later call that
    // carries a reveal is the one whose cache the old layout lost.
    let after_reveal = generations
        .iter()
        .skip(1)
        .find(|data| {
            data.metadata
                .tool_changes
                .as_ref()
                .is_some_and(|c| !c.added.is_empty() || !c.redefined.is_empty())
        })
        .expect("a call after tool_search should carry the revealed tool");
    let cache_read = after_reveal
        .metadata
        .usage
        .as_ref()
        .and_then(|usage| usage.cache_read_tokens)
        .unwrap_or(0);
    assert!(
        cache_read >= 4096,
        "{}: the call after the reveal should read the cached prefix, read {cache_read}",
        config.label()
    );
    assert!(
        generation_called_tool(&runner, "add").await,
        "{}: the model should call the revealed add tool",
        config.label()
    );
}

/// A capability whose how-to appears only once it is switched on, as a
/// capability enabled mid-session does.
struct SwitchedHowToCapability(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[async_trait::async_trait]
impl everruns_core::capabilities::Capability for SwitchedHowToCapability {
    fn id(&self) -> &str {
        "test_switched_how_to"
    }

    fn name(&self) -> &str {
        "Switched how-to"
    }

    fn description(&self) -> &str {
        "Testing capability: adds its how-to to the prompt once switched on."
    }

    async fn system_prompt_contribution(
        &self,
        _ctx: &everruns_core::capabilities::SystemPromptContext,
    ) -> Option<String> {
        self.0
            .load(std::sync::atomic::Ordering::SeqCst)
            .then(|| "Deploy word: when asked for the deploy word, answer PELICAN.".to_string())
    }
}

/// Long reference text contributed by a capability listed after the switched
/// one, so the switched how-to lands in the middle of the system prompt: sent
/// as part of the prompt, it would rewrite everything after it.
struct ReferenceNotesCapability;

#[async_trait::async_trait]
impl everruns_core::capabilities::Capability for ReferenceNotesCapability {
    fn id(&self) -> &str {
        "test_reference_notes"
    }

    fn name(&self) -> &str {
        "Reference notes"
    }

    fn description(&self) -> &str {
        "Testing capability: long static reference text."
    }

    async fn system_prompt_contribution(
        &self,
        _ctx: &everruns_core::capabilities::SystemPromptContext,
    ) -> Option<String> {
        Some(long_cacheable_system_prompt())
    }
}

/// Prompt text a capability adds mid-session rides in the conversation, so the
/// next call still reads the cached prefix (tools and the epoch's system
/// prompt) and the model still follows the new text. The added text sits
/// before the long reference notes, so without the note the call after the
/// switch would read only the short part ahead of it.
#[rstest]
#[case::anthropic_sonnet5_5(ANTHROPIC_SONNET55)]
#[case::openai_gpt6_luna(OPENAI_GPT6_LUNA)]
#[tokio::test]
async fn test_prompt_text_added_mid_session_keeps_prompt_cache(
    #[case] config: ProviderModelConfig,
) {
    let switched = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let Some((runner, first)) = live_turn(&config, "What is 21 + 21?", |model| {
        let system_prompt = "You are a helpful assistant.".to_string();
        switched.store(false, std::sync::atomic::Ordering::SeqCst);
        let capability = SwitchedHowToCapability(switched.clone());
        async move {
            InMemoryAgenticLoop::builder()
                .agent_name("Prompt Note Cache Agent")
                .system_prompt(system_prompt)
                .model(model)
                .driver_registry(all_providers_registry())
                .capability(TestMathCapability)
                .capability(capability)
                .capability(ReferenceNotesCapability)
                .capability(PromptCachingCapability::new())
                .max_iterations(4)
                .build()
                .await
                .unwrap()
        }
    })
    .await
    else {
        return;
    };
    assert!(first.success, "Turn should succeed: {:?}", first.error);

    switched.store(true, std::sync::atomic::Ordering::SeqCst);
    let second = runner
        .run_turn("What is the deploy word? Answer with the word only.")
        .await
        .unwrap();
    assert!(second.success, "Turn should succeed: {:?}", second.error);

    let generations: Vec<_> = runner
        .events_by_type(LLM_GENERATION)
        .await
        .into_iter()
        .filter_map(|event| match event.data {
            EventData::LlmGeneration(data) => Some(data),
            _ => None,
        })
        .collect();
    let noted = generations
        .iter()
        .find(|data| {
            data.metadata
                .tool_changes
                .as_ref()
                .is_some_and(|c| c.note.is_some())
        })
        .expect("the call after the switch should record the how-to as a note");
    let cache_read = noted
        .metadata
        .usage
        .as_ref()
        .and_then(|usage| usage.cache_read_tokens)
        .unwrap_or(0);
    // Everything the previous call sent is the prefix this one can read.
    let previous_input = generations
        .iter()
        .take_while(|data| !std::ptr::eq(*data, noted))
        .last()
        .and_then(|data| data.metadata.usage.as_ref())
        .map(|usage| {
            usage.input_tokens
                + usage.cache_read_tokens.unwrap_or(0)
                + usage.cache_creation_tokens.unwrap_or(0)
        })
        .unwrap_or(0);
    eprintln!(
        "{} cache_read after the switch: {cache_read} of a {previous_input}-token prefix",
        config.label()
    );
    assert!(
        previous_input >= 2048 && cache_read * 10 >= previous_input * 9,
        "{}: the call after the switch should read the cached prefix, read {cache_read} of {previous_input}",
        config.label()
    );
    assert!(
        second.response.to_uppercase().contains("PELICAN"),
        "{}: the model should follow the added how-to, said {:?}",
        config.label(),
        second.response
    );
}

/// Weather tools while switched on, none after, as a capability switched off
/// mid-session would.
struct SwitchedWeatherCapability(std::sync::Arc<std::sync::atomic::AtomicBool>);

#[async_trait::async_trait]
impl everruns_core::capabilities::Capability for SwitchedWeatherCapability {
    fn id(&self) -> &str {
        "test_switched_weather"
    }

    fn name(&self) -> &str {
        "Switched weather"
    }

    fn description(&self) -> &str {
        "Testing capability: weather tools until switched off."
    }

    fn tools(&self) -> Vec<Box<dyn everruns_core::tools::Tool>> {
        if self.0.load(std::sync::atomic::Ordering::SeqCst) {
            everruns_core::capabilities::Capability::tools(&TestWeatherCapability)
        } else {
            Vec::new()
        }
    }
}

/// On models that rewrite `tools` for every change, a removal waits while the
/// cache is warm: the call after the switch sends the same tools, records no
/// change, and remains functional. Cache reads are measured separately from
/// the deterministic Gemini wire/removed-tool contract in the loopback test.
#[rstest]
#[case::openrouter_gpt6_luna(OPENROUTER_GPT6_LUNA)]
#[case::gemini_flash(GEMINI_FLASH)]
#[tokio::test]
async fn test_tool_removal_waits_for_a_cold_cache(#[case] config: ProviderModelConfig) {
    let on = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let Some((runner, first)) = live_turn(&config, "What is 21 + 21?", |model| {
        on.store(true, std::sync::atomic::Ordering::SeqCst);
        let capability = SwitchedWeatherCapability(on.clone());
        async move {
            InMemoryAgenticLoop::builder()
                .agent_name("Removal Cache Agent")
                .system_prompt("You are a helpful assistant.".to_string())
                .model(model)
                .driver_registry(all_providers_registry())
                .capability(TestMathCapability)
                .capability(capability)
                .capability(ReferenceNotesCapability)
                .capability(PromptCachingCapability::new())
                .max_iterations(4)
                .build()
                .await
                .unwrap()
        }
    })
    .await
    else {
        return;
    };
    assert!(first.success, "Turn should succeed: {:?}", first.error);
    let generations = |events: Vec<everruns_core::events::Event>| -> Vec<_> {
        events
            .into_iter()
            .filter_map(|event| match event.data {
                EventData::LlmGeneration(data) => Some(data),
                _ => None,
            })
            .collect()
    };
    let before = generations(runner.events_by_type(LLM_GENERATION).await);

    on.store(false, std::sync::atomic::Ordering::SeqCst);
    let second = runner
        .run_turn("What is 2 + 3? Answer with the number only.")
        .await
        .unwrap();
    assert!(second.success, "Turn should succeed: {:?}", second.error);

    assert_eq!(
        second.response.trim(),
        "5",
        "{}: wrong answer",
        config.label()
    );

    let all = generations(runner.events_by_type(LLM_GENERATION).await);
    let after = &all[before.len()];
    assert!(
        after.metadata.tool_changes.is_none(),
        "{}: the removal should wait, recorded {:?}",
        config.label(),
        after.metadata.tool_changes
    );
    let cache_read = after
        .metadata
        .usage
        .as_ref()
        .and_then(|usage| usage.cache_read_tokens);
    let previous_input = before
        .last()
        .and_then(|data| data.metadata.usage.as_ref())
        .map(|usage| {
            usage.input_tokens
                + usage.cache_read_tokens.unwrap_or(0)
                + usage.cache_creation_tokens.unwrap_or(0)
        })
        .unwrap_or(0);
    eprintln!(
        "{} cache_read after the removal: {cache_read:?} of a {previous_input}-token prefix",
        config.label()
    );
    assert!(
        previous_input >= 2048,
        "{}: expected a cache-eligible prefix, got {previous_input} tokens",
        config.label()
    );
    // Gemini implicit caching is best effort, even for byte-identical requests:
    // https://ai.google.dev/gemini-api/docs/generate-content/caching
    // The loopback test gates our prefix and execution invariants instead.
    if config.provider_type != everruns_contracts::driver_registry::DriverId::Gemini {
        let cache_read = cache_read.unwrap_or(0);
        assert!(
            cache_read * 10 >= previous_input * 8,
            "{}: the call after the removal should read the cached prefix, read {cache_read} of {previous_input}",
            config.label()
        );
    }
}
