//! Extended thinking tests against real LLM endpoints (Anthropic only).
//!
//! These tests verify extended thinking (reasoning) works with the InMemoryAgenticLoop.
//! Extended thinking is an Anthropic-specific feature.
//!
//! Run with:
//!   cargo test -p everruns-core --test agent_run_with_thinking
//!
//! Required environment variables:
//!   - ANTHROPIC_API_KEY: For Anthropic tests
#![cfg(feature = "llm-tests")]

use everruns_core::capabilities::CurrentTimeCapability;
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::message::{ContentPart, Controls, MessageRole, ReasoningConfig};
use everruns_core::message_retriever::InputMessage;
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
    })
}

fn anthropic_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut registry);
    registry
}

// ============================================================================
// Extended Thinking Tests
// ============================================================================

#[tokio::test]
async fn test_extended_thinking() {
    let Some(model) = anthropic_model("claude-sonnet-4-20250514") else {
        eprintln!("Skipping test: ANTHROPIC_API_KEY not set");
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Thinking Agent")
        .system_prompt("You are a helpful assistant.")
        .model(model)
        .driver_registry(anthropic_registry())
        .max_iterations(3)
        .build()
        .await
        .unwrap();

    let input = InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text("What is 17 * 23?")],
        controls: Some(Controls {
            model_id: None,
            reasoning: Some(ReasoningConfig {
                effort: Some("medium".into()),
            }),
        }),
        metadata: None,
        tags: vec![],
    };

    let result = runner.run_turn(input).await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.response.contains("391"),
        "Response should contain the correct answer 391, got: {}",
        result.response
    );
}

#[tokio::test]
async fn test_thinking_with_tool_call() {
    let Some(model) = anthropic_model("claude-sonnet-4-20250514") else {
        eprintln!("Skipping test: ANTHROPIC_API_KEY not set");
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Thinking Time Agent")
        .system_prompt("Use get_current_time tool when asked about time.")
        .model(model)
        .driver_registry(anthropic_registry())
        .capability(CurrentTimeCapability)
        .max_iterations(5)
        .build()
        .await
        .unwrap();

    let input = InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text("What's the current time in UTC?")],
        controls: Some(Controls {
            model_id: None,
            reasoning: Some(ReasoningConfig {
                effort: Some("low".into()),
            }),
        }),
        metadata: None,
        tags: vec![],
    };

    let result = runner.run_turn(input).await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Should have called get_current_time"
    );
}
