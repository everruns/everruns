// Integration tests for tool_search (deferred tool loading).
//
// Tests the full pipeline: OpenAiToolSearchCapability → RuntimeAgent → LlmCallConfig →
// OpenResponses driver with tool_search enabled.
//
// Includes a real GPT-5.4 integration test that exercises tool_search end-to-end.
//
// Run all:
//   cargo test -p everruns-llm-tests --test tool_search_test --features llm-tests
//
// Required env vars (tests skip gracefully if missing):
//   OPENAI_API_KEY
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

use llm_test_matrix::*;

use everruns_core::capabilities::{
    CurrentTimeCapability, FileSystemCapability, GenericToolSearchCapability,
    OpenAiToolSearchCapability, SessionCapability, StatelessTodoListCapability, TestMathCapability,
    TestWeatherCapability,
};
use everruns_core::in_memory_loop::InMemoryAgenticLoop;

// ============================================================================
// Scenario: tool_search with GPT-5.4 (many tools → deferred loading)
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
        .capability(GenericToolSearchCapability::new())
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
        .capability(GenericToolSearchCapability::with_threshold(3))
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
