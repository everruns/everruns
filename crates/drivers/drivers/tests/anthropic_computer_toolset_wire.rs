#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Wire tests for Anthropic's native computer toolset (EVE-1133).
//
// Request: the provider-neutral `everruns/computer_use` option swaps the
// `computer` function tool for `computer_toolset_20260801` on models that take
// it, and only there. Stream: member `tool_use` blocks come back as calls of
// the `computer` tool. Replay: `computer` calls go back as member calls and
// their results echo `toolset_name`.

use everruns_drivers::anthropic::AnthropicChatDriver;
use everruns_provider::driver_registry::{
    LlmCallConfig, LlmContentPart, LlmStreamEvent, Message, MessageContent, MessageRole,
};
use everruns_provider::native_computer::NativeComputerUse;
use everruns_provider::tool_types::ToolCall;
use everruns_provider::{Provider, StaticHeaderAuth, ToolDefinition};
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

async fn mount(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
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
        ToolDefinition::function("computer", "operate a display", json!({ "type": "object" })),
        ToolDefinition::function("web_fetch", "fetch a page", json!({ "type": "object" })),
    ];
    if native {
        let (key, value) = NativeComputerUse {
            display_width: 1280,
            display_height: 800,
        }
        .to_driver_option();
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
    vec![Message::text(MessageRole::User, "fill the form")]
}

#[tokio::test]
async fn toolset_replaces_the_computer_function_tool() {
    let server = MockServer::start().await;
    mount(&server, end_turn()).await;

    send(&server, ask(), &config("claude-opus-5-5", true)).await;

    let body = sent_body(&server).await;
    let tools = body["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0]["type"], "computer_toolset_20260801");
    assert!(tools[0].get("name").is_none());
    assert_eq!(tools[0]["configs"]["zoom"]["enabled"], false);
    assert_eq!(tools[1]["name"], "web_fetch");
    assert!(!body.to_string().contains("everruns/computer_use"));
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
        assert_eq!(names, ["computer", "web_fetch"], "{model} native={native}");
    }
}

#[tokio::test]
async fn member_calls_stream_as_computer_calls() {
    let server = MockServer::start().await;
    let body = [
        sse_event(
            "message_start",
            json!({ "type": "message_start", "message": { "id": "msg_2", "usage": { "input_tokens": 12 } } }),
        ),
        sse_event(
            "content_block_start",
            json!({ "type": "content_block_start", "index": 0, "content_block": {
                "type": "tool_use", "id": "toolu_1", "name": "left_click",
                "toolset_name": "computer", "input": {} } }),
        ),
        sse_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 0, "delta": {
                "type": "input_json_delta", "partial_json": "{\"coordinate\": [120, 48]}" } }),
        ),
        sse_event("content_block_stop", json!({ "type": "content_block_stop", "index": 0 })),
        sse_event(
            "content_block_start",
            json!({ "type": "content_block_start", "index": 1, "content_block": {
                "type": "tool_use", "id": "toolu_2", "name": "type",
                "toolset_name": "computer", "input": {} } }),
        ),
        sse_event(
            "content_block_delta",
            json!({ "type": "content_block_delta", "index": 1, "delta": {
                "type": "input_json_delta", "partial_json": "{\"text\": \"Ada\"}" } }),
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
    assert_eq!(calls[0].name, "computer");
    assert_eq!(
        calls[0].arguments,
        json!({ "action": "left_click", "coordinate": [120, 48] })
    );
    assert_eq!(calls[1].name, "computer");
    assert_eq!(
        calls[1].arguments,
        json!({ "action": "type", "text": "Ada" })
    );
}

#[tokio::test]
async fn computer_calls_replay_as_member_calls_with_tagged_results() {
    let server = MockServer::start().await;
    mount(&server, end_turn()).await;
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.tool_calls = Some(vec![ToolCall {
        id: "toolu_1".into(),
        name: "computer".into(),
        arguments: json!({ "action": "left_click", "coordinate": [120, 48] }),
    }]);
    let mut result = Message::text(MessageRole::Tool, "");
    result.content = MessageContent::Parts(vec![
        LlmContentPart::Text {
            text: r#"{"status":"ok"}"#.into(),
        },
        LlmContentPart::Image {
            url: "data:image/png;base64,RlJBTUU=".into(),
        },
    ]);
    result.tool_call_id = Some("toolu_1".into());

    let messages = vec![ask().remove(0), assistant, result];
    send(&server, messages, &config("claude-opus-5-5", true)).await;

    let body = sent_body(&server).await;
    let messages = body["messages"].as_array().unwrap();
    let call = messages
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .find(|block| block["type"] == "tool_use")
        .unwrap();
    assert_eq!(
        call,
        json!({ "type": "tool_use", "id": "toolu_1", "name": "left_click",
                "toolset_name": "computer", "input": { "coordinate": [120, 48] } })
    );
    let result = messages
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .find(|block| block["type"] == "tool_result")
        .unwrap();
    assert_eq!(result["toolset_name"], "computer");
    assert_eq!(result["content"][1]["type"], "image");
}

#[tokio::test]
async fn computer_toolset_disables_provider_side_parallel_tool_use() {
    let server = MockServer::start().await;
    mount(&server, end_turn()).await;
    let mut request = config("claude-opus-5-5", true);
    request.parallel_tool_calls = Some(true);
    send(&server, ask(), &request).await;

    let body = sent_body(&server).await;
    assert_eq!(
        body["tool_choice"]["disable_parallel_tool_use"],
        json!(true),
        "computer member actions are stateful; a failed action must not be followed by more in the same turn"
    );
}
