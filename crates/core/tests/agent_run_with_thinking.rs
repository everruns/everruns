// Extended thinking tests against real LLM endpoints.
//
// Thinking/reasoning is provider-specific (Anthropic, Gemini, some OpenAI models).
// Uses rstest so new thinking-capable providers can be added as cases.
//
// Run with:
//   cargo test -p everruns-core --test agent_run_with_thinking
//
// Required environment variables:
//   - ANTHROPIC_API_KEY: For Anthropic tests
//   - OPENAI_API_KEY: For OpenAI reasoning tests (GPT-5.2)
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

use llm_test_matrix::*;
use rstest::rstest;

use everruns_core::capabilities::CurrentTimeCapability;
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::message::{ContentPart, Controls, MessageRole, ReasoningConfig};
use everruns_core::message_retriever::InputMessage;

// ============================================================================
// Scenario: extended thinking (reasoning)
// ============================================================================

#[rstest]
#[case::anthropic_sonnet(ANTHROPIC_SONNET)]
#[case::openai_gpt52(OPENAI_GPT52)]
#[case::openai_gpt54(OPENAI_GPT54)]
#[tokio::test]
async fn test_extended_thinking(#[case] config: ProviderModelConfig) {
    let Some(model) = config.model() else {
        eprintln!("Skipping: {} not set", config.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Thinking Agent")
        .system_prompt("You are a helpful assistant.")
        .model(model)
        .driver_registry(all_providers_registry())
        .max_iterations(3)
        .build()
        .await
        .unwrap();

    let input = InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text(
            "A farmer has 17 chickens. All but 9 die. How many are left? Explain step by step why this is tricky.",
        )],
        controls: Some(Controls {
            model_id: None,
            reasoning: Some(ReasoningConfig {
                effort: Some("high".into()),
            }),
        }),
        metadata: None,
        tags: vec![],
    };

    let result = runner.run_turn(input).await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.response.contains("9"),
        "Response should contain the answer 9, got: {}",
        result.response
    );

    // Verify thinking tokens were captured on the stored assistant message
    let messages = runner.messages().await.unwrap();
    let assistant_msg = messages
        .iter()
        .find(|m| m.role == MessageRole::Agent)
        .expect("Should have an agent message");

    assert!(
        assistant_msg.thinking.is_some(),
        "Agent message should have thinking content for {config}"
    );
    assert!(
        !assistant_msg.thinking.as_ref().unwrap().is_empty(),
        "Thinking content should not be empty for {config}"
    );

    // thinking_signature is provider-specific:
    // - Anthropic: cryptographic signature (always present with thinking)
    // - OpenAI o-series: encrypted_content (present when model uses encrypted reasoning)
    // - OpenAI GPT-5.x: not used (reasoning summary is readable, not encrypted)
    if matches!(config.provider_type, LlmProviderType::Anthropic) {
        assert!(
            assistant_msg.thinking_signature.is_some(),
            "Anthropic should have thinking_signature for multi-turn"
        );
    }
}

// ============================================================================
// Scenario: thinking + tool calling
// ============================================================================

#[rstest]
#[case::anthropic_sonnet(ANTHROPIC_SONNET)]
#[case::openai_gpt52(OPENAI_GPT52)]
#[case::openai_gpt54(OPENAI_GPT54)]
#[tokio::test]
async fn test_thinking_with_tool_call(#[case] config: ProviderModelConfig) {
    let Some(model) = config.model() else {
        eprintln!("Skipping: {} not set", config.label());
        return;
    };

    let runner = InMemoryAgenticLoop::builder()
        .agent_name("Thinking Time Agent")
        .system_prompt("Use get_current_time tool when asked about time.")
        .model(model)
        .driver_registry(all_providers_registry())
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
