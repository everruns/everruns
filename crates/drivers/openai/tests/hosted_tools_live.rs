#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live test for OpenAI hosted web search through the Responses driver
//! (EVE-1115) against the real OpenAI API.
//!
//! Proves OpenAI accepts the rendered `web_search` tool, runs it inside the
//! response, and that the stream reports the call as hosted-call progress plus
//! a per-call count in the completion metadata, never as an agent tool call.
//!
//! Ignored by default (requires network + `OPENAI_API_KEY`); run manually:
//!   `doppler run -- cargo test -p everruns-openai --test hosted_tools_live -- --ignored --nocapture`

use everruns_openai::provider;
use everruns_provider::driver_registry::{
    HostedToolCallStatus, LlmCallConfig, LlmStreamEvent, Message, MessageRole,
};
use everruns_provider::model::ReasoningEffort;
use everruns_provider::openai_hosted_tools::{OpenAiHostedTools, SearchContextSize, WebSearchTool};
use futures::StreamExt;

const LIVE_MODEL: &str = "gpt-5.6-luna";

#[tokio::test]
#[ignore = "live network + OPENAI_API_KEY"]
async fn openai_runs_hosted_web_search() {
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for the live test");
    let mut config = LlmCallConfig::new(LIVE_MODEL);
    config.reasoning_effort = Some(ReasoningEffort::Low);
    let (key, value) = OpenAiHostedTools {
        web_search: Some(WebSearchTool {
            search_context_size: Some(SearchContextSize::Low),
            ..Default::default()
        }),
    }
    .to_driver_option()
    .unwrap();
    config.driver_options.insert(key, value);

    let messages = vec![Message::text(
        MessageRole::User,
        "Search the web: what is the latest stable Rust release? Answer in one line.",
    )];
    let mut stream = provider("openai", api_key)
        .chat_completion_stream(messages, &config)
        .await
        .expect("OpenAI should accept the hosted web search tool");

    let (mut text, mut hosted, mut done) = (String::new(), Vec::new(), None);
    while let Some(event) = stream.next().await {
        match event.expect("stream should not fail") {
            LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
            LlmStreamEvent::HostedToolCall(call) => hosted.push(call),
            LlmStreamEvent::ToolCalls(calls) => panic!("hosted call surfaced as {calls:?}"),
            LlmStreamEvent::Error(error) => panic!("stream error: {error:?}"),
            LlmStreamEvent::Done(meta) => done = Some(meta),
            _ => {}
        }
    }
    let done = done.expect("stream completes");
    eprintln!(
        "answer: {text}\nhosted: {hosted:?}\ncounts: {:?}",
        done.hosted_tool_calls
    );

    assert!(!text.trim().is_empty(), "model should answer");
    assert!(
        hosted
            .iter()
            .any(|call| call.tool == "web_search" && call.status == HostedToolCallStatus::Completed),
        "expected a completed web search call, got {hosted:?}"
    );
    assert!(
        done.hosted_tool_calls
            .get("web_search_call")
            .copied()
            .unwrap_or(0)
            >= 1
    );
}
