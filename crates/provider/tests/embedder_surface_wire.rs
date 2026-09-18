// Wire tests for the surfaces an embedder drives the drivers through.
//
// These exist because the unit tests for each piece stub the layer below them:
// the error mapping is tested against a constructed body, the turn collector
// against a synthetic stream, the capture against a serialized struct. None of
// that proves the pieces meet correctly over a socket, which is the only way an
// embedder ever uses them. Each test here goes through a real HTTP server and
// the real driver.

use std::time::Duration;

use everruns_provider::driver_registry::{LlmCallConfig, Message, MessageRole};
use everruns_provider::error::LlmErrorKind;
use everruns_provider::tool_types::ToolDefinition;
use everruns_provider::turn_collector::{TurnLimits, collect_turn};
use everruns_provider::{BearerAuth, OpenAIProtocolChatDriver, Provider};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn provider(server: &MockServer) -> Provider {
    Provider::new("test", OpenAIProtocolChatDriver::new())
        .base_url(server.uri())
        .auth(BearerAuth::new("sk-test-key"))
}

fn user(text: &str) -> Vec<Message> {
    vec![Message::text(MessageRole::User, text)]
}

/// An SSE body from its `data:` payloads, terminated the way the API does.
fn sse(chunks: &[Value]) -> String {
    let mut body = String::new();
    for chunk in chunks {
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    body
}

fn text_chunk(delta: &str) -> Value {
    json!({
        "id": "chatcmpl-1",
        "choices": [{"index": 0, "delta": {"content": delta}, "finish_reason": null}],
    })
}

#[tokio::test]
async fn a_provider_refusal_arrives_with_its_status_and_code_intact() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {
                "message": "Rate limit reached for gpt-5-mini",
                "type": "requests",
                "code": "rate_limit_exceeded",
            }
        })))
        .mount(&server)
        .await;

    let error = provider(&server)
        .chat_completion(user("hi"), &LlmCallConfig::new("gpt-5-mini"))
        .await
        .expect_err("a 429 is a failure");

    // The whole point: a consumer re-expressing this failure reads fields,
    // not the display string.
    assert_eq!(error.http_status(), Some(429));
    assert_eq!(error.provider_error_code(), Some("rate_limit_exceeded"));
    assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::RateLimited));
    assert!(error.is_rate_limited());
    assert!(error.is_transient_llm_error());
}

#[tokio::test]
async fn an_unauthorized_key_is_classified_and_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "error": {"message": "Incorrect API key", "code": "invalid_api_key"}
        })))
        .mount(&server)
        .await;

    let error = provider(&server)
        .chat_completion(user("hi"), &LlmCallConfig::new("gpt-5-mini"))
        .await
        .expect_err("a 401 is a failure");

    assert_eq!(error.http_status(), Some(401));
    assert_eq!(error.provider_error_code(), Some("invalid_api_key"));
    assert!(error.is_auth_error());
    assert!(
        !error.is_transient_llm_error(),
        "a rejected key does not improve by retrying"
    );
}

#[tokio::test]
async fn a_tool_call_declared_in_one_line_round_trips_over_the_wire() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse(&[
            json!({
                "id": "chatcmpl-1",
                "choices": [{
                    "index": 0,
                    "delta": {"tool_calls": [{
                        "index": 0,
                        "id": "call_1",
                        "function": {"name": "get_weather", "arguments": "{\"city\":"},
                    }]},
                    "finish_reason": null,
                }],
            }),
            json!({
                "id": "chatcmpl-1",
                "choices": [{
                    "index": 0,
                    "delta": {"tool_calls": [{
                        "index": 0,
                        "function": {"arguments": "\"Kyiv\"}"},
                    }]},
                    "finish_reason": "tool_calls",
                }],
            }),
        ])))
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gpt-5-mini");
    config.tools = vec![ToolDefinition::function(
        "get_weather",
        "Current weather for a city",
        json!({"type": "object", "properties": {"city": {"type": "string"}}}),
    )];

    let response = provider(&server)
        .chat_completion(user("weather in Kyiv?"), &config)
        .await
        .expect("the turn completes");

    let calls = response.tool_calls.expect("the model called the tool");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_weather");
    // Reassembled across two chunks, which is the case that goes wrong when a
    // consumer folds the stream itself.
    assert_eq!(calls[0].arguments, json!({"city": "Kyiv"}));
    assert_eq!(
        response.metadata.finish_reason.as_deref(),
        Some("tool_calls")
    );
}

