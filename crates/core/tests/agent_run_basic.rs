// Parametrized agent-run tests against real LLM endpoints.
//
// Uses rstest to run every scenario against every provider × model combination
// defined in llm_test_matrix.rs. Add a new provider there and all tests here
// automatically cover it.
//
// Run all:
//   cargo test -p everruns-core --test agent_run_basic
//
// Run single provider:
//   cargo test -p everruns-core --test agent_run_basic -- anthropic
//
// Required env vars (tests skip gracefully if missing):
//   ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY
//
// Skip specific providers: SKIP_LLM_INTEGRATION_TESTS_PROVIDERS=gemini,openai
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

use llm_test_matrix::*;
use rstest::rstest;

use everruns_core::capabilities::CurrentTimeCapability;
use everruns_core::in_memory_loop::InMemoryAgenticLoop;

// ============================================================================
// Scenario: basic completion (no tools)
// ============================================================================

#[rstest]
#[case::anthropic_haiku(ANTHROPIC_HAIKU)]
#[case::openai_gpt4o_mini(OPENAI_GPT4O_MINI)]
#[case::gemini_flash(GEMINI_FLASH)]
#[tokio::test]
async fn test_basic_completion(#[case] config: ProviderModelConfig) {
    let Some(model) = config.model() else {
        eprintln!("Skipping: {} not set", config.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Dad Joke Agent")
        .system_prompt("Tell short dad jokes. Just the joke, no explanation.")
        .model(model)
        .driver_registry(all_providers_registry())
        .max_iterations(3)
        .build()
        .await
        .unwrap();

    let result = runner.run_turn("Tell me a dad joke").await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(!result.response.is_empty(), "Response should not be empty");
    assert_eq!(result.tool_calls_count, 0, "No tools should be called");
}

// ============================================================================
// Scenario: tool calling (CurrentTime capability)
// ============================================================================

#[rstest]
#[case::anthropic_haiku(ANTHROPIC_HAIKU)]
#[case::openai_gpt4o_mini(OPENAI_GPT4O_MINI)]
#[case::gemini_flash(GEMINI_FLASH)]
#[tokio::test]
async fn test_tool_call(#[case] config: ProviderModelConfig) {
    let Some(model) = config.model() else {
        eprintln!("Skipping: {} not set", config.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Time Agent")
        .system_prompt("When asked about time, use get_current_time tool first.")
        .model(model)
        .driver_registry(all_providers_registry())
        .capability(CurrentTimeCapability)
        .max_iterations(5)
        .build()
        .await
        .unwrap();

    let result = runner.run_turn("What time is it?").await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Should have called get_current_time"
    );
    assert!(
        result.iterations > 1,
        "Should have multiple iterations (reason -> act -> reason)"
    );
}
