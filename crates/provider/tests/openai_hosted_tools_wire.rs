// Wire tests for OpenAI hosted tools on the Open Responses driver (EVE-1115).
//
// Request: the `openai/hosted_tools` driver option becomes `tools` entries
// next to (or instead of) function tools, and an endpoint without hosted
// tools refuses the option rather than dropping it. Stream: a web search
// response (`web_search_call` items, `response.web_search_call.*` events, url
// citations) streams as answer text plus hosted-call progress events, never
// as an agent tool call or a parse error.

use everruns_provider::driver_registry::{
    HostedToolCall, HostedToolCallStatus, LlmCallConfig, LlmStreamEvent, Message, MessageRole,
};
use everruns_provider::openai_hosted_tools::{OpenAiHostedTools, SearchContextSize, WebSearchTool};
use everruns_provider::{OpenResponsesProtocolChatDriver, Provider, ToolDefinition};
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn hosted_driver(server: &MockServer, hosted_tools: bool) -> Provider {
    Provider::new(
        "openai",
        OpenResponsesProtocolChatDriver::new()
            .with_native_features(true, true)
            .with_hosted_tools(hosted_tools),
    )
    .base_url(format!("{}/v1", server.uri()))
}

fn web_search_config(model: &str) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    let tools = OpenAiHostedTools {
        web_search: Some(WebSearchTool {
            search_context_size: Some(SearchContextSize::Low),
            ..Default::default()
        }),
    };
    let (key, value) = tools.to_driver_option().expect("web search selected");
    config.driver_options.insert(key, value);
    config
}

fn sse(frames: &[Value]) -> String {
    let mut body = String::new();
    for frame in frames {
        body.push_str(&format!("data: {frame}\n\n"));
    }
    body
}

async fn mount(server: &MockServer, body: String) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/event-stream"))
        .mount(server)
        .await;
}

fn completed_only() -> String {
    sse(&[json!({
        "type": "response.completed",
        "response": { "id": "resp_1", "status": "completed", "output": [],
                      "usage": { "input_tokens": 5, "output_tokens": 1, "total_tokens": 6 } }
    })])
}

async fn sent_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    requests[0].body_json().unwrap()
}

async fn drain(provider: &Provider, config: &LlmCallConfig) -> Vec<LlmStreamEvent> {
    let mut stream = provider
        .chat_completion_stream(vec![Message::text(MessageRole::User, "news?")], config)
        .await
        .expect("request accepted");
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.expect("no transport error"));
    }
    events
}

#[tokio::test]
async fn web_search_is_sent_without_function_tools() {
    let server = MockServer::start().await;
    mount(&server, completed_only()).await;

    drain(
        &hosted_driver(&server, true),
        &web_search_config("gpt-6.1-sol"),
    )
    .await;

    let body = sent_body(&server).await;
    assert_eq!(
        body["tools"],
        json!([{ "type": "web_search", "search_context_size": "low" }])
    );
    // The option itself is Everruns-internal and never reaches the wire.
    assert!(!body.to_string().contains("openai/hosted_tools"));
}

#[tokio::test]
async fn web_search_follows_function_tools() {
    let server = MockServer::start().await;
    mount(&server, completed_only()).await;
    let mut config = web_search_config("gpt-6.1-sol");
    config.tools = vec![ToolDefinition::function(
        "lookup",
        "look something up",
        json!({ "type": "object", "properties": {} }),
    )];

    drain(&hosted_driver(&server, true), &config).await;

    let tools = sent_body(&server).await["tools"].clone();
    let types: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["type"].as_str().unwrap())
        .collect();
    assert_eq!(types, ["function", "web_search"]);
}

