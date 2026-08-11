// Parametrized agent-run tests against real LLM endpoints.
//
// Uses rstest to run every scenario against every provider × model combination
// defined in llm_test_matrix.rs. Add a new provider there and all tests here
// automatically cover it.
//
// Run all:
//   cargo test -p everruns-llm-tests --test agent_run_basic
//
// Run single provider:
//   cargo test -p everruns-llm-tests --test agent_run_basic -- anthropic
//
// Required env vars (tests skip gracefully if missing):
//   ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY, MODEL_API_KEY
//
// Skip specific providers: SKIP_LLM_INTEGRATION_TESTS_PROVIDERS=gemini,openai
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

use everruns_core::provider::DriverId;
use llm_test_matrix::*;
use rstest::rstest;

use everruns_core::capabilities::CurrentTimeCapability;
use everruns_core::traits::ResolvedModel;
use everruns_integrations_filesystem::FileSystemCapability;
use everruns_test_support::in_memory_loop::{InMemoryAgenticLoop, TurnResult};

// ============================================================================
// Scenario: basic completion (no tools)
// ============================================================================

#[rstest]
#[case::anthropic_haiku(ANTHROPIC_HAIKU)]
#[case::openai_gpt4o_mini(OPENAI_GPT4O_MINI)]
#[case::openai_gpt54(OPENAI_GPT54)]
#[case::gemini_flash(GEMINI_FLASH)]
#[case::openrouter_gpt4o_mini(OPENROUTER_GPT4O_MINI)]
#[case::fireworks_kimi_k2(FIREWORKS_KIMI_K2)]
#[case::meta_muse_spark_contributor(META_MUSE_SPARK_CONTRIBUTOR)]
#[tokio::test]
async fn test_basic_completion(#[case] config: ProviderModelConfig) {
    if config.model().is_none() {
        eprintln!("Skipping: {} not set", config.label());
        return;
    }

    let Some(result) = run_live_turn!(
        config,
        3,
        |r: &TurnResult| r.success && !r.response.is_empty(),
        {
            let model = config.model().expect("model set (checked above)");
            let runner = InMemoryAgenticLoop::builder()
                .agent_name("Dad Joke Agent")
                .system_prompt("Tell short dad jokes. Just the joke, no explanation.")
                .model(model)
                .driver_registry(all_providers_registry())
                .max_iterations(3)
                .build()
                .await
                .unwrap();
            runner.run_turn("Tell me a dad joke").await.unwrap()
        }
    ) else {
        return;
    };

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
#[case::openai_gpt54(OPENAI_GPT54)]
#[case::gemini_flash(GEMINI_FLASH)]
#[case::openrouter_gpt4o_mini(OPENROUTER_GPT4O_MINI)]
#[case::fireworks_kimi_k2(FIREWORKS_KIMI_K2)]
#[case::meta_muse_spark_contributor(META_MUSE_SPARK_CONTRIBUTOR)]
#[tokio::test]
async fn test_tool_call(#[case] config: ProviderModelConfig) {
    if config.model().is_none() {
        eprintln!("Skipping: {} not set", config.label());
        return;
    }

    // Live models are occasionally non-deterministic about emitting a tool call
    // for this borderline prompt: the "Should have called get_current_time"
    // assertion has flaked on main across successive pinned models (gpt-oss-120b,
    // then Anthropic Haiku / OpenAI). This case verifies the driver's
    // tool-calling *path* end-to-end against each provider — not single-shot
    // model determinism — so retry (also absorbing transient transport blips)
    // and require the tool to be called (and the loop to iterate) on at least
    // one attempt. A non-transient turn failure still surfaces via the asserts.
    let Some(result) = run_live_turn!(
        config,
        3,
        |r: &TurnResult| r.success && r.tool_calls_count > 0 && r.iterations > 1,
        {
            let model = config.model().expect("model set (checked above)");
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
            runner.run_turn("What time is it?").await.unwrap()
        }
    ) else {
        return;
    };

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Should have called get_current_time (tool_calls_count={}, iterations={})",
        result.tool_calls_count,
        result.iterations
    );
    assert!(
        result.iterations > 1,
        "Should have multiple iterations (reason -> act -> reason); iterations={}",
        result.iterations
    );
}

