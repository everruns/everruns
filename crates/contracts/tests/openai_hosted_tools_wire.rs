#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "http")]

// Wire tests for OpenAI hosted tools on the Open Responses driver (EVE-1115).
//
// Request: the `openai/hosted_tools` driver option becomes `tools` entries
// next to (or instead of) function tools, and an endpoint without hosted
// tools refuses the option rather than dropping it. Stream: a web search
// response (`web_search_call` items, `response.web_search_call.*` events, url
// citations) streams as answer text plus hosted-call progress events, never
// as an agent tool call or a parse error.

use everruns_contracts::driver_registry::{
    HostedToolCall, HostedToolCallStatus, LlmCallConfig, LlmStreamEvent, Message, MessageRole,
};
use everruns_contracts::hosted_mcp::{HostedMcpDriver, HostedMcpResolver, ResolvedHostedMcp};
use everruns_contracts::openai_hosted_tools::{
    McpServerTool, OPENAI_MCP_APPROVAL_TOOL, OpenAiHostedTools, SearchContextSize, WebSearchTool,
};
use everruns_contracts::tool_types::ToolCall;
use everruns_contracts::{OpenResponsesProtocolChatDriver, Provider, ToolDefinition};
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// A fresh server per test, not one from wiremock's pool: the driver shares one
// process-wide HTTP client, so a keep-alive connection left to a pooled
// server's port by an earlier test can fail on reuse, and the driver's
// transparent reconnect then sends the request twice.
async fn server() -> MockServer {
    MockServer::builder().start().await
}

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
        ..Default::default()
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
    let server = server().await;
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
    let server = server().await;
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
    let server = server().await;
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
    let server = server().await;
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
    let server = server().await;
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

