#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Wire tests for native web search on the Anthropic driver.
//
// Request: the provider-neutral `everruns/web_search` option swaps the
// `web_search` function tool for Anthropic's `web_search_20250305` server tool
// on models that have it, when Anthropic can apply the domain filters, and
// only then. Stream: `server_tool_use` / `web_search_tool_result` blocks are
// hosted calls, not client tool calls, and billed searches are counted.
// Replay: the raw blocks go back verbatim (encrypted content and citation
// indexes included), and are stripped when the request has no server tool.

use everruns_contracts::driver_registry::{
    HostedToolCallStatus, LlmCallConfig, LlmContentPart, LlmStreamEvent, Message, MessageContent,
    MessageRole,
};
use everruns_contracts::native_web_search::NativeWebSearch;
use everruns_contracts::{Provider, ProviderOpaqueContent, StaticHeaderAuth, ToolDefinition};
use everruns_drivers::anthropic::AnthropicChatDriver;
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn sse_event(event: &str, data: Value) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn driver(server: &MockServer) -> Provider {
    Provider::new("anthropic-test", AnthropicChatDriver::new())
        .base_url(format!("{}/v1/messages", server.uri()))
        .auth(StaticHeaderAuth::new("x-api-key", "test-key"))
}

// `Connection: close`: see `anthropic_computer_toolset_wire.rs`.
async fn mount(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("connection", "close")
                .set_body_raw(body, "text/event-stream"),
        )
        .mount(server)
        .await;
}

fn end_turn() -> String {
    [
        sse_event(
            "message_start",
            json!({ "type": "message_start", "message": { "id": "msg_1", "usage": { "input_tokens": 5 } } }),
        ),
        sse_event(
            "message_delta",
            json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 1 } }),
        ),
        sse_event("message_stop", json!({ "type": "message_stop" })),
    ]
    .concat()
}

fn config(model: &str, native: Option<NativeWebSearch>) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.tools = vec![
        ToolDefinition::function("web_search", "search the web", json!({ "type": "object" })),
        ToolDefinition::function("web_fetch", "fetch a page", json!({ "type": "object" })),
    ];
    if let Some(native) = native {
        let (key, value) = native.to_driver_option();
        config.driver_options.insert(key, value);
    }
    config
}

async fn send(
    server: &MockServer,
    messages: Vec<Message>,
    config: &LlmCallConfig,
) -> Vec<LlmStreamEvent> {
    let mut stream = driver(server)
        .chat_completion_stream(messages, config)
        .await
        .expect("stream should start");
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.expect("no transport error"));
    }
    events
}

async fn sent_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    requests[0].body_json().unwrap()
}

fn ask() -> Vec<Message> {
    vec![Message::text(MessageRole::User, "what is new in Rust?")]
}

fn tool_names(body: &Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn server_tool_replaces_the_web_search_function_tool() {
    let server = MockServer::start().await;
    mount(&server, end_turn()).await;
    let native = NativeWebSearch {
        blocked_domains: vec!["pinterest.com".into()],
        ..Default::default()
    };

    send(&server, ask(), &config("claude-haiku-5-5", Some(native))).await;

    let body = sent_body(&server).await;
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(
        tools[0],
        json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": 5,
                "blocked_domains": ["pinterest.com"] })
    );
    assert!(tools[0].get("input_schema").is_none());
    assert_eq!(tools[1]["name"], "web_fetch");
    assert!(!body.to_string().contains("everruns/web_search"));
}

#[tokio::test]
async fn function_tool_stays_when_anthropic_cannot_honor_the_request() {
    let both = NativeWebSearch {
        allowed_domains: vec!["docs.rs".into()],
        blocked_domains: vec!["pinterest.com".into()],
        ..Default::default()
    };
    for (model, native) in [
        // No request.
        ("claude-haiku-5-5", None),
        // Allowed and blocked domains together: Anthropic takes one kind.
        ("claude-haiku-5-5", Some(both)),
        // A model without web search.
        (
            "claude-3-5-sonnet-20241022",
            Some(NativeWebSearch::default()),
        ),
    ] {
        let server = MockServer::start().await;
        mount(&server, end_turn()).await;

        send(&server, ask(), &config(model, native.clone())).await;

        let body = sent_body(&server).await;
        assert_eq!(tool_names(&body), ["web_search", "web_fetch"], "{model}");
        assert!(body["tools"][0].get("input_schema").is_some(), "{model}");
    }
}