#[tokio::test]
async fn the_tool_schema_reaches_the_provider_in_the_openai_shape() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse(&[text_chunk("ok")])))
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gpt-5-mini");
    config.tools = vec![ToolDefinition::function(
        "get_weather",
        "Current weather for a city",
        json!({"type": "object", "properties": {"city": {"type": "string"}}}),
    )];
    provider(&server)
        .chat_completion(user("hi"), &config)
        .await
        .expect("the turn completes");

    let sent: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(sent["tools"][0]["type"], json!("function"));
    assert_eq!(sent["tools"][0]["function"]["name"], json!("get_weather"));
}

#[tokio::test]
async fn a_captured_request_is_the_body_the_server_received() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse(&[text_chunk("hello")])))
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gpt-5-mini");
    config.capture_request = true;
    config.temperature = Some(0.25);

    let response = provider(&server)
        .chat_completion(user("hi"), &config)
        .await
        .expect("the turn completes");

    let captured = response
        .metadata
        .request_body
        .expect("the call asked for the body");
    let received: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    // Not an approximation of the request: the bytes the server got.
    assert_eq!(captured, received);
    assert_eq!(captured["temperature"], json!(0.25));
}

#[tokio::test]
async fn nothing_credential_shaped_reaches_the_captured_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse(&[text_chunk("hello")])))
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gpt-5-mini");
    config.capture_request = true;
    let response = provider(&server)
        .chat_completion(user("hi"), &config)
        .await
        .expect("the turn completes");

    let captured = response
        .metadata
        .request_body
        .expect("captured")
        .to_string();
    // TM-LLM-039: the key is presented in a header, and the capture is the
    // body only — so the one secret in play cannot ride along with the prompt.
    assert!(!captured.contains("sk-test-key"), "{captured}");
    let received: &Request = &server.received_requests().await.unwrap()[0];
    assert!(
        received.headers.contains_key("authorization"),
        "the key really was sent, just not in the body"
    );
}

#[tokio::test]
async fn the_capture_stays_off_unless_the_call_asks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse(&[text_chunk("hello")])))
        .mount(&server)
        .await;

    let response = provider(&server)
        .chat_completion(user("hi"), &LlmCallConfig::new("gpt-5-mini"))
        .await
        .expect("the turn completes");
    assert_eq!(response.metadata.request_body, None);
}

#[tokio::test]
async fn reasoning_tokens_survive_the_streamed_usage_frame() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse(&[
            text_chunk("done"),
            json!({
                "id": "chatcmpl-1",
                "choices": [],
                "usage": {
                    "prompt_tokens": 40,
                    "completion_tokens": 30,
                    "completion_tokens_details": {"reasoning_tokens": 12},
                },
            }),
        ])))
        .mount(&server)
        .await;

    let response = provider(&server)
        .chat_completion(user("think"), &LlmCallConfig::new("gpt-5-mini"))
        .await
        .expect("the turn completes");

    assert_eq!(response.metadata.reasoning_tokens, Some(12));
    // A subset of the completion total, not an addition to it.
    assert_eq!(response.metadata.completion_tokens, Some(30));
}

#[tokio::test]
async fn a_runaway_answer_is_cut_off_at_the_byte_cap() {
    let server = MockServer::start().await;
    let flood: Vec<Value> = (0..64).map(|_| text_chunk(&"x".repeat(256))).collect();
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse(&flood)))
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gpt-5-mini");
    config.limits = TurnLimits::default().with_max_response_bytes(1024);

    let error = provider(&server)
        .chat_completion(user("go"), &config)
        .await
        .expect_err("an answer past the cap is not a turn");
    assert_eq!(
        error.llm_error_kind(),
        Some(LlmErrorKind::MalformedResponse)
    );
    assert!(error.to_string().contains("1024-byte limit"), "{error}");
}