#[tokio::test]
async fn endpoint_without_hosted_tools_refuses_instead_of_dropping() {
    let server = MockServer::start().await;
    mount(&server, completed_only()).await;

    let Err(error) = hosted_driver(&server, false)
        .chat_completion_stream(
            vec![Message::text(MessageRole::User, "news?")],
            &web_search_config("gpt-6.1-sol"),
        )
        .await
    else {
        panic!("a gateway without hosted tools must not send the request");
    };
    assert!(error.to_string().contains("hosted tools"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn no_option_sends_no_hosted_tools() {
    let server = MockServer::start().await;
    mount(&server, completed_only()).await;

    drain(
        &hosted_driver(&server, true),
        &LlmCallConfig::new("gpt-6.1-sol"),
    )
    .await;

    assert!(sent_body(&server).await.get("tools").is_none());
}

/// Frames in the order OpenAI streams a web-searched answer.
fn web_search_stream() -> String {
    let web_search_call = json!({
        "type": "web_search_call", "id": "ws_1", "status": "completed",
        "action": { "type": "search", "query": "everruns release" }
    });
    let citation = json!({
        "type": "url_citation", "url": "https://everruns.com/blog/",
        "title": "Everruns blog", "start_index": 0, "end_index": 8
    });
    let message = json!({
        "type": "message", "id": "msg_1", "status": "completed", "role": "assistant",
        "content": [{ "type": "output_text", "text": "Released", "annotations": [citation] }]
    });
    let response = |status: &str, output: Value| {
        json!({
            "id": "resp_ws", "object": "response", "created_at": 1, "status": status,
            "model": "gpt-6.1-sol", "output": output,
            "tools": [{ "type": "web_search", "search_context_size": "low" }],
            "usage": { "input_tokens": 120, "output_tokens": 4, "total_tokens": 124 }
        })
    };
    sse(&[
        json!({ "type": "response.created", "sequence_number": 0, "response": response("in_progress", json!([])) }),
        json!({ "type": "response.output_item.added", "sequence_number": 1, "output_index": 0,
                "item": { "type": "web_search_call", "id": "ws_1", "status": "in_progress" } }),
        json!({ "type": "response.web_search_call.in_progress", "sequence_number": 2, "output_index": 0, "item_id": "ws_1" }),
        json!({ "type": "response.web_search_call.searching", "sequence_number": 3, "output_index": 0, "item_id": "ws_1" }),
        json!({ "type": "response.web_search_call.completed", "sequence_number": 4, "output_index": 0, "item_id": "ws_1" }),
        json!({ "type": "response.output_item.done", "sequence_number": 5, "output_index": 0, "item": web_search_call }),
        json!({ "type": "response.output_item.added", "sequence_number": 6, "output_index": 1,
                "item": { "type": "message", "id": "msg_1", "status": "in_progress", "role": "assistant", "content": [] } }),
        json!({ "type": "response.output_text.delta", "sequence_number": 7, "item_id": "msg_1",
                "output_index": 1, "content_index": 0, "delta": "Released" }),
        json!({ "type": "response.output_text.annotation.added", "sequence_number": 8, "item_id": "msg_1",
                "output_index": 1, "content_index": 0, "annotation_index": 0, "annotation": citation }),
        json!({ "type": "response.output_text.done", "sequence_number": 9, "item_id": "msg_1",
                "output_index": 1, "content_index": 0, "text": "Released" }),
        json!({ "type": "response.output_item.done", "sequence_number": 10, "output_index": 1, "item": message }),
        json!({ "type": "response.completed", "sequence_number": 11,
                "response": response("completed", json!([web_search_call, message])) }),
    ])
}

#[tokio::test]
async fn web_search_response_streams_as_answer_text() {
    let server = MockServer::start().await;
    mount(&server, web_search_stream()).await;

    let events = drain(
        &hosted_driver(&server, true),
        &web_search_config("gpt-6.1-sol"),
    )
    .await;

    let mut text = String::new();
    let mut done = None;
    let mut hosted = Vec::new();
    for event in events {
        match event {
            LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
            LlmStreamEvent::HostedToolCall(call) => hosted.push(call),
            LlmStreamEvent::Done(meta) => done = Some(meta),
            LlmStreamEvent::ToolCalls(calls) => {
                panic!("a hosted call is not an agent tool call: {calls:?}")
            }
            LlmStreamEvent::NativeToolCall(call) => {
                panic!("a hosted call is not a native tool call: {call:?}")
            }
            LlmStreamEvent::Error(error) => panic!("web search frame broke the stream: {error:?}"),
            _ => {}
        }
    }
    assert_eq!(text, "Released");
    let done = done.expect("stream completes");
    assert_eq!(done.finish_reason.as_deref(), Some("stop"));
    assert_eq!(done.response_id.as_deref(), Some("resp_ws"));
    assert_eq!(done.prompt_tokens, Some(120));
    assert_eq!(done.completion_tokens, Some(4));
    assert_eq!(done.hosted_tool_calls.get("web_search_call"), Some(&1));
    assert_eq!(
        hosted,
        vec![
            HostedToolCall {
                id: "ws_1".into(),
                tool: "web_search".into(),
                status: HostedToolCallStatus::InProgress,
                summary: None,
            },
            HostedToolCall {
                id: "ws_1".into(),
                tool: "web_search".into(),
                status: HostedToolCallStatus::Completed,
                summary: Some("everruns release".into()),
            },
        ]
    );
}