fn search_response() -> String {
    [
        sse_event(
            "message_start",
            json!({ "type": "message_start", "message": { "id": "msg_2", "usage": {
                "input_tokens": 40, "server_tool_use": { "web_search_requests": 0 } } } }),
        ),
        sse_event(
            "content_block_start",
            json!({ "type": "content_block_start", "index": 0, "content_block": {
                "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search" } }),
        ),
        sse_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 0, "delta": {
                "type": "input_json_delta", "partial_json": "{\"query\":\"rust 1.95 release\"}" } }),
        ),
        sse_event(
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": 0 }),
        ),
        sse_event(
            "content_block_start",
            json!({ "type": "content_block_start", "index": 1, "content_block": {
                "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [
                    { "type": "web_search_result", "url": "https://blog.rust-lang.org/",
                      "title": "Rust Blog", "encrypted_content": "EqgfCioIARgB", "page_age": "1 day" } ] } }),
        ),
        sse_event(
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": 1 }),
        ),
        sse_event(
            "content_block_start",
            json!({ "type": "content_block_start", "index": 2, "content_block": {
                "type": "text", "text": "", "citations": [] } }),
        ),
        sse_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 2, "delta": {
                "type": "citations_delta", "citation": {
                    "type": "web_search_result_location", "url": "https://blog.rust-lang.org/",
                    "title": "Rust Blog", "encrypted_index": "Eo8BCioI", "cited_text": "Rust 1.95" } } }),
        ),
        sse_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 2, "delta": {
                "type": "text_delta", "text": "Rust 1.95 is out." } }),
        ),
        sse_event(
            "content_block_stop",
            json!({ "type": "content_block_stop", "index": 2 }),
        ),
        sse_event(
            "message_delta",
            json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" },
                    "usage": { "output_tokens": 30, "server_tool_use": { "web_search_requests": 1 } } }),
        ),
        sse_event("message_stop", json!({ "type": "message_stop" })),
    ]
    .concat()
}

#[tokio::test]
async fn searches_stream_as_hosted_calls_and_are_counted() {
    let server = MockServer::start().await;
    mount(&server, search_response()).await;

    let events = send(
        &server,
        ask(),
        &config("claude-haiku-5-5", Some(NativeWebSearch::default())),
    )
    .await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, LlmStreamEvent::ToolCalls(_))),
        "a server search is never a client tool call"
    );
    let calls: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            LlmStreamEvent::HostedToolCall(call) => Some(call.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|call| call.id == "srvtoolu_1"));
    assert!(calls.iter().all(|call| call.tool == "web_search"));
    assert_eq!(calls[0].status, HostedToolCallStatus::InProgress);
    assert_eq!(calls[0].summary, None);
    assert_eq!(calls[1].status, HostedToolCallStatus::InProgress);
    assert_eq!(calls[1].summary.as_deref(), Some("rust 1.95 release"));
    assert_eq!(calls[2].status, HostedToolCallStatus::Completed);
    assert_eq!(calls[2].summary.as_deref(), Some("rust 1.95 release"));

    let text: String = events
        .iter()
        .filter_map(|event| match event {
            LlmStreamEvent::TextDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Rust 1.95 is out.");

    let Some(LlmStreamEvent::Done(metadata)) = events.last() else {
        panic!("expected done");
    };
    assert_eq!(metadata.finish_reason.as_deref(), Some("stop"));
    assert_eq!(metadata.hosted_tool_calls.get("web_search_call"), Some(&1));
    // The raw blocks the next turn replays: query, encrypted results, and the
    // citation with its encrypted index.
    let content = &metadata.provider_opaque_content.as_ref().unwrap().content;
    assert_eq!(
        content[0],
        json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search",
                "input": { "query": "rust 1.95 release" } })
    );
    assert_eq!(
        content[1]["content"][0]["encrypted_content"],
        "EqgfCioIARgB"
    );
    assert_eq!(content[2]["text"], "Rust 1.95 is out.");
    assert_eq!(content[2]["citations"][0]["encrypted_index"], "Eo8BCioI");
}

