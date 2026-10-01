// Wire tests for the native OpenAI `computer` tool on the Open Responses
// driver (EVE-1133).
//
// Request: the provider-neutral `everruns/computer_use` option swaps the
// `computer` function tool for `{"type": "computer"}`, only on an endpoint with
// OpenAI's hosted tools and a model that has the GA computer tool; anything
// else keeps the function tool and does not fail. Stream: a `computer_call`
// surfaces as a call of the `computer` tool (client-executed), never as a
// hosted-call event. Replay: the call and its screenshot go back as
// `computer_call` / `computer_call_output`.

use everruns_provider::driver_registry::{
    LlmCallConfig, LlmContentPart, LlmStreamEvent, Message, MessageContent, MessageRole,
};
use everruns_provider::native_computer::NativeComputerUse;
use everruns_provider::tool_types::ToolCall;
use everruns_provider::{OpenResponsesProtocolChatDriver, Provider, ToolDefinition};
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn driver(server: &MockServer, hosted_tools: bool) -> Provider {
    Provider::new(
        "openai",
        OpenResponsesProtocolChatDriver::new()
            .with_native_features(true, true)
            .with_hosted_tools(hosted_tools),
    )
    .base_url(format!("{}/v1", server.uri()))
}

fn function(name: &str) -> ToolDefinition {
    ToolDefinition::function(
        name,
        "a tool",
        json!({ "type": "object", "properties": { "action": { "type": "string" } } }),
    )
}

fn config(model: &str, native: bool) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.tools = vec![function("computer"), function("web_fetch")];
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

fn sse(frames: &[Value]) -> String {
    frames
        .iter()
        .map(|frame| format!("data: {frame}\n\n"))
        .collect()
}

fn completed_only() -> String {
    sse(&[json!({
        "type": "response.completed",
        "response": { "id": "resp_1", "status": "completed", "output": [],
                      "usage": { "input_tokens": 5, "output_tokens": 1, "total_tokens": 6 } }
    })])
}

async fn mount(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

async fn sent_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    requests[0].body_json().unwrap()
}

async fn drain(
    provider: &Provider,
    messages: Vec<Message>,
    config: &LlmCallConfig,
) -> Vec<LlmStreamEvent> {
    let mut stream = provider
        .chat_completion_stream(messages, config)
        .await
        .expect("request accepted");
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.expect("no transport error"));
    }
    events
}

fn ask() -> Vec<Message> {
    vec![Message::text(MessageRole::User, "fill the form")]
}

fn tool_shapes(body: &Value) -> Vec<String> {
    body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| match tool["name"].as_str() {
            Some(name) => format!("{}:{name}", tool["type"].as_str().unwrap()),
            None => tool["type"].as_str().unwrap().to_string(),
        })
        .collect()
}

#[tokio::test]
async fn native_computer_replaces_the_function_tool() {
    let server = MockServer::start().await;
    mount(&server, completed_only()).await;

    drain(&driver(&server, true), ask(), &config("gpt-6.1-sol", true)).await;

    let body = sent_body(&server).await;
    assert_eq!(tool_shapes(&body), ["function:web_fetch", "computer"]);
    // The option is Everruns-internal and never reaches the wire.
    assert!(!body.to_string().contains("everruns/computer_use"));
}

#[tokio::test]
async fn function_tool_stays_without_the_option_a_capable_model_or_hosted_tools() {
    for (model, native, hosted) in [
        ("gpt-6.1-sol", false, true),
        ("gpt-4.1", true, true),
        ("gpt-6.1-sol", true, false),
    ] {
        let server = MockServer::start().await;
        mount(&server, completed_only()).await;

        // A gateway without hosted tools must not fail: the function tool works.
        drain(&driver(&server, hosted), ask(), &config(model, native)).await;

        let body = sent_body(&server).await;
        assert_eq!(
            tool_shapes(&body),
            ["function:computer", "function:web_fetch"],
            "{model} native={native} hosted={hosted}"
        );
    }
}

