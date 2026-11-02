//! Full HTTP lifecycle coverage, including payload deltas and replay recovery.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use everruns_contracts::driver_registry::{LlmContentPart, LlmResponse, PromptCacheConfig};
use everruns_contracts::tool_types::ToolDefinition;
use everruns_contracts::{LlmCallConfig, Message, MessageRole, Provider, StaticHeaderAuth};
use everruns_drivers::anthropic::AnthropicChatDriver;
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

fn config() -> LlmCallConfig {
    let mut config = LlmCallConfig::new("claude-sonnet-4-5");
    config.max_tokens = Some(1024);
    config.capture_request = true;
    config
        .metadata
        .insert("session_id".into(), "test-session".into());
    config
        .driver_options
        .insert("anthropic/message_threads".into(), json!(true));
    config
}

fn provider(server: &MockServer, key: &str) -> Provider {
    Provider::new("test", AnthropicChatDriver::new())
        .base_url(format!("{}/v1", server.uri()))
        .auth(StaticHeaderAuth::new("x-api-key", key))
}

fn initial() -> Vec<Message> {
    vec![
        Message::text(MessageRole::System, "Remember the conversation."),
        Message::text(MessageRole::User, "My number is 47."),
    ]
}

fn next(messages: &mut Vec<Message>, response: &LlmResponse) {
    messages.push(Message::parts(
        MessageRole::Assistant,
        vec![LlmContentPart::ProviderOpaque(
            response.metadata.provider_opaque_content.clone().unwrap(),
        )],
    ));
    messages.last_mut().unwrap().tool_calls = response.tool_calls.clone();
    messages.push(Message::text(MessageRole::User, "What is my number?"));
}

fn frame(kind: &str, value: Value) -> String {
    format!("event: {kind}\ndata: {value}\n\n")
}

fn response(
    request: &Request,
    index: usize,
    stop: &str,
    tool: bool,
    truncate: bool,
) -> ResponseTemplate {
    let body: Value = request.body_json().unwrap();
    let content = if tool {
        json!([{"type":"tool_use","id":"tool_1","name":"lookup","input":{"id":47}}])
    } else {
        json!([{"type":"text","text":"47"}])
    };
    let message = json!({"id":format!("msg_{index}"),"model":"claude-sonnet-4-5","role":"assistant","type":"message","content":content,"stop_reason":stop,"usage":{"input_tokens":10,"output_tokens":2}});
    let template = ResponseTemplate::new(200).insert_header("connection", "close");
    if body["stream"] == false {
        return template.set_body_json(message);
    }
    let mut start = message.clone();
    start["content"] = json!([]);
    let mut frames = frame("message_start", json!({"message":start}));
    frames += &frame(
        "content_block_start",
        json!({"index":0,"content_block":if tool {content[0].clone()} else {json!({"type":"text","text":""})}}),
    );
    frames += &frame(
        "content_block_delta",
        json!({"index":0,"delta":if tool {json!({"type":"input_json_delta","partial_json":"{\"id\":47}"})} else {json!({"type":"text_delta","text":"47"})}}),
    );
    frames += &frame("content_block_stop", json!({"index":0}));
    frames += &frame(
        "message_delta",
        json!({"delta":{"stop_reason":stop},"usage":{"output_tokens":2}}),
    );
    if !truncate {
        frames += &frame("message_stop", json!({}));
    }
    template.set_body_raw(frames, "text/event-stream")
}

