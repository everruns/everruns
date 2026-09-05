//! Live smoke tests for Fireworks AI chat + model discovery.
//!
//! Exercises a real streamed chat completion through `FireworksChatDriver` plus
//! live `/models` discovery and profile mapping. These are the live counterparts
//! to the wiremock unit tests. (Live tool-calling is covered by the shared
//! `everruns-llm-tests` matrix via the `FIREWORKS_KIMI_K2` case.)
//!
//! Ignored by default (requires network + `FIREWORKS_API_KEY`); run manually:
//!   `doppler run -- cargo test -p everruns-fireworks --test chat_live -- --ignored --nocapture`

use everruns_fireworks::provider;
use everruns_provider::driver_registry::{
    LlmCallConfig, LlmMessage, LlmMessageRole, LlmStreamEvent,
};
use futures::StreamExt;

const LIVE_MODEL: &str = "accounts/fireworks/models/gpt-oss-120b";

fn api_key() -> String {
    std::env::var("FIREWORKS_API_KEY")
        .expect("FIREWORKS_API_KEY must be set for the live smoke test")
}

#[tokio::test]
#[ignore = "live network + FIREWORKS_API_KEY"]
async fn fireworks_chat_streams_response() {
    let provider = provider("fireworks", api_key());

    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: LIVE_MODEL.to_string(),
        temperature: Some(0.0),
        // Generous budget: reasoning models can spend tokens on
        // hidden reasoning before emitting the visible answer.
        max_tokens: Some(512),
        tools: vec![],
        reasoning_effort: None,
        metadata: std::collections::HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        openrouter_routing: None,
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        reasoning_state: None,
    };

    let messages = vec![LlmMessage::text(
        LlmMessageRole::User,
        "Reply with exactly one word: pong",
    )];

    let mut stream = provider
        .chat_completion_stream(messages, &config)
        .await
        .expect("Fireworks should accept the chat request");

    let mut text = String::new();
    let mut thinking = String::new();
    let mut done = false;
    let mut error: Option<String> = None;
    while let Some(event) = stream.next().await {
        match event.expect("stream item should not be a transport error") {
            LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
            LlmStreamEvent::ReasoningDelta { delta, .. } => thinking.push_str(&delta),
            LlmStreamEvent::Done(meta) => {
                done = true;
                eprintln!(
                    "Fireworks chat done: finish={:?} tokens in/out={:?}/{:?}",
                    meta.finish_reason, meta.prompt_tokens, meta.completion_tokens,
                );
            }
            LlmStreamEvent::Error(e) => error = Some(e.to_string()),
            _ => {}
        }
    }

    assert!(error.is_none(), "stream returned an error: {error:?}");
    assert!(done, "stream did not complete with a Done event");
    // A reasoning model may surface output as visible text and/or hidden
    // reasoning; the contract is that the turn produced *some* model output.
    assert!(
        !text.trim().is_empty() || !thinking.trim().is_empty(),
        "expected non-empty assistant output (text or thinking)"
    );
    eprintln!(
        "Fireworks chat reply: text={text:?} thinking_len={}",
        thinking.len()
    );
}

#[tokio::test]
#[ignore = "live network + FIREWORKS_API_KEY"]
async fn fireworks_discovery_returns_chat_models_with_profiles() {
    let provider = provider("fireworks", api_key());

    let models = provider
        .list_models()
        .await
        .expect("discovery request should succeed")
        .expect("Fireworks discovery should be supported");

    assert!(!models.is_empty(), "expected at least one discovered model");
    for m in &models {
        // Only chat models should be returned (image/non-chat filtered out).
        let profile = m
            .discovered_profile
            .as_ref()
            .expect("each model should carry a discovered profile");
        eprintln!(
            "model={} tools={} attach={} ctx={:?}",
            m.model_id,
            profile.tool_call,
            profile.attachment,
            profile.limits.as_ref().map(|l| l.context),
        );
    }
}