#[tokio::test]
async fn computer_call_is_a_client_tool_call_not_a_hosted_call() {
    let server = MockServer::start().await;
    let item = json!({
        "type": "computer_call", "id": "cu_1", "call_id": "call_1", "status": "completed",
        "actions": [
            { "type": "click", "button": "left", "x": 120, "y": 48 },
            { "type": "type", "text": "Ada" }
        ]
    });
    mount(
        &server,
        sse(&[
            json!({ "type": "response.output_item.added", "sequence_number": 1, "output_index": 0,
                    "item": { "type": "computer_call", "id": "cu_1", "call_id": "call_1",
                              "status": "in_progress", "actions": [] } }),
            json!({ "type": "response.output_item.done", "sequence_number": 2, "output_index": 0, "item": item }),
            json!({ "type": "response.completed", "sequence_number": 3, "response": {
                "id": "resp_cu", "object": "response", "created_at": 1, "status": "completed",
                "model": "gpt-6.1-sol", "output": [item],
                "usage": { "input_tokens": 50, "output_tokens": 9, "total_tokens": 59 } } }),
        ]),
    )
    .await;

    let events = drain(&driver(&server, true), ask(), &config("gpt-6.1-sol", true)).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event, LlmStreamEvent::HostedToolCall(_))),
        "a computer call is client-executed, never a hosted call"
    );
    let calls: Vec<ToolCall> = events
        .iter()
        .rev()
        .find_map(|event| match event {
            LlmStreamEvent::ToolCalls(calls) => Some(calls.clone()),
            _ => None,
        })
        .expect("the computer call surfaces as a tool call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "computer");
    assert_eq!(
        calls[0].arguments,
        json!({ "actions": [
            { "action": "left_click", "coordinate": [120, 48] },
            { "action": "type", "text": "Ada" }
        ]})
    );
    let done = events.iter().find_map(|event| match event {
        LlmStreamEvent::Done(meta) => Some(meta),
        _ => None,
    });
    let done = done.expect("stream completes");
    assert_eq!(done.finish_reason.as_deref(), Some("tool_calls"));
    assert!(done.hosted_tool_calls.is_empty());
}

#[tokio::test]
async fn computer_results_replay_as_computer_items_with_the_screenshot() {
    let server = MockServer::start().await;
    mount(&server, completed_only()).await;
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.tool_calls = Some(vec![ToolCall {
        id: "call_1".into(),
        name: "computer".into(),
        arguments: json!({ "actions": [{ "action": "left_click", "coordinate": [120, 48] }] }),
    }]);
    let mut result = Message::text(MessageRole::Tool, "");
    result.content = MessageContent::Parts(vec![
        LlmContentPart::Text {
            text: r#"{"status":"ok"}"#.into(),
        },
        LlmContentPart::Image {
            url: "data:image/png;base64,FRAME".into(),
        },
    ]);
    result.tool_call_id = Some("call_1".into());

    let messages = vec![ask().remove(0), assistant, result];
    drain(
        &driver(&server, true),
        messages,
        &config("gpt-6.1-sol", true),
    )
    .await;

    let input = sent_body(&server).await["input"].clone();
    let items = input.as_array().unwrap();
    assert!(items.iter().any(|item| item
        == &json!({ "type": "computer_call", "call_id": "call_1", "status": "completed",
                    "actions": [{ "type": "click", "button": "left", "x": 120, "y": 48 }] })));
    assert!(items.iter().any(|item| item
        == &json!({ "type": "computer_call_output", "call_id": "call_1",
                    "output": { "type": "computer_screenshot",
                                "image_url": "data:image/png;base64,FRAME", "detail": "original" } })));
    assert!(
        !items
            .iter()
            .any(|item| item["type"] == "function_call" || item["type"] == "function_call_output")
    );
}
