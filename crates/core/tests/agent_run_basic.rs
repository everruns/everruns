//! Basic agent run tests against real LLM endpoints (Anthropic + OpenAI).
//!
//! These tests verify the InMemoryAgenticLoop works with real LLM providers.
//! Tests skip gracefully if API keys are not set.
//!
//! Run with:
//!   cargo test -p everruns-core --test agent_run_basic
//!
//! Required environment variables:
//!   - ANTHROPIC_API_KEY: For Anthropic tests
//!   - OPENAI_API_KEY: For OpenAI tests
#![cfg(feature = "llm-tests")]

use everruns_core::capabilities::CurrentTimeCapability;
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::traits::ModelWithProvider;

// ============================================================================
// Helpers
// ============================================================================

fn anthropic_model(model_name: &str) -> Option<ModelWithProvider> {
    let api_key = std::env::var("ANTHROPIC_API_KEY").ok()?;
    Some(ModelWithProvider {
        model: model_name.to_string(),
        provider_type: LlmProviderType::Anthropic,
        api_key: Some(api_key),
        base_url: None,
        provider_settings: None,
    })
}

fn openai_model(model_name: &str) -> Option<ModelWithProvider> {
    let api_key = std::env::var("OPENAI_API_KEY").ok()?;
    Some(ModelWithProvider {
        model: model_name.to_string(),
        provider_type: LlmProviderType::Openai,
        api_key: Some(api_key),
        base_url: None,
        provider_settings: None,
    })
}

fn anthropic_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut registry);
    registry
}

fn openai_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    everruns_openai::register_driver(&mut registry);
    registry
}

// ============================================================================
// Anthropic Tests
// ============================================================================

#[tokio::test]
async fn test_anthropic_basic_completion() {
    let Some(model) = anthropic_model("claude-3-5-haiku-20241022") else {
        eprintln!("Skipping test: ANTHROPIC_API_KEY not set");
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Dad Joke Agent")
        .system_prompt("Tell short dad jokes. Just the joke, no explanation.")
        .model(model)
        .driver_registry(anthropic_registry())
        .max_iterations(3)
        .build()
        .await
        .unwrap();

    let result = runner.run_turn("Tell me a dad joke").await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(!result.response.is_empty(), "Response should not be empty");
    assert_eq!(result.tool_calls_count, 0, "No tools should be called");
}

#[tokio::test]
async fn test_anthropic_tool_call() {
    let Some(model) = anthropic_model("claude-3-5-haiku-20241022") else {
        eprintln!("Skipping test: ANTHROPIC_API_KEY not set");
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Time Agent")
        .system_prompt("When asked about time, use get_current_time tool first.")
        .model(model)
        .driver_registry(anthropic_registry())
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

// ============================================================================
// OpenAI Tests
// ============================================================================

#[tokio::test]
async fn test_openai_basic_completion() {
    let Some(model) = openai_model("gpt-4o-mini") else {
        eprintln!("Skipping test: OPENAI_API_KEY not set");
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Dad Joke Agent")
        .system_prompt("Tell short dad jokes. Just the joke, no explanation.")
        .model(model)
        .driver_registry(openai_registry())
        .max_iterations(3)
        .build()
        .await
        .unwrap();

    let result = runner.run_turn("Tell me a dad joke").await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(!result.response.is_empty(), "Response should not be empty");
    assert_eq!(result.tool_calls_count, 0, "No tools should be called");
}

#[tokio::test]
async fn test_openai_tool_call() {
    let Some(model) = openai_model("gpt-4o-mini") else {
        eprintln!("Skipping test: OPENAI_API_KEY not set");
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Time Agent")
        .system_prompt("When asked about time, use get_current_time tool first.")
        .model(model)
        .driver_registry(openai_registry())
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
