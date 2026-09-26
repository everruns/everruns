// Stream-contract wire tests for the OpenAI Chat Completions protocol driver.
//
// Each test pins one guarantee from the stream contract documented on
// `LlmStreamEvent`, as a host sees it through `Provider`: no empty text
// deltas, a missing finish reason stays `None`, error envelopes inside a `200`
// keep the vendor's message and status, and object-shaped tool arguments
// survive. These came from host-integration feedback where each behavior had
// to be discovered by reading driver source.

use everruns_provider::driver_registry::{LlmCallConfig, LlmStreamEvent};
use everruns_provider::{BearerAuth, LlmRetryConfig, OpenAIProtocolChatDriver, Provider, Result};
use futures::StreamExt;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A mock server that lives for the whole test process.
///
/// Drivers share one process-wide `reqwest` pool, but each `#[tokio::test]`
/// runs its own runtime. If a finished test's server were dropped, a later
/// server could get the same port and the pool would hand it a connection
/// whose task died with that earlier runtime ("error sending request").
/// Leaking the server keeps its port bound, so a port is never reused.
async fn mock_server() -> &'static MockServer {
    Box::leak(Box::new(MockServer::start().await))
}

async fn provider(server: &MockServer, content_type: &str, body: String) -> Provider {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(body, content_type))
        .mount(server)
        .await;
    Provider::new(
        "stream-contract",
        OpenAIProtocolChatDriver::new().with_retry_config(LlmRetryConfig::no_retry()),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(BearerAuth::new("test-key"))
}

fn sse(chunks: &[Value]) -> String {
    chunks
        .iter()
        .map(|chunk| format!("data: {chunk}\n\n"))
        .collect::<String>()
        + "data: [DONE]\n\n"
}

/// Every event a host sees through `Provider::chat_completion_stream`.
async fn stream_events(chunks: &[Value]) -> Vec<Result<LlmStreamEvent>> {
    let server = mock_server().await;
    let provider = provider(server, "text/event-stream", sse(chunks)).await;
    provider
        .chat_completion_stream(vec![], &LlmCallConfig::new("model"))
        .await
        .expect("the stream starts")
        .collect()
        .await
}

#[tokio::test]
async fn a_missing_finish_reason_stays_none_instead_of_becoming_stop() {
    let events = stream_events(&[
        json!({"choices":[{"delta":{"content":"cut"}}]}),
        json!({"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1}}),
    ])
    .await;
    let Some(Ok(LlmStreamEvent::Done(meta))) = events.last() else {
        panic!("stream must end with Done: {events:?}");
    };
    assert_eq!(meta.finish_reason, None);
}

#[tokio::test]
async fn usage_role_and_tool_fragment_chunks_emit_no_empty_text_deltas() {
    let events = stream_events(&[
        json!({"choices":[{"delta":{"role":"assistant","content":""}}]}),
        json!({"choices":[{"delta":{"content":"hi"}}]}),
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"f","arguments":"{}"}}]}}]}),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1}}),
    ])
    .await;
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| match event {
            Ok(LlmStreamEvent::TextDelta(text)) => {
                assert!(!text.is_empty(), "empty TextDelta reached the host");
                "text"
            }
            Ok(LlmStreamEvent::ToolCalls(_)) => "tools",
            Ok(LlmStreamEvent::Done(_)) => "done",
            other => panic!("unexpected event {other:?}"),
        })
        .collect();
    assert_eq!(kinds, ["text", "tools", "done"]);
}

#[tokio::test]
async fn an_error_envelope_inside_a_200_stream_keeps_the_vendor_message() {
    for (envelope, code, status) in [
        (
            json!({"error":{"message":"upstream died","code":502}}),
            None,
            Some(502),
        ),
        // OpenRouter's mid-stream shape: the error rides beside a
        // `finish_reason: "error"` choice.
        (
            json!({"id":"gen-1","error":{"message":"upstream died","code":"server_error"},"choices":[{"index":0,"delta":{"content":""},"finish_reason":"error"}]}),
            Some("server_error"),
            None,
        ),
    ] {
        let events = stream_events(&[
            json!({"choices":[{"delta":{"content":"partial"}}]}),
            envelope,
        ])
        .await;
        let error = events
            .iter()
            .find_map(|event| match event {
                Ok(LlmStreamEvent::Error(error)) => Some(error.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no Error event in {events:?}"));
        assert_eq!(error.message, "upstream died");
        assert_eq!(error.code.as_deref(), code);
        assert_eq!(error.status, status);
    }
}

#[tokio::test]
async fn an_error_envelope_in_a_200_json_body_is_an_error_with_its_status() {
    let server = mock_server().await;
    let body = json!({"error": {"message": "upstream died", "code": 502}}).to_string();
    let provider = provider(server, "application/json", body).await;
    let error = provider
        .chat_completion_non_streaming(vec![], &LlmCallConfig::new("model"))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("upstream died"), "{error}");
    assert_eq!(error.http_status(), Some(502));
}

#[tokio::test]
async fn object_shaped_tool_arguments_are_kept_rather_than_failing_the_chunk() {
    let events = stream_events(&[
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read","arguments":{"path":"a.rs"}}}]}}]}),
        json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
    ])
    .await;
    let calls = events
        .iter()
        .find_map(|event| match event {
            Ok(LlmStreamEvent::ToolCalls(calls)) => Some(calls.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no ToolCalls in {events:?}"));
    assert_eq!(calls[0].arguments, json!({"path":"a.rs"}));
}

#[allow(deprecated)]
#[test]
fn pre_rename_message_names_still_resolve() {
    let message: everruns_provider::LlmMessage =
        everruns_provider::LlmMessage::text(everruns_provider::LlmMessageRole::User, "hi");
    assert!(matches!(
        message.content,
        everruns_provider::LlmMessageContent::Text(_)
    ));
}