#[tokio::test]
async fn cited_text_blocks_become_web_citations_in_streamed_chars() {
    // Text before the search and the cited block after it, both non-ASCII: the
    // citation spans the cited block in chars of the concatenated answer.
    let block = |index: u32, block: Value| {
        sse_event(
            "content_block_start",
            json!({ "type": "content_block_start", "index": index, "content_block": block }),
        )
    };
    let delta = |index: u32, delta: Value| {
        sse_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": index, "delta": delta }),
        )
    };
    let body = [
        sse_event(
            "message_start",
            json!({ "type": "message_start", "message": { "id": "msg_3", "usage": { "input_tokens": 9 } } }),
        ),
        block(0, json!({ "type": "text", "text": "" })),
        delta(0, json!({ "type": "text_delta", "text": "Grüße 😀! " })),
        block(1, json!({ "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search",
                         "input": { "query": "rust" } })),
        block(2, json!({ "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [
            { "type": "web_search_result", "url": "https://blog.rust-lang.org/", "title": "Rust Blog",
              "encrypted_content": "E" } ] })),
        block(3, json!({ "type": "text", "text": "", "citations": [] })),
        delta(3, json!({ "type": "citations_delta", "citation": {
            "type": "web_search_result_location", "url": "https://blog.rust-lang.org/",
            "title": "Rust Blog", "encrypted_index": "Eo8B", "cited_text": "Rust 1.95" } })),
        delta(3, json!({ "type": "text_delta", "text": "Rust 1.95 " })),
        delta(3, json!({ "type": "text_delta", "text": "ist da 🎉." })),
        sse_event(
            "message_delta",
            json!({ "type": "message_delta", "delta": { "stop_reason": "end_turn" }, "usage": { "output_tokens": 9 } }),
        ),
        sse_event("message_stop", json!({ "type": "message_stop" })),
    ]
    .concat();
    let server = MockServer::builder().start().await;
    mount(&server, body).await;

    let events = send(
        &server,
        ask(),
        &config("claude-haiku-5-5", Some(NativeWebSearch::default())),
    )
    .await;
    let text: String = events
        .iter()
        .filter_map(|event| match event {
            LlmStreamEvent::TextDelta(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Grüße 😀! Rust 1.95 ist da 🎉.");
    let Some(LlmStreamEvent::Done(metadata)) = events.last() else {
        panic!("expected done");
    };
    let [citation] = metadata.web_citations.as_slice() else {
        panic!("expected one citation: {:?}", metadata.web_citations);
    };
    assert_eq!(citation.url, "https://blog.rust-lang.org/");
    assert_eq!(citation.title.as_deref(), Some("Rust Blog"));
    assert_eq!(citation.snippet.as_deref(), Some("Rust 1.95"));
    let span = citation.span.as_ref().unwrap();
    let covered: String = text
        .chars()
        .skip(span.start)
        .take(span.end - span.start)
        .collect();
    assert_eq!(covered, "Rust 1.95 ist da 🎉.");
    assert_eq!(span.text, covered);
    assert_eq!(span.start, 9);
}

fn searched_history() -> Vec<Message> {
    let preserved = json!([
        { "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search",
          "input": { "query": "rust 1.95 release" } },
        { "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [
            { "type": "web_search_result", "url": "https://blog.rust-lang.org/",
              "title": "Rust Blog", "encrypted_content": "EqgfCioIARgB" } ] },
        { "type": "text", "text": "Rust 1.95 is out.", "citations": [
            { "type": "web_search_result_location", "url": "https://blog.rust-lang.org/",
              "title": "Rust Blog", "encrypted_index": "Eo8BCioI", "cited_text": "Rust 1.95" } ] }
    ]);
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.content = MessageContent::Parts(vec![
        LlmContentPart::Text {
            text: "Rust 1.95 is out.".into(),
        },
        LlmContentPart::ProviderOpaque(ProviderOpaqueContent::new("anthropic", preserved)),
    ]);
    vec![
        ask().remove(0),
        assistant,
        Message::text(MessageRole::User, "and before that?"),
    ]
}

#[tokio::test]
async fn search_blocks_replay_verbatim_while_the_server_tool_is_on() {
    let server = MockServer::start().await;
    mount(&server, end_turn()).await;

    send(
        &server,
        searched_history(),
        &config("claude-haiku-5-5", Some(NativeWebSearch::default())),
    )
    .await;

    let body = sent_body(&server).await;
    let assistant = &body["messages"][1];
    assert_eq!(assistant["role"], "assistant");
    let blocks = assistant["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0]["type"], "server_tool_use");
    assert_eq!(blocks[1]["content"][0]["encrypted_content"], "EqgfCioIARgB");
    assert_eq!(blocks[2]["citations"][0]["encrypted_index"], "Eo8BCioI");
}

#[tokio::test]
async fn search_blocks_are_stripped_once_the_server_tool_is_off() {
    let server = MockServer::start().await;
    mount(&server, end_turn()).await;

    // The plan moved off native search: the function tool is back.
    send(
        &server,
        searched_history(),
        &config("claude-haiku-5-5", None),
    )
    .await;

    let body = sent_body(&server).await;
    assert_eq!(tool_names(&body), ["web_search", "web_fetch"]);
    assert_eq!(
        body["messages"][1]["content"],
        json!([{ "type": "text", "text": "Rust 1.95 is out." }])
    );
}
