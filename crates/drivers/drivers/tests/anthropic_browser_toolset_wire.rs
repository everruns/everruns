#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Wire tests for Anthropic's native browser toolset (EVE-1133).
//
// Request: the provider-neutral `everruns/browser_use` option swaps the
// `browser` function tool for `browser_toolset_20260801` on models that take
// it, and only there. Stream: member `tool_use` blocks come back as calls of
// the `browser` tool. Replay: `browser` calls go back as member calls and
// their results carry `toolset_name` and a `browser_state` block.

use everruns_contracts::driver_registry::{
    LlmCallConfig, LlmContentPart, LlmStreamEvent, Message, MessageContent, MessageRole,
};
use everruns_contracts::native_computer::{NativeBrowserUse, NativeComputerUse};
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::{Provider, StaticHeaderAuth, ToolDefinition};
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

// `Connection: close` keeps every request on a fresh connection. The driver's
// HTTP client is process-wide, so a keep-alive connection outlives the test
// that opened it, and its I/O task runs on that test's tokio runtime. wiremock
// recycles pooled servers (same port) as soon as one is dropped, so another
// test could pick up that parked connection; if the owning runtime stopped
// mid-response, the stream failed before its first event and the driver's
// reconnect sent a second request (CI flake: `left: 2, right: 1`).
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

fn config(model: &str, native: bool) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.tools = vec![
        ToolDefinition::function("browser", "operate a browser", json!({ "type": "object" })),
        ToolDefinition::function("web_fetch", "fetch a page", json!({ "type": "object" })),
    ];
    if native {
        let (key, value) = NativeBrowserUse::default().to_driver_option();
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
    vec![Message::text(MessageRole::User, "find the pricing page")]
}

#[tokio::test]
async fn toolset_replaces_the_browser_function_tool() {
    let server = MockServer::start().await;
    mount(&server, end_turn()).await;

    send(&server, ask(), &config("claude-opus-5-5", true)).await;

    let body = sent_body(&server).await;
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0], json!({ "type": "browser_toolset_20260801" }));
    assert_eq!(tools[1]["name"], "web_fetch");
    assert!(!body.to_string().contains("everruns/browser_use"));
    assert_eq!(
        body["tool_choice"]["disable_parallel_tool_use"],
        json!(true)
    );
}

#[tokio::test]
async fn function_tool_stays_without_the_option_or_a_toolset_model() {
    for (model, native) in [("claude-opus-5-5", false), ("claude-haiku-4-5", true)] {
        let server = MockServer::start().await;
        mount(&server, end_turn()).await;

        send(&server, ask(), &config(model, native)).await;

        let body = sent_body(&server).await;
        let names: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["browser", "web_fetch"], "{model} native={native}");
    }
}

#[tokio::test]
async fn both_toolsets_go_side_by_side() {
    let server = MockServer::start().await;
    mount(&server, end_turn()).await;
    let mut request = config("claude-opus-5-5", true);
    request.tools.push(ToolDefinition::function(
        "computer",
        "operate a display",
        json!({ "type": "object" }),
    ));
    let (key, value) = NativeComputerUse {
        display_width: 1280,
        display_height: 800,
    }
    .to_driver_option();
    request.driver_options.insert(key, value);

    send(&server, ask(), &request).await;

    let body = sent_body(&server).await;
    let types: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["type"].as_str())
        .collect();
    assert!(types.contains(&"browser_toolset_20260801"), "{types:?}");
    assert!(types.contains(&"computer_toolset_20260801"), "{types:?}");
}

#[tokio::test]
async fn member_calls_stream_as_browser_calls() {
    let server = MockServer::start().await;
    let body = [
        sse_event(
            "message_start",
            json!({ "type": "message_start", "message": { "id": "msg_2", "usage": { "input_tokens": 12 } } }),
        ),
        sse_event(
            "content_block_start",
            json!({ "type": "content_block_start", "index": 0, "content_block": {
                "type": "tool_use", "id": "toolu_1", "name": "navigate",
                "toolset_name": "browser", "input": {} } }),
        ),
        sse_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 0, "delta": {
                "type": "input_json_delta", "partial_json": "{\"url\": \"example.com/pricing\"}" } }),
        ),
        sse_event("content_block_stop", json!({ "type": "content_block_stop", "index": 0 })),
        sse_event(
            "content_block_start",
            json!({ "type": "content_block_start", "index": 1, "content_block": {
                "type": "tool_use", "id": "toolu_2", "name": "find",
                "toolset_name": "browser", "input": {} } }),
        ),
        sse_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 1, "delta": {
                "type": "input_json_delta", "partial_json": "{\"query\": \"plans table\"}" } }),
        ),
        sse_event("content_block_stop", json!({ "type": "content_block_stop", "index": 1 })),
        sse_event(
            "message_delta",
            json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 9 } }),
        ),
        sse_event("message_stop", json!({ "type": "message_stop" })),
    ]
    .concat();
    mount(&server, body).await;

    let events = send(&server, ask(), &config("claude-opus-5-5", true)).await;

    let calls: Vec<ToolCall> = events
        .iter()
        .find_map(|event| match event {
            LlmStreamEvent::ToolCalls(calls) => Some(calls.clone()),
            _ => None,
        })
        .expect("member calls surface as tool calls");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].name, "browser");
    assert_eq!(
        calls[0].arguments,
        json!({ "action": "navigate", "url": "example.com/pricing",
                "native_batch": { "id": "toolu_1" } })
    );
    assert_eq!(
        calls[1].arguments,
        json!({ "action": "find", "query": "plans table", "native_batch": { "id": "toolu_1" } })
    );
}

#[tokio::test]
async fn browser_calls_replay_as_member_calls_with_state_blocks() {
    let server = MockServer::start().await;
    mount(&server, end_turn()).await;
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.tool_calls = Some(vec![ToolCall {
        id: "toolu_1".into(),
        name: "browser".into(),
        arguments: json!({ "action": "screenshot", "native_batch": { "id": "toolu_1" } }),
    }]);
    let tool_json = json!({
        "status": "ok", "action": "screenshot", "text": "",
        "browser_state": { "tabs": [{ "tab_id": "T1", "title": "Pricing",
                                      "url": "https://example.com/pricing", "active": true }] }
    });
    let mut result = Message::text(MessageRole::Tool, "");
    result.content = MessageContent::Parts(vec![
        LlmContentPart::Text {
            text: tool_json.to_string(),
        },
        LlmContentPart::Image {
            url: "data:image/png;base64,RlJBTUU=".into(),
        },
    ]);
    result.tool_call_id = Some("toolu_1".into());

    let messages = vec![ask().remove(0), assistant, result];
    send(&server, messages, &config("claude-opus-5-5", true)).await;

    let body = sent_body(&server).await;
    let blocks: Vec<Value> = body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .collect();
    let call = blocks
        .iter()
        .find(|block| block["type"] == "tool_use")
        .unwrap();
    assert_eq!(
        *call,
        json!({ "type": "tool_use", "id": "toolu_1", "name": "screenshot",
                "toolset_name": "browser", "input": {} })
    );
    let result = blocks
        .iter()
        .find(|block| block["type"] == "tool_result")
        .unwrap();
    assert_eq!(result["toolset_name"], "browser");
    let content = result["content"].as_array().unwrap();
    assert_eq!(content[0]["type"], "text");
    assert_eq!(content[1]["type"], "image");
    assert_eq!(
        content[2],
        json!({ "type": "browser_state", "tabs": [{ "tab_id": "T1", "title": "Pricing",
                "url": "https://example.com/pricing", "active": true }] })
    );
}