async fn server(
    error: Option<(u16, &'static str)>,
    stop: &'static str,
    tool: bool,
    truncate: bool,
) -> MockServer {
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST")).and(path("/v1/messages"))
        .respond_with(move |request: &Request| {
            let index = count.fetch_add(1, Ordering::SeqCst);
            if index == 1 && let Some((status, code)) = error {
                return ResponseTemplate::new(status).insert_header("connection", "close")
                    .set_body_json(json!({"error":{"type":"invalid_request_error","message":"fixture","details":{"error_code":code}}}));
            }
            response(request, index, stop, tool, truncate)
        }).mount(&server).await;
    server
}

#[tokio::test]
async fn streaming_and_json_continue_with_only_new_messages_and_keep_cache_controls() {
    for native in [false, true] {
        let server = server(None, "end_turn", false, false).await;
        let provider = provider(&server, "key");
        let mut config = config();
        config.prefer_non_streaming = native;
        config.prompt_cache = Some(PromptCacheConfig::default());
        config.prompt_cache.as_mut().unwrap().enabled = true;
        let mut messages = initial();
        for i in 0..3 {
            let answer = provider
                .chat_completion(messages.clone(), &config)
                .await
                .unwrap();
            assert_eq!(answer.text, "47");
            let body = answer.metadata.request_body.as_ref().unwrap();
            assert_eq!(body["stream"], !native);
            assert_eq!(
                body["thread"]["type"],
                if i == 0 { "create" } else { "continue" }
            );
            assert_eq!(body["messages"].as_array().unwrap().len(), 1);
            if i > 0 {
                assert!(body.get("system").is_none());
                assert!(body.get("tools").is_none());
                assert_eq!(
                    body["thread"]["previous_message_id"],
                    format!("msg_{}", i - 1)
                );
            }
            next(&mut messages, &answer);
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests.iter().all(|request| {
            request.headers["anthropic-beta"]
                .to_str()
                .unwrap()
                .contains("message-threads-2026-08-12")
        }));
    }
}

#[tokio::test]
async fn tool_results_continue_without_resending_tools_or_assistant_calls() {
    let server = server(None, "tool_use", true, false).await;
    let provider = provider(&server, "key");
    let mut config = config();
    config.tools = vec![ToolDefinition::function(
        "lookup",
        "Look up a number",
        json!({"type":"object","properties":{"id":{"type":"integer"}}}),
    )];
    let mut messages = initial();
    let first = provider
        .chat_completion(messages.clone(), &config)
        .await
        .unwrap();
    next(&mut messages, &first);
    messages.pop();
    let mut result = Message::text(MessageRole::Tool, "found");
    result.tool_call_id = Some("tool_1".into());
    messages.push(result);
    let answer = provider.chat_completion(messages, &config).await.unwrap();
    let body = answer.metadata.request_body.unwrap();
    assert_eq!(body["thread"]["type"], "continue");
    assert!(body.get("tools").is_none());
    assert_eq!(body["messages"][0]["content"][0]["type"], "tool_result");
    assert_eq!(body["messages"][0]["content"][0]["tool_use_id"], "tool_1");
}

#[tokio::test]
async fn thread_errors_replay_full_history_once_with_system_restored() {
    for (status, code) in [
        (404, "thread_not_found"),
        (400, "thread_already_continued"),
        (400, "thread_continue_in_flight"),
        (400, "thread_fingerprint_mismatch"),
        (400, "thread_unsupported_request"),
    ] {
        let server = server(Some((status, code)), "end_turn", false, false).await;
        let provider = provider(&server, "key");
        let config = config();
        let mut messages = initial();
        let first = provider
            .chat_completion(messages.clone(), &config)
            .await
            .unwrap();
        next(&mut messages, &first);
        let answer = provider.chat_completion(messages, &config).await.unwrap();
        let body = answer.metadata.request_body.unwrap();
        assert_eq!(body["messages"].as_array().unwrap().len(), 3, "{code}");
        assert!(body.get("system").is_some());
        if code == "thread_unsupported_request" {
            assert!(body.get("thread").is_none());
        } else {
            assert_eq!(body["thread"]["type"], "create");
        }
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            3,
            "bounded replay {code}"
        );
    }
}

