// Extended thinking tests against real LLM endpoints.
//
// Thinking/reasoning is provider-specific (Anthropic, Meta, Gemini, and some OpenAI models).
// Uses rstest so new thinking-capable providers can be added as cases.
//
// Run with:
//   cargo test -p everruns-core --test agent_run_with_thinking
//
// Required environment variables:
//   - ANTHROPIC_API_KEY: For Anthropic tests
//   - OPENAI_API_KEY: For OpenAI reasoning tests (GPT-5.2)
//   - MODEL_API_KEY: For Meta Muse Spark Contributor tests
#![cfg(feature = "llm-tests")]

mod llm_test_matrix;

use everruns_core::provider::DriverId;
use llm_test_matrix::*;
use rstest::rstest;

use everruns_builtins::CurrentTimeCapability;
use everruns_core::message::{ContentPart, Controls, MessageRole, ReasoningConfig};
use everruns_core::message_retriever::InputMessage;
use everruns_test_support::in_memory_loop::{InMemoryAgenticLoop, TurnResult};

// ============================================================================
// Scenario: extended thinking (reasoning)
// ============================================================================

#[rstest]
// Uses Opus 5 rather than the `claude-opus-4-7` alias: that alias now returns
// its reasoning as public response text with an empty private `thinking` field
// (observed 6/6 on main), which is not what this private-field assertion checks.
// Current Opus/Sonnet models surface reasoning on the private field.
#[case::anthropic_opus5(ANTHROPIC_OPUS5)]
#[case::anthropic_sonnet(ANTHROPIC_SONNET)]
#[case::openai_gpt52(OPENAI_GPT52)]
#[case::openai_gpt54(OPENAI_GPT54)]
#[case::meta_muse_spark_contributor(META_MUSE_SPARK_CONTRIBUTOR)]
#[tokio::test]
async fn test_extended_thinking(#[case] config: ProviderModelConfig) {
    if config.model().is_none() {
        eprintln!("Skipping: {} not set", config.label());
        return;
    }

    // Retry the build+run against transient transport blips and adaptive-thinking
    // non-determinism, but keep the accepted attempt's `runner` because the
    // reasoning-field assertions below inspect `runner.messages()` — so this
    // cannot use the `run_live_turn!` macro (which only returns the result).
    //
    // The acceptance predicate below requires success + the answer + captured
    // reasoning, so a single attempt can miss on any of three independent axes
    // (streaming/transport error, answer not yet emitted, reasoning surfaced as
    // public text instead of the private field). Opus in particular flaked all
    // three in one run on `main`, so the budget is generous enough to absorb a
    // couple of bad draws while still failing a genuinely broken provider.
    const MAX_ATTEMPTS: usize = 6;
    let mut accepted: Option<(InMemoryAgenticLoop, TurnResult)> = None;
    for attempt in 1..=MAX_ATTEMPTS {
        let model = config.model().expect("model set (checked above)");
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
        // Trailing zeros of 2024! = floor(2024/5) + floor(2024/25) +
        // floor(2024/125) + floor(2024/625) = 404 + 80 + 16 + 3 = 503.
        let input = InputMessage {
            role: MessageRole::User,
            content: vec![ContentPart::text(
                "How many trailing zeros does 2024! (factorial) have? Work it out carefully, then state the final count as a plain number.",
            )],
            controls: Some(Controls {
                speed: None,
                verbosity: None,
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

        skip_if_quota!(result, config.label());

        // Adaptive-thinking models occasionally answer correctly without emitting
        // a thinking block; for providers that keep reasoning on the private
        // `thinking` field, require it in the acceptance check so we retry rather
        // than accept an attempt that would fail the reasoning assertions below.
        let reasoning_captured = if config.reasoning_on_thinking_field {
            runner
                .messages()
                .await
                .ok()
                .and_then(|msgs| {
                    msgs.into_iter()
                        .find(|m| m.role == MessageRole::Agent)
                        .map(|m| m.thinking.is_some_and(|t| !t.is_empty()))
                })
                .unwrap_or(false)
        } else {
            true
        };
        if result.success && result.response.contains("503") && reasoning_captured {
            accepted = Some((runner, result));
            break;
        }
        if attempt == MAX_ATTEMPTS {
            // Fall through with the last attempt so the assertions below produce
            // the precise failure message (success / contains-503 / reasoning).
            accepted = Some((runner, result));
            break;
        }
        let transient = !result.success
            && result
                .error
                .as_deref()
                .is_some_and(is_transient_transport_error);
        eprintln!(
            "{}: attempt {attempt}/{} extended-thinking not accepted \
             (success={}, transient_transport={}, contains_503={}, reasoning_captured={}, error={:?}); retrying",
            config.label(),
            MAX_ATTEMPTS,
            result.success,
            transient,
            result.response.contains("503"),
            reasoning_captured,
            result.error,
        );
    }

    let (runner, result) = accepted.expect("retry loop always populates a result");

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.response.contains("503"),
        "Response should contain the answer 503, got: {}",
        result.response
    );

    // Verify reasoning was captured on the stored assistant message.
    let messages = runner.messages().await.unwrap();
    let assistant_msg = messages
        .iter()
        .find(|m| m.role == MessageRole::Agent)
        .expect("Should have an agent message");

    if config.reasoning_on_thinking_field {
        // Anthropic (and OpenAI o-series) keep raw reasoning on the private
        // `thinking` field.
        assert!(
            assistant_msg.thinking.is_some(),
            "Agent message should have thinking content for {config}"
        );
        assert!(
            !assistant_msg.thinking.as_ref().unwrap().is_empty(),
            "Thinking content should not be empty for {config}"
        );
    } else {
        // OpenAI GPT-5.x surfaces its readable reasoning summary as public
        // text instead of on the private `thinking` field. The summary is
        // folded into the response, which we already asserted is non-empty and
        // carries the worked-out answer above, so `thinking` must stay empty
        // (absent or blank).
        assert!(
            assistant_msg.thinking.as_ref().is_none_or(|t| t.is_empty()),
            "GPT-5.x reasoning summary should surface as public text, not on \
             the private thinking field, for {config}"
        );
    }

    // thinking_signature is provider-specific:
    // - Anthropic: cryptographic signature (always present with thinking)
    // - OpenAI o-series: encrypted_content (present when model uses encrypted reasoning)
    // - OpenAI GPT-5.x: not used (reasoning summary is readable, not encrypted)
    if config.provider_type == DriverId::Anthropic {
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
#[case::anthropic_opus(ANTHROPIC_OPUS)]
#[case::anthropic_sonnet(ANTHROPIC_SONNET)]
#[case::openai_gpt52(OPENAI_GPT52)]
#[case::openai_gpt54(OPENAI_GPT54)]
#[case::meta_muse_spark_contributor(META_MUSE_SPARK_CONTRIBUTOR)]
#[tokio::test]
async fn test_thinking_with_tool_call(#[case] config: ProviderModelConfig) {
    if config.model().is_none() {
        eprintln!("Skipping: {} not set", config.label());
        return;
    }

    // Live models are occasionally non-deterministic about emitting a tool call
    // for this prompt (the "Should have called get_current_time" assertion has
    // flaked on main), so retry — also absorbing transient transport blips —
    // and require the tool to be called on at least one attempt. A non-transient
    // turn failure still surfaces via the asserts below.
    let Some(result) = run_live_turn!(
        config,
        3,
        |r: &TurnResult| r.success && r.tool_calls_count > 0,
        {
            let model = config.model().expect("model set (checked above)");
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
                    speed: None,
                    verbosity: None,
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
            runner.run_turn(input).await.unwrap()
        }
    ) else {
        return;
    };

    assert!(result.success, "Turn should succeed: {:?}", result.error);
    assert!(
        result.tool_calls_count > 0,
        "Should have called get_current_time (tool_calls_count={})",
        result.tool_calls_count
    );
}