fn mcp_config() -> LlmCallConfig {
    let mut config = LlmCallConfig::new("gpt-6.1-sol");
    let tools = OpenAiHostedTools {
        mcp_servers: vec![McpServerTool {
            server_label: "deepwiki".into(),
            server_url: "https://mcp.deepwiki.com/mcp".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let (key, value) = tools.to_driver_option().expect("mcp selected");
    config.driver_options.insert(key, value);
    config
}

#[tokio::test]
async fn mcp_approval_request_pauses_as_a_synthetic_call() {
    let server = server().await;
    let approval = json!({
        "type": "mcp_approval_request", "id": "mcpr_1", "server_label": "deepwiki",
        "name": "ask_question", "arguments": "{\"repoName\":\"everruns/everruns\"}"
    });
    let list = json!({ "type": "mcp_list_tools", "id": "mcpl_1", "server_label": "deepwiki", "tools": [] });
    mount(
        &server,
        sse(&[
            json!({ "type": "response.output_item.done", "sequence_number": 1, "output_index": 0, "item": list }),
            json!({ "type": "response.output_item.done", "sequence_number": 2, "output_index": 1, "item": approval }),
            json!({ "type": "response.completed", "sequence_number": 3, "response": {
                "id": "resp_mcp", "object": "response", "created_at": 1, "status": "completed",
                "model": "gpt-6.1-sol", "output": [list, approval],
                "usage": { "input_tokens": 50, "output_tokens": 9, "total_tokens": 59 } } }),
        ]),
    )
    .await;

    let events = drain(&hosted_driver(&server, true), &mcp_config()).await;

    let body = sent_body(&server).await;
    assert_eq!(
        body["tools"],
        json!([{ "type": "mcp", "server_label": "deepwiki",
                 "server_url": "https://mcp.deepwiki.com/mcp", "require_approval": "always" }])
    );
    let calls: Vec<ToolCall> = events
        .iter()
        .rev()
        .find_map(|event| match event {
            LlmStreamEvent::ToolCalls(calls) => Some(calls.clone()),
            _ => None,
        })
        .expect("approval surfaces as a tool call");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "mcpr_1");
    assert_eq!(calls[0].name, OPENAI_MCP_APPROVAL_TOOL);
    assert_eq!(calls[0].arguments["name"], "ask_question");
    let done = events.iter().find_map(|event| match event {
        LlmStreamEvent::Done(meta) => Some(meta),
        _ => None,
    });
    assert_eq!(done.unwrap().finish_reason.as_deref(), Some("tool_calls"));
}

#[tokio::test]
async fn approved_call_replays_as_approval_response() {
    let server = server().await;
    mount(&server, completed_only()).await;
    let arguments =
        json!({ "server_label": "deepwiki", "name": "ask_question", "arguments": "{}" });
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.tool_calls = Some(vec![ToolCall {
        id: "mcpr_1".into(),
        name: OPENAI_MCP_APPROVAL_TOOL.into(),
        arguments,
    }]);
    let mut answer = Message::text(MessageRole::Tool, r#"{"approve":true}"#);
    answer.tool_call_id = Some("mcpr_1".into());

    let mut stream = hosted_driver(&server, true)
        .chat_completion_stream(
            vec![Message::text(MessageRole::User, "ask"), assistant, answer],
            &mcp_config(),
        )
        .await
        .expect("request accepted");
    while stream.next().await.is_some() {}

    let input = sent_body(&server).await["input"].clone();
    let items = input.as_array().unwrap();
    assert!(
        items
            .iter()
            .any(|item| item["type"] == "mcp_approval_request" && item["id"] == "mcpr_1")
    );
    assert!(items.iter().any(|item| item
        == &json!({ "type": "mcp_approval_response", "approval_request_id": "mcpr_1", "approve": true })));
    assert!(!items.iter().any(|item| item["type"] == "function_call"));
}

struct RegisteredServers;

#[async_trait::async_trait]
impl HostedMcpResolver for RegisteredServers {
    async fn resolve(&self, server: &str) -> everruns_contracts::error::Result<ResolvedHostedMcp> {
        assert_eq!(server, "github");
        Ok(ResolvedHostedMcp {
            url: "https://mcp.github.example/mcp".into(),
            headers: [("Authorization".to_string(), "Bearer gh-token".to_string())].into(),
        })
    }
}

fn registered_mcp_config() -> LlmCallConfig {
    let mut config = LlmCallConfig::new("gpt-6.1-sol");
    let tools = OpenAiHostedTools {
        mcp_servers: vec![McpServerTool {
            server_label: "gh".into(),
            mcp_server: Some("github".into()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let (key, value) = tools.to_driver_option().expect("mcp selected");
    config.driver_options.insert(key, value);
    config
}

#[tokio::test]
async fn registered_mcp_server_sends_resolved_credentials() {
    let server = server().await;
    mount(&server, completed_only()).await;
    let inner = OpenResponsesProtocolChatDriver::new()
        .with_native_features(true, true)
        .with_hosted_tools(true);
    let driver = HostedMcpDriver::new(
        std::sync::Arc::new(inner),
        std::sync::Arc::new(RegisteredServers),
    );
    let provider = Provider::new("openai", driver).base_url(format!("{}/v1", server.uri()));

    drain(&provider, &registered_mcp_config()).await;

    let body = sent_body(&server).await;
    assert_eq!(
        body["tools"],
        json!([{ "type": "mcp", "server_label": "gh", "server_url": "https://mcp.github.example/mcp",
                 "require_approval": "always", "headers": { "Authorization": "Bearer gh-token" } }])
    );
}

#[tokio::test]
async fn registered_mcp_server_without_a_resolver_is_refused() {
    let server = server().await;
    mount(&server, completed_only()).await;

    let Err(error) = hosted_driver(&server, true)
        .chat_completion_stream(
            vec![Message::text(MessageRole::User, "news?")],
            &registered_mcp_config(),
        )
        .await
    else {
        panic!("an unresolved registered server must not be sent");
    };
    assert!(error.to_string().contains("MCP server gh"), "{error}");
    assert!(server.received_requests().await.unwrap().is_empty());
}

// Native web search (`everruns/web_search`): the shared `web_search` function
// tool is swapped for OpenAI's hosted search only when the endpoint, the model
// and the agent's filters allow it; otherwise the function tool stays so the
// search plan's next provider answers.

fn web_search_function() -> ToolDefinition {
    ToolDefinition::function(
        "web_search",
        "Search the web",
        json!({ "type": "object", "properties": { "query": { "type": "string" } } }),
    )
}

fn native_search_config(
    model: &str,
    native: everruns_contracts::native_web_search::NativeWebSearch,
) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.tools = vec![
        web_search_function(),
        ToolDefinition::function(
            "lookup",
            "look something up",
            json!({ "type": "object", "properties": {} }),
        ),
    ];
    let (key, value) = native.to_driver_option();
    config.driver_options.insert(key, value);
    config
}

fn tool_entries(body: &Value) -> Vec<(String, Option<String>)> {
    body["tools"]
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .map(|tool| {
                    (
                        tool["type"].as_str().unwrap_or_default().to_string(),
                        tool["name"].as_str().map(str::to_string),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn native_search_replaces_the_web_search_function_tool() {
    let server = server().await;
    mount(&server, completed_only()).await;
    let native = everruns_contracts::native_web_search::NativeWebSearch {
        provider: "auto".into(),
        allowed_domains: vec!["docs.rs".into()],
        ..Default::default()
    };

    drain(
        &hosted_driver(&server, true),
        &native_search_config("gpt-6.1-sol", native),
    )
    .await;

    let body = sent_body(&server).await;
    assert_eq!(
        tool_entries(&body),
        [
            ("function".to_string(), Some("lookup".to_string())),
            ("web_search".to_string(), None)
        ]
    );
    assert_eq!(
        body["tools"][1],
        json!({ "type": "web_search", "filters": { "allowed_domains": ["docs.rs"] } })
    );
    assert!(!body.to_string().contains("everruns/web_search"));
}

#[tokio::test]
async fn native_search_declines_and_keeps_the_function_tool() {
    use everruns_contracts::native_web_search::NativeWebSearch;
    let blocked = NativeWebSearch {
        blocked_domains: vec!["pinterest.com".into()],
        ..Default::default()
    };
    // (driver has hosted tools, model, request)
    let cases = [
        // OpenAI cannot exclude domains: the plan's next provider must.
        (true, "gpt-6.1-sol", blocked),
        // A model without web search.
        (true, "gpt-3.5-turbo", NativeWebSearch::default()),
        // A gateway without OpenAI's hosted tools ignores the request (no
        // error: unlike the hosted-tools option it is not a requirement).
        (false, "gpt-6.1-sol", NativeWebSearch::default()),
    ];
    for (hosted, model, native) in cases {
        let server = server().await;
        mount(&server, completed_only()).await;
        drain(
            &hosted_driver(&server, hosted),
            &native_search_config(model, native),
        )
        .await;
        assert_eq!(
            tool_entries(&sent_body(&server).await),
            [
                ("function".to_string(), Some("web_search".to_string())),
                ("function".to_string(), Some("lookup".to_string()))
            ],
            "{model} hosted={hosted}"
        );
    }
}

#[tokio::test]
async fn configured_hosted_web_search_is_not_sent_twice() {
    let server = server().await;
    mount(&server, completed_only()).await;
    let mut config = native_search_config("gpt-6.1-sol", Default::default());
    config
        .driver_options
        .extend(web_search_config("gpt-6.1-sol").driver_options);

    drain(&hosted_driver(&server, true), &config).await;

    let body = sent_body(&server).await;
    // OpenAI Server Tools' own entry (context size low) wins.
    assert_eq!(
        tool_entries(&body),
        [
            ("function".to_string(), Some("lookup".to_string())),
            ("web_search".to_string(), None)
        ]
    );
    assert_eq!(body["tools"][1]["search_context_size"], "low");
}

/// A session that searched natively on OpenAI and then moves to a model
/// without native search replays its history as plain messages: the hosted
/// `web_search_call` items were never part of the transcript, so nothing
/// provider-specific is sent, and the function tool is back.
#[tokio::test]
async fn history_after_native_search_replays_on_a_model_without_it() {
    let server = server().await;
    mount(&server, completed_only()).await;
    let mut config = native_search_config("gpt-3.5-turbo", Default::default());
    // The earlier turn's continuation handle belongs to the other model.
    config.previous_response_id = None;
    let messages = vec![
        Message::text(MessageRole::User, "What shipped in tokio 1.50?"),
        Message::text(
            MessageRole::Assistant,
            "Tokio 1.50 shipped on 2026-10-02 (https://tokio.rs/blog/2026-10-tokio-1-50).",
        ),
        Message::text(MessageRole::User, "And 1.51?"),
    ];
    let mut stream = hosted_driver(&server, true)
        .chat_completion_stream(messages, &config)
        .await
        .expect("request accepted");
    while let Some(event) = stream.next().await {
        event.expect("no transport error");
    }

    let body = sent_body(&server).await;
    assert!(!body.to_string().contains("web_search_call"));
    assert_eq!(
        tool_entries(&body)[0],
        ("function".to_string(), Some("web_search".to_string()))
    );
    let roles: Vec<&str> = body["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["role"].as_str())
        .collect();
    assert_eq!(roles, ["user", "assistant", "user"]);
}

#[tokio::test]
async fn gateway_server_tool_use_reaches_completion_metadata() {
    let server = server().await;
    mount(
        &server,
        sse(&[json!({
            "type": "response.completed",
            "response": { "id": "resp_or", "status": "completed", "output": [],
                          "usage": { "input_tokens": 5, "output_tokens": 1, "total_tokens": 6,
                                     "cost": 0.021,
                                     "server_tool_use": { "web_search_requests": 2 } } }
        })]),
    )
    .await;

    let events = drain(
        &hosted_driver(&server, false),
        &LlmCallConfig::new("openai/gpt-5.2"),
    )
    .await;

    let done = events
        .into_iter()
        .find_map(|event| match event {
            LlmStreamEvent::Done(meta) => Some(meta),
            _ => None,
        })
        .expect("stream completes");
    assert_eq!(done.hosted_tool_calls.get("web_search_call"), Some(&2));
    assert_eq!(done.provider_cost_usd, Some(0.021));
}

#[tokio::test]
async fn unfinished_search_keeps_partial_answer_and_citations_without_retry() {
    for status in ["searching", "in_progress", "failed", "incomplete"] {
        let server = server().await;
        let frames: Vec<Value> = web_search_stream()
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|frame| frame["type"] != "response.web_search_call.completed")
            .map(|mut frame| {
                if frame["type"] == "response.output_item.done"
                    && frame["item"]["type"] == "web_search_call"
                {
                    frame["item"]["status"] = json!(status);
                }
                if frame["type"] == "response.completed" {
                    frame["response"]["output"][0]["status"] = json!(status);
                }
                frame
            })
            .collect();
        mount(&server, sse(&frames)).await;
        let events = drain(
            &hosted_driver(&server, true),
            &web_search_config("gpt-6.1-sol"),
        )
        .await;
        let mut text = String::new();
        let mut hosted = Vec::new();
        let mut completion = None;
        for event in events {
            match event {
                LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
                LlmStreamEvent::HostedToolCall(call) => hosted.push(call),
                LlmStreamEvent::Done(meta) => completion = Some(meta),
                LlmStreamEvent::Error(error) => {
                    panic!("search status must not fail the response: {error:?}")
                }
                LlmStreamEvent::ToolCalls(calls) => {
                    panic!("must not dispatch hosted calls: {calls:?}")
                }
                _ => {}
            }
        }
        assert_eq!(text, "Released");
        assert_eq!(hosted.len(), 2);
        assert_eq!(hosted[0].status, HostedToolCallStatus::InProgress);
        assert_eq!(hosted[1].status, HostedToolCallStatus::Failed, "{status}");
        assert_eq!(hosted[1].summary.as_deref(), Some("everruns release"));
        let completion = completion.expect("partial answer still completes");
        assert_eq!(completion.web_citations.len(), 1);
        assert_eq!(
            completion.web_citations[0].url,
            "https://everruns.com/blog/"
        );
        assert_eq!(
            completion.hosted_tool_calls.get("web_search_call"),
            Some(&1)
        );
        assert_eq!(completion.finish_reason.as_deref(), Some("stop"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
