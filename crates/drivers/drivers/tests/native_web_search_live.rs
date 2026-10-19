#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live tests for native web search (`everruns/web_search`) on the OpenAI
//! Responses, OpenRouter, Anthropic and Gemini drivers.
//!
//! Each sends the shared `web_search` function tool with a native search
//! request, asks a question that needs the web, and checks that the provider
//! searched on its own: no `web_search` function call came back, and the
//! completion counts at least one `web_search_call` (the hosted-call cost
//! component of the generation).
//!
//! Key-gated, never ignored: a test without its key returns early, and one
//! whose account is out of credits or quota says so and returns, because that
//! is a billing condition rather than a code regression. Run:
//!   `doppler run -- cargo test -p everruns-drivers --features openai,openrouter,anthropic,gemini --test native_web_search_live -- --nocapture`

use everruns_contracts::driver_registry::{LlmCallConfig, LlmStreamEvent, Message, MessageRole};
use everruns_contracts::model::ReasoningEffort;
use everruns_contracts::native_web_search::NativeWebSearch;
use everruns_contracts::{Provider, ToolDefinition};
use futures::StreamExt;
use serde_json::json;

const PROMPT: &str = "Search the web: what is the latest stable Rust release? Answer in one line \
                      and cite the page you used.";

fn key(name: &str) -> Option<String> {
    let key = std::env::var(name)
        .ok()
        .filter(|key| !key.trim().is_empty());
    if key.is_none() {
        eprintln!("skipping: {name} is not set");
    }
    key
}

/// Billing conditions of the live account, not regressions.
fn out_of_credits(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    ["insufficient_quota", "quota", "credits", "billing", "402"]
        .iter()
        .any(|needle| error.contains(needle))
}

fn config(model: &str) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.reasoning_effort = Some(ReasoningEffort::Low);
    config.tools = vec![ToolDefinition::function(
        "web_search",
        "Search the web. Returns results with a title, url and snippet.",
        json!({ "type": "object", "properties": { "query": { "type": "string" } },
                "required": ["query"] }),
    )];
    let (key, value) = NativeWebSearch {
        provider: "auto".into(),
        ..Default::default()
    }
    .to_driver_option();
    config.driver_options.insert(key, value);
    config
}

/// Stream [`PROMPT`] and check the provider searched natively. Returns early
/// (with a note) when the account cannot pay for the call.
async fn assert_searched_natively(provider: Provider, config: LlmCallConfig) {
    let messages = vec![Message::text(MessageRole::User, PROMPT)];
    let mut stream = match provider.chat_completion_stream(messages, &config).await {
        Ok(stream) => stream,
        Err(error) if out_of_credits(&error.to_string()) => {
            eprintln!("skipping: live account cannot pay for the call: {error}");
            return;
        }
        Err(error) => panic!("provider rejected the native search request: {error}"),
    };
    let (mut text, mut hosted, mut done) = (String::new(), Vec::new(), None);
    while let Some(event) = stream.next().await {
        match event {
            Ok(LlmStreamEvent::TextDelta(delta)) => text.push_str(&delta),
            Ok(LlmStreamEvent::HostedToolCall(call)) => hosted.push(call),
            Ok(LlmStreamEvent::ToolCalls(calls)) => {
                assert!(
                    calls.iter().all(|call| call.name != "web_search"),
                    "the model called the function tool instead of searching natively: {calls:?}"
                );
            }
            Ok(LlmStreamEvent::Error(error)) if out_of_credits(&format!("{error:?}")) => {
                eprintln!("skipping: live account cannot pay for the call: {error:?}");
                return;
            }
            Ok(LlmStreamEvent::Error(error)) => panic!("stream error: {error:?}"),
            Ok(LlmStreamEvent::Done(meta)) => done = Some(meta),
            Ok(_) => {}
            Err(error) if out_of_credits(&error.to_string()) => {
                eprintln!("skipping: live account cannot pay for the call: {error}");
                return;
            }
            Err(error) => panic!("stream failed: {error}"),
        }
    }
    let done = done.expect("stream completes");
    eprintln!(
        "answer: {text}\nhosted: {hosted:?}\ncounts: {:?}\ncost: {:?}",
        done.hosted_tool_calls, done.provider_cost_usd
    );
    assert!(!text.trim().is_empty(), "model should answer");
    assert!(
        done.hosted_tool_calls
            .get("web_search_call")
            .copied()
            .unwrap_or(0)
            >= 1,
        "expected at least one native search, got {:?}",
        done.hosted_tool_calls
    );
}

#[tokio::test]
async fn openai_answers_the_web_search_tool_with_its_own_search() {
    let Some(api_key) = key("OPENAI_API_KEY") else {
        return;
    };
    let provider = everruns_drivers::openai::provider("openai", api_key);
    assert_searched_natively(provider, config("gpt-6-luna")).await;
}

#[tokio::test]
async fn openrouter_answers_the_web_search_tool_with_the_native_engine() {
    let Some(api_key) = key("OPENROUTER_API_KEY") else {
        return;
    };
    let provider = everruns_drivers::openrouter::provider("openrouter", api_key);
    assert_searched_natively(provider, config("openai/gpt-6-luna")).await;
}

/// A vendor whose search OpenRouter reports as its own `openrouter:web_search`
/// item and `server_tool_use_details`, not a `web_search_call`.
#[tokio::test]
async fn openrouter_counts_a_claude_search_through_its_server_tool() {
    let Some(api_key) = key("OPENROUTER_API_KEY") else {
        return;
    };
    let provider = everruns_drivers::openrouter::provider("openrouter", api_key);
    assert_searched_natively(provider, config("anthropic/claude-haiku-4.5")).await;
}

#[tokio::test]
async fn anthropic_answers_the_web_search_tool_with_its_server_tool() {
    let Some(api_key) = key("ANTHROPIC_API_KEY") else {
        return;
    };
    let provider = everruns_drivers::anthropic::provider("anthropic", api_key);
    assert_searched_natively(provider, config("claude-haiku-5-5")).await;
}

#[tokio::test]
async fn gemini_answers_the_web_search_tool_with_google_search() {
    let Some(api_key) = key("GEMINI_API_KEY") else {
        return;
    };
    let provider = everruns_drivers::gemini::provider("gemini", api_key);
    assert_searched_natively(provider, config("gemini-3.8-flash")).await;
}