#[tokio::test]
async fn unrelated_errors_are_not_replayed() {
    let server = server(Some((404, "model_not_found")), "end_turn", false, false).await;
    let provider = provider(&server, "key");
    let config = config();
    let mut messages = initial();
    let first = provider
        .chat_completion(messages.clone(), &config)
        .await
        .unwrap();
    next(&mut messages, &first);
    assert!(provider.chat_completion(messages, &config).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

#[tokio::test]
async fn edits_configuration_identity_and_sessions_start_new_threads() {
    for change in [
        "edit",
        "system",
        "session",
        "org",
        "credentials",
        "beta",
        "model",
        "tools",
        "cache_strategy",
    ] {
        let server = server(None, "end_turn", false, false).await;
        let mut provider = provider(&server, "key");
        let mut config = config();
        let mut messages = initial();
        let first = provider
            .chat_completion(messages.clone(), &config)
            .await
            .unwrap();
        next(&mut messages, &first);
        match change {
            "edit" => messages[1] = Message::text(MessageRole::User, "changed"),
            "system" => messages[0] = Message::text(MessageRole::System, "changed"),
            "session" => {
                config.metadata.insert("session_id".into(), "other".into());
            }
            "org" => {
                config.metadata.insert("org_id".into(), "other".into());
            }
            "credentials" => provider = self::provider(&server, "other-key"),
            "beta" => config
                .extra_headers
                .push(("anthropic-beta".into(), "new-beta".into())),
            "model" => config.model = "claude-haiku-4-5".into(),
            "cache_strategy" => {
                config.prompt_cache = Some(PromptCacheConfig::default());
            }
            "tools" => {
                config.tools = vec![ToolDefinition::function(
                    "lookup",
                    "lookup",
                    json!({"type":"object"}),
                )]
            }
            _ => unreachable!(),
        }
        let answer = provider.chat_completion(messages, &config).await.unwrap();
        assert_eq!(
            answer.metadata.request_body.unwrap()["thread"]["type"],
            "create",
            "{change}"
        );
    }
}

#[tokio::test]
async fn concurrent_continuations_claim_response_once() {
    let server = server(None, "end_turn", false, false).await;
    let provider = provider(&server, "key");
    let config = config();
    let mut messages = initial();
    let first = provider
        .chat_completion(messages.clone(), &config)
        .await
        .unwrap();
    next(&mut messages, &first);
    let (a, b) = tokio::join!(
        provider.chat_completion(messages.clone(), &config),
        provider.chat_completion(messages, &config)
    );
    let mut kinds = vec![
        a.unwrap().metadata.request_body.unwrap()["thread"]["type"].clone(),
        b.unwrap().metadata.request_body.unwrap()["thread"]["type"].clone(),
    ];
    kinds.sort_by_key(Value::to_string);
    assert_eq!(kinds, vec![json!("continue"), json!("create")]);
}

#[tokio::test]
async fn disabled_unscoped_and_refused_calls_do_not_reuse_state() {
    for mode in [
        "disabled",
        "unscoped",
        "refusal",
        "max_tokens",
        "pause_turn",
    ] {
        let stop = match mode {
            "refusal" => "refusal",
            "max_tokens" => "max_tokens",
            "pause_turn" => "pause_turn",
            _ => "end_turn",
        };
        let server = server(None, stop, false, false).await;
        let provider = provider(&server, "key");
        let mut config = config();
        if mode == "disabled" {
            config.driver_options.clear();
        }
        if mode == "unscoped" {
            config.metadata.clear();
        }
        let mut messages = initial();
        let first = provider
            .chat_completion(messages.clone(), &config)
            .await
            .unwrap();
        next(&mut messages, &first);
        let answer = provider.chat_completion(messages, &config).await.unwrap();
        let body = answer.metadata.request_body.unwrap();
        assert_eq!(body["messages"].as_array().unwrap().len(), 3);
        assert_ne!(body["thread"]["type"], "continue");
        if matches!(mode, "disabled" | "unscoped") {
            assert!(body.get("thread").is_none());
        }
    }
}

#[tokio::test]
async fn stream_without_message_stop_never_publishes_thread_state() {
    let server = server(None, "end_turn", false, true).await;
    let provider = provider(&server, "key");
    let config = config();
    let mut messages = initial();
    let mut stream = provider
        .chat_completion_stream(messages.clone(), &config)
        .await
        .unwrap();
    while stream.next().await.is_some() {}
    messages.push(Message::text(MessageRole::Assistant, "47"));
    messages.push(Message::text(MessageRole::User, "continue"));
    let _stream = provider
        .chat_completion_stream(messages, &config)
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    let last: Value = requests.last().unwrap().body_json().unwrap();
    assert_eq!(last["thread"]["type"], "create");
}

#[tokio::test]
#[ignore = "requires ANTHROPIC_API_KEY and message-threads beta access; makes paid API calls"]
async fn live_message_threads_preserve_a_fact_across_turns() {
    let key = std::env::var("ANTHROPIC_API_KEY").expect("ANTHROPIC_API_KEY");
    let provider = Provider::new("anthropic", AnthropicChatDriver::new())
        .base_url("https://api.anthropic.com/v1")
        .auth(StaticHeaderAuth::new("x-api-key", key));
    let mut config = config();
    config.model = "claude-haiku-4-5".into();
    config.max_tokens = Some(128);
    config.metadata.insert(
        "session_id".into(),
        format!("message-threads-smoke-{:?}", std::time::SystemTime::now()),
    );
    let mut messages = initial();
    let first = provider
        .chat_completion(messages.clone(), &config)
        .await
        .unwrap();
    assert_eq!(
        first.metadata.request_body.as_ref().unwrap()["thread"]["type"],
        "create"
    );
    next(&mut messages, &first);
    let second = provider
        .chat_completion(messages.clone(), &config)
        .await
        .unwrap();
    assert!(second.text.contains("47"));
    let body = second.metadata.request_body.as_ref().unwrap();
    assert_eq!(body["thread"]["type"], "continue");
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    next(&mut messages, &second);
    messages[0] = Message::text(MessageRole::System, "Answer the user's question briefly.");
    let third = provider.chat_completion(messages, &config).await.unwrap();
    assert!(third.text.contains("47"));
    assert_eq!(
        third.metadata.request_body.as_ref().unwrap()["thread"]["type"],
        "create"
    );
    println!(
        "PASS: create -> delta continuation -> full replay after system change; fact 47 preserved"
    );
}

#[tokio::test]
async fn unsupported_create_falls_back_without_thread_or_beta_and_replay_is_bounded() {
    for code in ["thread_unsupported_request", "thread_not_found"] {
        let server = MockServer::start().await;
        let count = Arc::new(AtomicUsize::new(0));
        Mock::given(method("POST")).and(path("/v1/messages"))
            .respond_with(move |request: &Request| {
                let index = count.fetch_add(1, Ordering::SeqCst);
                if code == "thread_not_found" || index == 0 {
                    ResponseTemplate::new(if code == "thread_not_found" {404} else {400})
                        .insert_header("connection","close")
                        .set_body_json(json!({"error":{"type":"invalid_request_error","details":{"error_code":code}}}))
                } else { response(request,index,"end_turn",false,false) }
            }).mount(&server).await;
        let result = provider(&server, "key")
            .chat_completion(initial(), &config())
            .await;
        let requests = server.received_requests().await.unwrap();
        if code == "thread_unsupported_request" {
            assert!(result.is_ok());
            assert_eq!(requests.len(), 2);
            let body: Value = requests[1].body_json().unwrap();
            assert!(body.get("thread").is_none());
            assert!(
                !requests[1]
                    .headers
                    .get("anthropic-beta")
                    .is_some_and(|value| value
                        .to_str()
                        .unwrap()
                        .contains("message-threads-2026-08-12"))
            );
        } else {
            assert!(result.is_err());
            assert_eq!(
                requests.len(),
                1,
                "create failures cannot trigger another create"
            );
        }
    }
}
