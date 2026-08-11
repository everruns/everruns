// Integration tests for tool_search (deferred tool loading).
//
// Tests the full pipeline for provider-backed and generic client-side tool_search
// capability wiring through RuntimeAgent and LlmCallConfig.
//
// Includes real GPT-5.4 and GPT-5.5 integration tests that exercise hosted
// tool_search end-to-end against the OpenAI API.
//
// Run all:
//   cargo test -p everruns-llm-tests --test tool_search_test --features llm-tests
//
// Required env vars (tests skip gracefully if missing):
//   OPENAI_API_KEY
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

use everruns_test_support::{TestMathCapability, TestWeatherCapability};
use llm_test_matrix::*;

use everruns_builtins::{
    AutoToolSearchCapability, ClaudeToolSearchCapability, CurrentTimeCapability,
    OpenAiToolSearchCapability, StatelessTodoListCapability, TOOL_SEARCH_TOOL_NAME,
    ToolSearchCapability,
};
use everruns_core::capabilities::SessionCapability;
use everruns_core::events::{EventData, LLM_GENERATION};
use everruns_integrations_bashkit::BashkitShellCapability;
use everruns_integrations_filesystem::FileSystemCapability;
use everruns_test_support::in_memory_loop::InMemoryAgenticLoop;

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

// ============================================================================
// Scenario: hosted OpenAI tool_search (deferred loading)
// ============================================================================

