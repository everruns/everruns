//! Test-only Worker. No credentials or external models are required.
use everruns_contracts::http_transport::{HttpTransport, Request};
use everruns_contracts::{
    BearerAuth, ChatDriver, ClientSideTool, LlmCallConfig, LlmRetryConfig, LlmStreamEvent, Message,
    MessageRole, OpenAIProtocolChatDriver, OpenResponsesProtocolChatDriver, Provider,
    ToolDefinition,
};
use everruns_drivers::{
    openai::{OpenAIChatDriver, OpenAICompletionsChatDriver},
    openrouter::{OpenRouterChatDriver, options::*},
};
use futures::StreamExt;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use worker::*;

// Exercise exactly the adapter used by the Durable Object example.
#[path = "../../../examples/celld-engine/src/fetch.rs"]
mod host_fetch;
use host_fetch::FetchTransport;

async fn run(base: &str, driver: &str, scenario: &str) -> everruns_contracts::Result<Value> {
    let transport = Arc::new(FetchTransport);
    if scenario == "empty-body" {
        let response = transport
            .execute(
                Request::builder()
                    .uri(format!("{base}/empty"))
                    .body(vec![])
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 204);
        assert!(response.bytes().await.unwrap().is_empty());
        return Ok(json!({"empty": true}));
    }
    let retry = LlmRetryConfig {
        max_retries: 1,
        initial_backoff: Duration::from_millis(1),
        jitter_factor: 0.0,
        ..Default::default()
    };
    let wire: Box<dyn ChatDriver> = match driver {
        "openrouter" => Box::new(
            OpenRouterChatDriver::new()
                .with_transport(transport.clone())
                .with_retry_config(retry),
        ),
        "openai" => Box::new(
            OpenAIChatDriver::new()
                .with_transport(transport.clone())
                .with_retry_config(retry),
        ),
        "openai-completions" => Box::new(
            OpenAICompletionsChatDriver::new()
                .with_transport(transport.clone())
                .with_retry_config(retry),
        ),
        "responses" => Box::new(
            OpenResponsesProtocolChatDriver::new()
                .with_transport(transport.clone())
                .with_retry_config(retry),
        ),
        "missing" => Box::new(OpenAIProtocolChatDriver::new()),
        _ => Box::new(
            OpenAIProtocolChatDriver::new()
                .with_transport(transport.clone())
                .with_retry_config(retry),
        ),
    };
    let provider = Provider::new("test", wire)
        .base_url(format!("{base}/{driver}/{scenario}/v1"))
        .auth(BearerAuth::new("test-key"))
        .header("x-test", "provider");
    let mut config = LlmCallConfig::new("test-model");
    config.max_tokens = Some(128);
    config.temperature = Some(0.2);
    config.capture_request = true;
    if scenario == "roundtrip" || scenario == "retry" {
        config.limits.total = Some(Duration::from_secs(10));
        config.limits.first_event = Some(Duration::from_secs(5));
    }
    if scenario == "timeout" {
        config.limits.total = Some(Duration::from_millis(100));
    }
    if scenario == "malformed" {
        config.limits.require_terminal_event = true;
    }
    config.extra_headers = vec![("X-Test".into(), "call".into())];
    if driver == "openrouter" {
        insert_routing_option(
            &mut config.driver_options,
            &OpenRouterRoutingConfig {
                models: vec!["test-model".into(), "fallback-model".into()],
                route: Some(OpenRouterRoute::Fallback),
                ..Default::default()
            },
        );
        config.metadata.insert(
            OPENROUTER_X_TITLE_METADATA_KEY.into(),
            "WASM transport test".into(),
        );
    }
    let mut messages = vec![Message::text(MessageRole::User, "What is 20 plus 22?")];
    if scenario != "roundtrip" && scenario != "retry" {
        match provider
            .chat_completion_non_streaming(messages, &config)
            .await
        {
            Ok(response) => return Ok(json!({"text":response.text})),
            Err(error) => {
                return Ok(
                    json!({"error":error.to_string(),"status":error.http_status(),"kind":error.llm_error_kind()}),
                );
            }
        }
    }
    config.tools.push(ToolDefinition::ClientSide(ClientSideTool::new("add", "Add two numbers",
        json!({"type":"object","properties":{"a":{"type":"integer"},"b":{"type":"integer"}},"required":["a","b"]})
    )));
    let mut stream = provider
        .chat_completion_stream(messages.clone(), &config)
        .await?;
    let mut calls = Vec::new();
    let mut done = None;
    while let Some(event) = stream.next().await {
        match event? {
            LlmStreamEvent::ToolCalls(value) => calls = value,
            LlmStreamEvent::Done(value) => done = Some(value),
            LlmStreamEvent::Error(error) => panic!("unexpected provider stream error: {error:?}"),
            _ => {}
        }
    }
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "add");
    assert_eq!(calls[0].arguments, json!({"a":20,"b":22}));
    let done = done.expect("tool call must finish");
    assert!(done.request_body.is_some());
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.tool_calls = Some(calls.clone());
    messages.push(assistant);
    let mut result = Message::text(
        MessageRole::Tool,
        (calls[0].arguments["a"].as_i64().unwrap() + calls[0].arguments["b"].as_i64().unwrap())
            .to_string(),
    );
    result.tool_call_id = Some(calls[0].id.clone());
    messages.push(result);
    let mut stream = provider.chat_completion_stream(messages, &config).await?;
    let mut text = String::new();
    let mut done = None;
    while let Some(event) = stream.next().await {
        match event? {
            LlmStreamEvent::TextDelta(delta) => {
                text.push_str(&delta);
                // The server withholds Done until this acknowledgement. A
                // buffered adapter deadlocks, proving this is incremental SSE.
                transport
                    .execute(
                        Request::builder()
                            .uri(format!("{base}/ack/{driver}/{scenario}"))
                            .body(vec![])
                            .unwrap(),
                    )
                    .await
                    .unwrap();
            }
            LlmStreamEvent::Done(value) => done = Some(value),
            LlmStreamEvent::Error(error) => panic!("unexpected provider stream error: {error:?}"),
            _ => {}
        }
    }
    assert_eq!(text, "42 ✓");
    let done = done.expect("answer must finish");
    assert!(done.request_body.is_some());
    Ok(
        json!({"text":text,"tool_result":42,"input_tokens":done.prompt_tokens,"output_tokens":done.completion_tokens}),
    )
}

#[event(fetch)]
async fn main(req: worker::Request, _env: Env, _ctx: Context) -> worker::Result<Response> {
    let url = req.url()?;
    let params: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
    let result = run(&params["base"], &params["driver"], &params["scenario"])
        .await
        .map_err(|e| Error::from(e.to_string()))?;
    Response::from_json(&result)
}
