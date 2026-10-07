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

use everruns_test_support::{TestMathCapability, TestWeatherCapability};
use llm_test_matrix::*;

use everruns_capabilities::capabilities::SessionCapability;
use everruns_core::builtins::{
    AutoToolSearchCapability, ClaudeToolSearchCapability, CurrentTimeCapability,
    OpenAiToolSearchCapability, StatelessTodoListCapability, TOOL_SEARCH_TOOL_NAME,
    ToolSearchCapability,
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

const TIME_SYSTEM_PROMPT: &str =
    "You are a helpful assistant. When asked about time, use the get_current_time tool.";

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
        let Some((runner, result)) =
            live_turn(&config, "What time is it right now?", |model| async move {
                InMemoryAgenticLoop::builder()
                    .agent_name("Tool Search Agent")
                    .system_prompt(TIME_SYSTEM_PROMPT)
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
        let Some((runner, result)) =
            live_turn(&config, "What time is it right now?", |model| async move {
                InMemoryAgenticLoop::builder()
                    .agent_name("Auto Tool Search Agent")
                    .system_prompt(TIME_SYSTEM_PROMPT)
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
    // A turn that calls no tool at all is model sampling (nightly 2026-10-06 saw
    // Opus 5 answer without one), so it is re-sampled. A call to any other tool
    // name is the regression this test exists for (`bash_run`) and never retries.
    const MAX_ATTEMPTS: usize = 3;
    let mut tool_names: Vec<String> = Vec::new();
    let mut bash_calls = Vec::new();

    for attempt in 1..=MAX_ATTEMPTS {
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

        let calls: Vec<_> = runner
            .events_by_type(LLM_GENERATION)
            .await
            .into_iter()
            .filter_map(|event| match event.data {
                EventData::LlmGeneration(data) => Some(data.output.tool_calls),
                _ => None,
            })
            .flatten()
            .collect();
        tool_names = calls.iter().map(|call| call.name.clone()).collect();
        bash_calls = calls
            .into_iter()
            .filter(|call| call.name == "bash")
            .collect();
        if !bash_calls.is_empty() || !tool_names.is_empty() {
            break;
        }
        eprintln!("{config}: attempt {attempt}/{MAX_ATTEMPTS}: no tool call; re-sampling");
    }

    assert!(
        !bash_calls.is_empty(),
        "{config} should call the `bash` tool within {MAX_ATTEMPTS} attempts; \
         tool calls seen: {tool_names:?}"
    );
    assert!(bash_calls.iter().all(|call| {
        call.arguments.get("commands").is_some() && call.arguments.get("command").is_none()
    }));
}

/// Tests that tool_search gracefully works when below threshold
/// (falls back to standard tool format — no namespaces, no defer_loading)
#[tokio::test]
async fn test_gpt54_tool_search_below_threshold_fallback() {
    let Some((_, result)) = live_turn(&OPENAI_GPT54, "What time is it?", |model| async move {
        InMemoryAgenticLoop::builder()
            .agent_name("Below Threshold Agent")
            .system_prompt("When asked about time, use the get_current_time tool.")
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