/// Tests tool_search end-to-end with GPT-5.4:
/// - Adds enough capabilities to exceed the threshold (16 tools > 15)
/// - Adds OpenAiToolSearchCapability
/// - Verifies the model can still call tools correctly with deferred schemas
#[tokio::test]
async fn test_gpt54_tool_search_with_many_capabilities() {
    let Some(model) = OPENAI_GPT54.model() else {
        eprintln!("Skipping: {} not set", OPENAI_GPT54.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Tool Search Agent")
        .system_prompt(
            "You are a helpful assistant. When asked about time, use the get_current_time tool.",
        )
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
        .unwrap();

    let result = runner.run_turn("What time is it right now?").await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Model should call get_current_time even with deferred tool loading"
    );
}

/// Tests tool_search with a lower custom threshold so it activates with few tools
#[tokio::test]
async fn test_gpt54_tool_search_low_threshold() {
    let Some(model) = OPENAI_GPT54.model() else {
        eprintln!("Skipping: {} not set", OPENAI_GPT54.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
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
        .unwrap();

    let result = runner.run_turn("What is 7 + 3?").await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Model should call add tool even with deferred schemas"
    );
    assert_hosted_tool_search_was_enabled(&runner).await;
}

/// Tests hosted tool_search with GPT-5.5 and a lower custom threshold.
///
/// This covers the current default OpenAI model family against the real API and
/// verifies both halves of the contract: hosted tool_search is present on the
/// request, and the deferred tool can still be called and completed by the loop.
#[tokio::test]
async fn test_gpt55_tool_search_low_threshold() {
    let Some(model) = OPENAI_GPT55.model() else {
        eprintln!("Skipping: {} not set", OPENAI_GPT55.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
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
        .unwrap();

    let result = runner.run_turn("What is 7 + 3?").await.unwrap();

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
/// GPT-5.4 occasionally answers "what time is it" from priors without calling a
/// tool, so the round-trip half is retried; the hosted-resolution half is
/// asserted on every attempt's generation events.
#[tokio::test]
async fn test_gpt54_auto_tool_search_resolves_to_hosted() {
    if OPENAI_GPT54.model().is_none() {
        eprintln!("Skipping: {} not set", OPENAI_GPT54.label());
        return;
    }

    const MAX_ATTEMPTS: usize = 5;
    let mut called_get_current_time = false;

    for attempt in 1..=MAX_ATTEMPTS {
        let model = OPENAI_GPT54.model().expect("checked above");
        let runner = InMemoryAgenticLoop::builder()
            .agent_name("Auto Tool Search Agent")
            .system_prompt(
                "You are a helpful assistant. When asked about time, use the get_current_time tool.",
            )
            .model(model)
            .driver_registry(all_providers_registry())
            // Multiple capabilities to exceed the 15-tool threshold (16 total).
            .capability(CurrentTimeCapability) // 1 tool
            .capability(TestMathCapability) // 4 tools
            .capability(TestWeatherCapability) // 2 tools
            .capability(FileSystemCapability) // 6 tools
            .capability(SessionCapability) // 2 tools
            .capability(StatelessTodoListCapability) // 1 tool
            // Model-adaptive: on GPT-5.4 this must resolve to the hosted mechanism.
            .capability(AutoToolSearchCapability::new())
            .max_iterations(5)
            .build()
            .await
            .unwrap();

        let result = runner.run_turn("What time is it right now?").await.unwrap();
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
                "auto_tool_search on GPT-5.4 must resolve to hosted: the client-side \
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
    let Some(model) = OPENAI_GPT55.model() else {
        eprintln!("Skipping: {} not set", OPENAI_GPT55.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
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
        .unwrap();

    let result = runner.run_turn("What is 7 + 3?").await.unwrap();

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
    let Some(model) = ANTHROPIC_HAIKU.model() else {
        eprintln!("Skipping: {} not set", ANTHROPIC_HAIKU.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Generic Tool Search Agent")
        .system_prompt("You are a helpful assistant. When asked to add numbers, use the add tool.")
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
        .unwrap();

    let result = runner.run_turn("What is 21 + 21?").await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Model should load and call the add tool via generic tool_search"
    );
}

/// Tests generic tool_search with a low threshold so it activates with few tools.
#[tokio::test]
async fn test_anthropic_generic_tool_search_low_threshold() {
    let Some(model) = ANTHROPIC_HAIKU.model() else {
        eprintln!("Skipping: {} not set", ANTHROPIC_HAIKU.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
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
        .unwrap();

    let result = runner.run_turn("What is 7 + 3?").await.unwrap();

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
    let Some(model) = ANTHROPIC_HAIKU.model() else {
        eprintln!("Skipping: {} not set", ANTHROPIC_HAIKU.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
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
        .unwrap();

    let result = runner.run_turn("What is 7 + 3?").await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Claude should call the add tool even with deferred schemas"
    );
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
}

/// Tests model-adaptive hosted resolution on Claude Haiku 4.5.
///
/// On a native Claude model, `auto_tool_search` must resolve (at
/// capability-collection time, via `Capability::resolve_for_model`) to the hosted
/// Anthropic mechanism — not the client-side fallback. The hosted mechanism adds
/// *no* client-side `tool_search` tool, so its absence from the model's tool list
/// proves hosted resolution, independent of whether a tool was called on a turn.
#[tokio::test]
async fn test_anthropic_auto_tool_search_resolves_to_hosted() {
    let Some(model) = ANTHROPIC_HAIKU.model() else {
        eprintln!("Skipping: {} not set", ANTHROPIC_HAIKU.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Claude Auto Tool Search Agent")
        .system_prompt("When asked to add numbers, use the add tool.")
        .model(model)
        .driver_registry(all_providers_registry())
        .capability(TestMathCapability)
        .capability(CurrentTimeCapability)
        // Model-adaptive: on Claude 4.5 this must resolve to the hosted mechanism.
        .capability(AutoToolSearchCapability::with_threshold(3))
        .max_iterations(6)
        .build()
        .await
        .unwrap();

    let result = runner.run_turn("What is 7 + 3?").await.unwrap();

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
            "auto_tool_search on Claude 4.5 must resolve to hosted: the client-side \
             `{TOOL_SEARCH_TOOL_NAME}` tool must not be offered to the model"
        );
    }
}

/// Reproduces the production surface where Opus 5 invented `bash_run` after
/// Bash's schema was deferred. Bash is a hot-path exception now, so hosted
/// search remains active for the rest of the tool set while the model always
/// receives Bash's exact `bash` / `commands` contract.
#[tokio::test]
async fn test_anthropic_opus5_hosted_search_calls_bash_contract() {
    let Some(model) = ANTHROPIC_OPUS5.model() else {
        eprintln!("Skipping: {} not set", ANTHROPIC_OPUS5.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
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
        .unwrap();

    let result = runner
        .run_turn("Use Bash to run `printf everruns-bash-contract`.")
        .await
        .unwrap();

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

    assert!(!bash_calls.is_empty(), "Opus 5 should call the `bash` tool");
    assert!(bash_calls.iter().all(|call| {
        call.arguments.get("commands").is_some() && call.arguments.get("command").is_none()
    }));
}

/// Tests that tool_search gracefully works when below threshold
/// (falls back to standard tool format — no namespaces, no defer_loading)
#[tokio::test]
async fn test_gpt54_tool_search_below_threshold_fallback() {
    let Some(model) = OPENAI_GPT54.model() else {
        eprintln!("Skipping: {} not set", OPENAI_GPT54.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
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
        .unwrap();

    let result = runner.run_turn("What time is it?").await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Should still work with standard tool format (below threshold)"
    );
}
