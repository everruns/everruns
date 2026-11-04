#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "http")]

use everruns_contracts::user_facing_error::{ErrorDisclosure, UserFacingErrorContext};
use everruns_contracts::{BearerAuth, LlmCallConfig, OpenResponsesProtocolChatDriver, Provider};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const REASON: &str = "I cannot help with that request, even after a timeout or rate limit.";

fn resource() -> Value {
    json!({"id":"resp_refused", "object":"response", "created_at":1, "model":"test-model", "status":"completed",
        "output":[{"type":"message","id":"msg_1","role":"assistant","status":"completed",
            "content":[{"type":"refusal","refusal":REASON}]}],
        "usage":{"input_tokens":3,"output_tokens":5,"total_tokens":8}})
}

#[tokio::test]
async fn streaming_and_json_refusals_have_the_same_non_retryable_classification() {
    for native_json in [false, true] {
        let server = MockServer::start().await;
        let response = if native_json {
            ResponseTemplate::new(200).set_body_json(resource())
        } else {
            let frames = [
                json!({"type":"response.refusal.delta", "item_id":"msg_1","output_index":0,"content_index":0,"sequence_number":1,"delta":"I cannot help"}),
                json!({"type":"response.refusal.delta", "item_id":"msg_1","output_index":0,"content_index":0,"sequence_number":2,"delta":" with that request"}),
                json!({"type":"response.refusal.done", "item_id":"msg_1","output_index":0,"content_index":0,"sequence_number":3,"refusal":REASON}),
                json!({"type":"response.completed","sequence_number":4,"response":resource()}),
            ];
            ResponseTemplate::new(200).set_body_raw(
                frames
                    .iter()
                    .map(|f| format!("data: {f}\n\n"))
                    .collect::<String>(),
                "text/event-stream",
            )
        };
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        let provider = Provider::new("test", OpenResponsesProtocolChatDriver::new())
            .base_url(format!("{}/responses", server.uri()))
            .auth(BearerAuth::new("test"));
        let mut config = LlmCallConfig::new("test-model");
        config.prefer_non_streaming = native_json;
        let error = provider
            .chat_completion(vec![], &config)
            .await
            .expect_err("a refusal is not a successful answer");
        assert_eq!(
            serde_json::to_value(error.llm_error_kind()).unwrap(),
            json!("refusal")
        );
        assert!(!error.is_transient_llm_error());
        assert!(error.is_non_retryable());
        assert!(error.to_string().contains(REASON), "{error}");
        let classified = error.user_facing_error(UserFacingErrorContext::default());
        assert_eq!(classified.code, "model_refusal");
        assert!(!classified.fallback_message().contains(REASON));
        let detailed =
            classified.apply_disclosure(ErrorDisclosure::Detailed, Some(&error.to_string()));
        assert!(detailed.fields["detail"].as_str().unwrap().contains(REASON));
        assert_eq!(
            classified
                .apply_disclosure(ErrorDisclosure::Generic, None)
                .code,
            "processing_error"
        );
        server.verify().await;
    }
}

#[tokio::test]
async fn refusal_terminal_shapes_do_not_leak_text_or_success() {
    use everruns_contracts::LlmStreamEvent;
    use futures::StreamExt;
    for frame in [
        json!({"type":"response.completed","response":resource()}),
        json!({"type":"response.done","response":resource()}),
        json!({"type":"response.output_item.done","item":resource()["output"][0]}),
        json!({"type":"response.content_part.done","part":{"type":"refusal","refusal":REASON}}),
        json!({"type":"response.refusal.done","refusal":REASON}),
        // No final snapshot: retain the fragment and its non-transient meaning.
        json!({"type":"response.refusal.delta","delta":REASON}),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!("data: {frame}\n\ndata: [DONE]\n\n"),
                "text/event-stream",
            ))
            .expect(1)
            .mount(&server)
            .await;
        let provider = Provider::new("test", OpenResponsesProtocolChatDriver::new())
            .base_url(format!("{}/responses", server.uri()));
        let mut stream = provider
            .chat_completion_stream(vec![], &LlmCallConfig::new("test-model"))
            .await
            .unwrap();
        let mut errors = vec![];
        while let Some(event) = stream.next().await {
            match event.unwrap() {
                LlmStreamEvent::Error(error) => errors.push(error),
                LlmStreamEvent::TextDelta(text) => assert!(text.is_empty()),
                other => panic!("refusal became successful output: {other:?}"),
            }
        }
        assert_eq!(errors.len(), 1, "{frame}");
        assert_eq!(errors[0].message, REASON);
        assert!(!everruns_contracts::llm_retry::is_transient_stream_error(
            &errors[0]
        ));
    }
}

#[tokio::test]
async fn batch_refusal_preserves_the_same_reason_and_kind() {
    let error = OpenResponsesProtocolChatDriver::new()
        .decode_batch_response(resource())
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(error.llm_error_kind()).unwrap(),
        json!("refusal")
    );
    assert!(error.to_string().contains(REASON));
}

#[tokio::test]
async fn unicode_refusal_buffer_is_bounded() {
    let server = MockServer::start().await;
    let frame = json!({"type":"response.refusal.delta","delta":"Ні🙂".repeat(5000)});
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(format!("data: {frame}\n\n"), "text/event-stream"),
        )
        .mount(&server)
        .await;
    let error = Provider::new("test", OpenResponsesProtocolChatDriver::new())
        .base_url(format!("{}/responses", server.uri()))
        .chat_completion(vec![], &LlmCallConfig::new("test-model"))
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(error.llm_error_kind()).unwrap(),
        json!("refusal")
    );
    assert!(error.to_string().len() < 16 * 1024 + 100);
}

#[tokio::test]
async fn interrupted_refusal_does_not_turn_into_a_provider_outage() {
    let server = MockServer::start().await;
    let frames = [
        json!({"type":"response.refusal.delta","delta":REASON}),
        json!({"type":"error","error":{"code":"server_error","message":"timeout"}}),
    ];
    Mock::given(method("POST"))
        .and(path("/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                frames
                    .iter()
                    .map(|f| format!("data: {f}\n\n"))
                    .collect::<String>(),
                "text/event-stream",
            ),
        )
        .expect(1)
        .mount(&server)
        .await;
    let error = Provider::new("test", OpenResponsesProtocolChatDriver::new())
        .base_url(format!("{}/responses", server.uri()))
        .chat_completion(vec![], &LlmCallConfig::new("test-model"))
        .await
        .unwrap_err();
    assert_eq!(
        serde_json::to_value(error.llm_error_kind()).unwrap(),
        json!("refusal")
    );
    assert!(error.to_string().contains(REASON));
}
