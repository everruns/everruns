//! Live smoke test for OpenRouter chat + session tracking.
//!
//! Exercises a real streamed chat completion through `OpenRouterChatDriver`,
//! including the `OpenRouterRequestExtension` that forwards `session_id` and
//! routing controls. Confirms OpenRouter accepts the decorated request and
//! streams back a non-empty response — the live counterpart to the wiremock
//! request-contract tests.
//!
//! Ignored by default (requires network + `OPENROUTER_API_KEY`); run manually:
//!   `doppler run -- cargo test -p everruns-openrouter --test chat_live -- --ignored --nocapture`

use everruns_core::driver_registry::{
    ChatDriver, LlmCallConfig, LlmMessage, LlmMessageRole, LlmStreamEvent, OpenRouterRoute,
    OpenRouterRoutingConfig,
};
use everruns_openrouter::OpenRouterChatDriver;
use futures::StreamExt;

#[tokio::test]
#[ignore = "live network + OPENROUTER_API_KEY"]
async fn openrouter_chat_with_session_id_and_routing_succeeds() {
    let api_key = std::env::var("OPENROUTER_API_KEY")
        .expect("OPENROUTER_API_KEY must be set for the live smoke test");

    let driver = OpenRouterChatDriver::new(api_key);

    let mut metadata = std::collections::HashMap::new();
    metadata.insert("session_id".to_string(), "session_live_smoke".to_string());

    let config = LlmCallConfig {
        speed: None,
        model: "openai/gpt-4o-mini".to_string(),
        temperature: Some(0.0),
        max_tokens: Some(16),
        tools: vec![],
        reasoning_effort: None,
        metadata,
        previous_response_id: None,
        tool_search: None,
        prompt_cache: None,
        // Exercise the routing-decoration path alongside session_id forwarding.
        openrouter_routing: Some(OpenRouterRoutingConfig {
            models: vec![
                "openai/gpt-4o-mini".to_string(),
                "openai/gpt-4o".to_string(),
            ],
            route: Some(OpenRouterRoute::Fallback),
            ..Default::default()
        }),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
    };

    let messages = vec![LlmMessage::text(
        LlmMessageRole::User,
        "Reply with exactly one word: pong",
    )];

    let mut stream = driver
        .chat_completion_stream(messages, &config)
        .await
        .expect("OpenRouter should accept the decorated chat request");

    let mut text = String::new();
    let mut done = false;
    let mut error: Option<String> = None;
    while let Some(event) = stream.next().await {
        match event.expect("stream item should not be a transport error") {
            LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
            LlmStreamEvent::Done(meta) => {
                done = true;
                eprintln!(
                    "OpenRouter chat done: finish={:?} tokens in/out={:?}/{:?} cost_usd={:?}",
                    meta.finish_reason,
                    meta.prompt_tokens,
                    meta.completion_tokens,
                    meta.provider_cost_usd,
                );
            }
            LlmStreamEvent::Error(e) => error = Some(e.to_string()),
            _ => {}
        }
    }

    assert!(error.is_none(), "stream returned an error: {error:?}");
    assert!(done, "stream did not complete with a Done event");
    assert!(
        !text.trim().is_empty(),
        "expected non-empty assistant text, got {text:?}"
    );
    eprintln!("OpenRouter chat reply: {text:?}");
}