#[tokio::test]
async fn a_provider_that_never_answers_does_not_hold_the_caller() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse(&[text_chunk("late")]))
                .set_delay(Duration::from_secs(30)),
        )
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gpt-5-mini");
    config.limits = TurnLimits::default().with_total(Duration::from_millis(200));

    let started = std::time::Instant::now();
    let error = provider(&server)
        .chat_completion(user("go"), &config)
        .await
        .expect_err("the turn ran out of time");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the limit must cut the wait short, took {:?}",
        started.elapsed()
    );
    assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::Unavailable));
}

#[tokio::test]
async fn an_unlimited_call_keeps_its_previous_unbounded_behavior() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(sse(&[text_chunk(&"x".repeat(200_000))])),
        )
        .mount(&server)
        .await;

    let response = provider(&server)
        .chat_completion(user("go"), &LlmCallConfig::new("gpt-5-mini"))
        .await
        .expect("no limits were set, so nothing bounds this");
    assert_eq!(response.text.len(), 200_000);
}

#[tokio::test]
async fn a_streaming_caller_sees_deltas_live_and_still_gets_the_folded_turn() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse(&[
            text_chunk("Hel"),
            text_chunk("lo"),
            json!({
                "id": "chatcmpl-1",
                "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 3, "completion_tokens": 2},
            }),
        ])))
        .mount(&server)
        .await;

    let stream = provider(&server)
        .chat_completion_stream(user("hi"), &LlmCallConfig::new("gpt-5-mini"))
        .await
        .expect("the stream starts");

    let mut seen = Vec::new();
    let turn = collect_turn(stream, &TurnLimits::default(), |event| {
        if let everruns_provider::driver_registry::LlmStreamEvent::TextDelta(delta) = event {
            seen.push(delta.clone());
        }
    })
    .await
    .expect("the turn folds");

    // The observer sees every event, including the empty delta the finish
    // frame rides in on; only the fold skips empties. A renderer therefore
    // filters, and the text it shows still matches the folded turn.
    assert_eq!(
        seen,
        vec!["Hel".to_string(), "lo".to_string(), String::new()]
    );
    let rendered: String = seen.concat();
    assert_eq!(rendered, turn.text);
    assert_eq!(turn.text, "Hello");
    assert!(turn.complete, "the stream carried its terminal event");
    assert!(turn.timing.time_to_first_event.is_some());
}

#[tokio::test]
async fn a_cut_short_stream_is_visible_rather_than_passing_for_a_whole_turn() {
    let server = MockServer::start().await;
    // No `[DONE]`, no finish frame: the connection just ends.
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: {\"id\":\"c\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"partial\"}}]}\n\n"))
        .mount(&server)
        .await;

    let stream = provider(&server)
        .chat_completion_stream(user("hi"), &LlmCallConfig::new("gpt-5-mini"))
        .await
        .expect("the stream starts");
    let turn = collect_turn(stream, &TurnLimits::default(), |_| {})
        .await
        .expect("the lenient default keeps the partial text");
    assert_eq!(turn.text, "partial");
    assert!(
        !turn.complete,
        "the caller can tell this turn was cut short"
    );

    let stream = provider(&server)
        .chat_completion_stream(user("hi"), &LlmCallConfig::new("gpt-5-mini"))
        .await
        .expect("the stream starts");
    let error = collect_turn(
        stream,
        &TurnLimits::default().requiring_terminal_event(),
        |_| {},
    )
    .await
    .expect_err("a caller that needs real usage refuses it");
    assert_eq!(
        error.llm_error_kind(),
        Some(LlmErrorKind::MalformedResponse)
    );
}
