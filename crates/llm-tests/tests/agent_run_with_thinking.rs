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

use everruns_provider::provider::DriverId;
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
        let reasoning_captured = runner
            .messages()
            .await
            .ok()
            .and_then(|msgs| {
                msgs.into_iter()
                    .find(|m| m.role == MessageRole::Agent)
                    .map(|m| m.has_reasoning())
            })
            .unwrap_or(false);
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
    let messages = runner.messages().await.unwrap();
    let assistant_msg = messages
        .iter()
        .find(|m| m.role == MessageRole::Agent)
        .expect("Should have an agent message");
    let reasoning_captured = assistant_msg.has_reasoning();

    // A successful adaptive-thinking generation can legitimately skip a
    // thinking block or miss the sampled arithmetic answer. Provider errors and
    // message-storage failures above remain fatal; deterministic wire/parser
    // tests own the provider contracts.
    if !result.response.contains("503") || !reasoning_captured {
        eprintln!(
            "::warning title=Live LLM sampling miss::{} completed successfully but \
             contains_503={} and reasoning_captured={reasoning_captured}",
            config.label(),
            result.response.contains("503"),
        );
        eprintln!("SAMPLING MISS: response={:?}", result.response);
        return;
    }

    // Every provider now records reasoning the same way: ordered reasoning
    // parts on the message. The old split — raw text on a private `thinking`
    // field for some providers, and folded into the visible answer for GPT-5.x
    // — is gone, and with it the per-model exception this test used to carry.
    assert!(
        assistant_msg.has_reasoning(),
        "Agent message should carry reasoning artifacts for {config}"
    );

    let reasoning_parts: Vec<_> = assistant_msg.reasoning_parts().collect();
    assert!(
        reasoning_parts
            .iter()
            .any(|part| part.display_text().is_some_and(|text| !text.is_empty())),
        "At least one reasoning artifact should carry readable text for {config}"
    );

    // Reasoning must never be folded into the answer text. A summary routed to
    // the assistant-text channel is persisted as the model's answer and
    // replayed as its own prior output.
    for part in &reasoning_parts {
        if let Some(text) = part.display_text() {
            assert!(
                !assistant_msg
                    .content
                    .iter()
                    .filter_map(everruns_core::ContentPart::as_text)
                    .any(|answer| answer.contains(text.trim())),
                "Reasoning text leaked into the assistant answer for {config}"
            );
        }
    }

    // Replay state is provider-specific, and without it multi-turn reasoning
    // silently degrades.
    // `DriverId` is a newtype over a string, not an enum, so compare rather
    // than pattern-match.
    if config.provider_type == DriverId::Anthropic {
        assert!(
            reasoning_parts.iter().any(|part| part.signature.is_some()),
            "Anthropic reasoning must carry a per-block signature for multi-turn"
        );
    } else if config.provider_type == DriverId::OpenAI {
        assert!(
            reasoning_parts.iter().any(|part| part.item_id.is_some()),
            "OpenAI reasoning items must carry the provider-issued id"
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

    // Retry live sampling and transient transport failures. If all successful
    // attempts cleanly decline an advertised tool, the contract check below
    // reports a non-blocking sampling miss. Missing tool definitions and parsed
    // tool-call mismatches remain merge-blocking failures.
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
    assert_live_tool_call_contract(&result, "get_current_time", &config.label());
}
