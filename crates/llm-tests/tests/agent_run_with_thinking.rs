// Extended thinking tests against real LLM endpoints.
//
// Thinking/reasoning is provider-specific (Anthropic, Gemini, some OpenAI models).
// Uses rstest so new thinking-capable providers can be added as cases.
//
// OpenAI reasoning models split into two behaviors that need separate coverage:
//   - Raw-reasoning models (GPT-5.5) stream `ReasoningTextDelta`, which maps to
//     the private `thinking` field — see `test_extended_thinking`.
//   - Summary-only models (GPT-5.2/5.4) stream `ReasoningSummaryDelta`, a
//     readable progress summary that is surfaced as public response text and so
//     leaves `thinking` empty — see `test_reasoning_summary_surfaced_as_text`.
//
// Run with:
//   cargo test -p everruns-core --test agent_run_with_thinking
//
// Required environment variables:
//   - ANTHROPIC_API_KEY: For Anthropic tests
//   - OPENAI_API_KEY: For OpenAI reasoning tests (GPT-5.2/5.4/5.5)
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

use everruns_core::provider::DriverId;
use llm_test_matrix::*;
use rstest::rstest;

use everruns_core::capabilities::CurrentTimeCapability;
use everruns_core::in_memory_loop::InMemoryAgenticLoop;
use everruns_core::message::{ContentPart, Controls, MessageRole, ReasoningConfig};
use everruns_core::message_retriever::InputMessage;

// ============================================================================
// Scenario: extended thinking (reasoning)
// ============================================================================

// Providers that stream raw reasoning, which lands on the private `thinking`
// field: Anthropic (all models) and OpenAI GPT-5.5. Summary-only OpenAI models
// (GPT-5.2/5.4) are covered by `test_reasoning_summary_surfaced_as_text`.
#[rstest]
#[case::anthropic_opus(ANTHROPIC_OPUS)]
#[case::anthropic_sonnet(ANTHROPIC_SONNET)]
#[case::openai_gpt55(OPENAI_GPT55)]
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

    // The prompt must be genuinely hard: on adaptive-thinking models (Claude
    // Fable 5 / Opus 4.8+, GPT-5.x reasoning) the model decides whether to
    // think at all, and trivial riddles deterministically skip thinking.
    // Trailing zeros of 2024! = floor(2024/5) + floor(2024/25) + floor(2024/125)
    // + floor(2024/625) = 404 + 80 + 16 + 3 = 503.
    let input = InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text(
            "How many trailing zeros does 2024! (factorial) have? Work it out carefully, then state the final count as a plain number.",
        )],
        controls: Some(Controls {
            model_id: None,
            locale: None,
            reasoning: Some(ReasoningConfig {
                effort: Some("high".into()),
            }),
            error_disclosure: None,
            hints: None,
        }),
        metadata: None,
        tags: vec![],
    };

    let result = runner.run_turn(input).await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.response.contains("503"),
        "Response should contain the answer 503, got: {}",
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
    // - OpenAI o-series / GPT-5.5: encrypted_content (present when the model
    //   uses encrypted reasoning); not asserted here as it is best-effort.
    if matches!(config.provider_type, DriverId::Anthropic) {
        assert!(
            assistant_msg.thinking_signature.is_some(),
            "Anthropic should have thinking_signature for multi-turn"
        );
    }
}

// ============================================================================
// Scenario: reasoning summary surfaced as public text (summary-only models)
// ============================================================================

// GPT-5.2/5.4 stream a readable reasoning *summary* (`ReasoningSummaryDelta`)
// rather than raw chain-of-thought. That summary is mapped to public response
// text (see `openresponses_protocol::handle_streaming_event`), so it shows up
// in the assistant response and the private `thinking` field stays empty.
#[rstest]
#[case::openai_gpt52(OPENAI_GPT52)]
#[case::openai_gpt54(OPENAI_GPT54)]
#[tokio::test]
async fn test_reasoning_summary_surfaced_as_text(#[case] config: ProviderModelConfig) {
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

    // Same hard prompt as `test_extended_thinking`: a trivial prompt would let
    // adaptive-reasoning models skip reasoning entirely.
    let input = InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text(
            "How many trailing zeros does 2024! (factorial) have? Work it out carefully, then state the final count as a plain number.",
        )],
        controls: Some(Controls {
            model_id: None,
            locale: None,
            reasoning: Some(ReasoningConfig {
                effort: Some("high".into()),
            }),
            error_disclosure: None,
            hints: None,
        }),
        metadata: None,
        tags: vec![],
    };

    let result = runner.run_turn(input).await.unwrap();

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.response.contains("503"),
        "Response should contain the answer 503, got: {}",
        result.response
    );

    // The reasoning summary is surfaced as public text, not hidden reasoning,
    // so the private `thinking` field must stay empty for summary-only models.
    let messages = runner.messages().await.unwrap();
    let assistant_msg = messages
        .iter()
        .find(|m| m.role == MessageRole::Agent)
        .expect("Should have an agent message");

    assert!(
        assistant_msg.thinking.as_ref().is_none_or(|t| t.is_empty()),
        "Summary-only model {config} should not populate private thinking, got: {:?}",
        assistant_msg.thinking
    );
}

// ============================================================================
// Scenario: thinking + tool calling
// ============================================================================

#[rstest]
#[case::anthropic_opus(ANTHROPIC_OPUS)]
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
            locale: None,
            reasoning: Some(ReasoningConfig {
                effort: Some("low".into()),
            }),
            error_disclosure: None,
            hints: None,
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