// ============================================================================
// Scenario: file system tools accepted by LLM (schema validation regression)
// ============================================================================

#[rstest]
#[case::anthropic_haiku(ANTHROPIC_HAIKU)]
#[case::openai_gpt4o_mini(OPENAI_GPT4O_MINI)]
#[case::openai_gpt54(OPENAI_GPT54)]
#[case::meta_muse_spark_contributor(META_MUSE_SPARK_CONTRIBUTOR)]
// Gemini excluded: rejects additionalProperties in nested object schemas (separate issue)
#[tokio::test]
async fn test_file_system_tool_schemas_accepted(#[case] config: ProviderModelConfig) {
    if config.model().is_none() {
        eprintln!("Skipping: {} not set", config.label());
        return;
    }

    // Regression: edit_file schema had top-level oneOf which OpenAI rejects.
    // This test ensures the schema is accepted by providers. Retry to absorb
    // transient transport blips (observed on main: "Stream error: Transport
    // error: error decoding response body") so a network hiccup during the live
    // turn doesn't red main CI.
    let Some(result) = run_live_turn!(config, 3, |r: &TurnResult| r.success, {
        let model = config.model().expect("model set (checked above)");
        let runner = InMemoryAgenticLoop::builder()
            .agent_name("File Agent")
            .system_prompt("Say hello. Do not use any tools.")
            .model(model)
            .driver_registry(all_providers_registry())
            .capability(FileSystemCapability)
            .max_iterations(1)
            .build()
            .await
            .unwrap();
        runner.run_turn("Say hello").await.unwrap()
    }) else {
        return;
    };

    assert!(
        result.success,
        "Turn should succeed with file system tools registered: {:?}",
        result.error
    );
}

// ============================================================================
// Scenario: model not available error handling
// ============================================================================

#[rstest]
#[case::anthropic_nonexistent(
    "claude-sonnet-4-6-20260217",
    DriverId::Anthropic,
    "ANTHROPIC_API_KEY"
)]
#[case::openai_nonexistent("gpt-99-nonexistent", DriverId::OpenAI, "OPENAI_API_KEY")]
#[tokio::test]
async fn test_model_not_available_returns_user_friendly_error(
    #[case] model_name: &str,
    #[case] provider_type: DriverId,
    #[case] env_var: &str,
) {
    let Some(api_key) = std::env::var(env_var).ok().filter(|k| !k.is_empty()) else {
        eprintln!("Skipping: {} not set", env_var);
        return;
    };

    let model = ResolvedModel {
        model: model_name.to_string(),
        provider_type,
        api_key: Some(api_key),
        base_url: None,
        provider_metadata: None,
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Test Agent")
        .system_prompt("You are helpful.")
        .model(model)
        .driver_registry(all_providers_registry())
        .max_iterations(1)
        .build()
        .await
        .unwrap();

    let result = runner.run_turn("Hello").await.unwrap();

    // If the provider account is out of credits/quota, the live API returns a
    // billing error before it ever validates the model name, so the
    // "model not available" path can't be exercised — skip rather than fail
    // (same billing-condition handling as the other live cases).
    skip_if_quota!(result, model_name);

    // The turn should complete (not crash) but with failure
    assert!(!result.success, "Turn should fail for nonexistent model");
    let error = result.error.as_deref().unwrap_or("");
    assert!(!error.is_empty(), "Should have error message");
    assert!(
        error.contains("Model not available"),
        "Error should mention model not available: {}",
        error
    );
    assert!(
        error.contains(model_name),
        "Error should contain the model name '{}': {}",
        model_name,
        error
    );
}
