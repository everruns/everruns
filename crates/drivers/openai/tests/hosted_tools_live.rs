#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live tests for OpenAI hosted tools through the Responses driver (EVE-1115)
//! against the real OpenAI API.
//!
//! Proves OpenAI accepts each rendered hosted tool, runs it inside the
//! response, and that the stream reports the call as hosted-call progress plus
//! a per-call count in the completion metadata, never as an agent tool call.
//!
//! Ignored by default (requires network + `OPENAI_API_KEY`; file search also
//! needs `OPENAI_TEST_VECTOR_STORE_ID`, a vector store holding any document);
//! run manually:
//!   `doppler run -- cargo test -p everruns-openai --test hosted_tools_live -- --ignored --nocapture`

use everruns_openai::provider;
use everruns_provider::driver_registry::{
    HostedToolCallStatus, LlmCallConfig, LlmStreamEvent, Message, MessageRole,
};
use everruns_provider::model::ReasoningEffort;
use everruns_provider::openai_hosted_tools::{
    ContainerTool, FileSearchTool, OpenAiHostedTools, SearchContextSize, WebSearchTool,
};
use futures::StreamExt;

const LIVE_MODEL: &str = "gpt-5.6-luna";

/// Stream one prompt with `tools` and check the named hosted call completed.
async fn assert_hosted_call(tools: OpenAiHostedTools, prompt: &str, tool: &str, item: &str) {
    let api_key =
        std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set for the live test");
    let mut config = LlmCallConfig::new(LIVE_MODEL);
    config.reasoning_effort = Some(ReasoningEffort::Low);
    let (key, value) = tools.to_driver_option().unwrap();
    config.driver_options.insert(key, value);

    let messages = vec![Message::text(MessageRole::User, prompt)];
    let mut stream = provider("openai", api_key)
        .chat_completion_stream(messages, &config)
        .await
        .expect("OpenAI should accept the hosted tool");

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
            .any(|call| call.tool == tool && call.status == HostedToolCallStatus::Completed),
        "expected a completed {tool} call, got {hosted:?}"
    );
    assert!(done.hosted_tool_calls.get(item).copied().unwrap_or(0) >= 1);
}

#[tokio::test]
#[ignore = "live network + OPENAI_API_KEY"]
async fn openai_runs_hosted_web_search() {
    let tools = OpenAiHostedTools {
        web_search: Some(WebSearchTool {
            search_context_size: Some(SearchContextSize::Low),
            ..Default::default()
        }),
        ..Default::default()
    };
    let prompt = "Search the web: what is the latest stable Rust release? Answer in one line.";
    assert_hosted_call(tools, prompt, "web_search", "web_search_call").await;
}

#[tokio::test]
#[ignore = "live network + OPENAI_API_KEY"]
async fn openai_runs_hosted_code_interpreter() {
    let tools = OpenAiHostedTools {
        code_interpreter: Some(ContainerTool::default()),
        ..Default::default()
    };
    let prompt = "Use python to compute 2**100. Reply with the number only.";
    assert_hosted_call(tools, prompt, "code_interpreter", "code_interpreter_call").await;
}

#[tokio::test]
#[ignore = "live network + OPENAI_API_KEY"]
async fn openai_runs_hosted_shell() {
    let tools = OpenAiHostedTools {
        shell: Some(ContainerTool::default()),
        ..Default::default()
    };
    let prompt = "Run `uname -s` in the shell and reply with its output only.";
    assert_hosted_call(tools, prompt, "shell", "shell_call").await;
}

#[tokio::test]
#[ignore = "live network + OPENAI_API_KEY + OPENAI_TEST_VECTOR_STORE_ID"]
async fn openai_runs_hosted_file_search() {
    let Ok(store) = std::env::var("OPENAI_TEST_VECTOR_STORE_ID") else {
        eprintln!("skipping: OPENAI_TEST_VECTOR_STORE_ID is not set");
        return;
    };
    let tools = OpenAiHostedTools {
        file_search: Some(FileSearchTool {
            vector_store_ids: vec![store],
            max_num_results: Some(3),
        }),
        ..Default::default()
    };
    let prompt = "Search the files: what does the document say? Answer in one line.";
    assert_hosted_call(tools, prompt, "file_search", "file_search_call").await;
}
