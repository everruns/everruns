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

fn fixture_driver() -> OpenResponsesProtocolChatDriver {
    // Wiremock reuses server addresses across tests, but each #[tokio::test]
    // owns a separate runtime. A process-wide client can reuse a connection
    // whose I/O task is aborted when another test finishes, provoking a valid
    // pre-response transport retry. Keep this fixture's pool on its own runtime
    // without disabling retries or weakening the one-request expectations.
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    OpenResponsesProtocolChatDriver::new().with_transport(std::sync::Arc::new(client))
}

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
        let provider = Provider::new("test", fixture_driver())
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
        let provider =
            Provider::new("test", fixture_driver()).base_url(format!("{}/responses", server.uri()));
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
        server.verify().await;
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
    let error = Provider::new("test", fixture_driver())
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
    let error = Provider::new("test", fixture_driver())
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

// A pooled mock address can be reused by a second #[tokio::test] while the
// runtime that owns its old HTTP connection shuts down. Reproduce that ordering
// deliberately instead of relying on concurrent test completion timing.
#[test]
fn refusal_fixture_survives_another_test_runtime_shutting_down() {
    let old_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (request_seen_tx, request_seen_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let release_rx = std::sync::Mutex::new(Some(release_rx));
    let server = old_runtime.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(
                    "data: {}\n\n",
                    json!({"type":"response.refusal.done","refusal":REASON})
                ),
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let error = Provider::new("warmup", fixture_driver())
            .base_url(format!("{}/responses", server.uri()))
            .chat_completion(vec![], &LlmCallConfig::new("test-model"))
            .await
            .unwrap_err();
        assert_eq!(
            error.llm_error_kind(),
            Some(everruns_contracts::LlmErrorKind::Refusal)
        );
        server.reset().await;
        let response = ResponseTemplate::new(200).set_body_raw(
            format!(
                "data: {}\n\n",
                json!({"type":"response.refusal.done","refusal":REASON})
            ),
            "text/event-stream",
        );
        Mock::given(method("POST"))
            .and(path("/responses"))
            .respond_with(move |_: &wiremock::Request| {
                if let Some(release) = release_rx.lock().unwrap().take() {
                    request_seen_tx.send(()).unwrap();
                    release
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                }
                response.clone()
            })
            .expect(1)
            .mount(&server)
            .await;
        server
    });
    let url = format!("{}/responses", server.uri());
    let caller = std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                Provider::new("next-test", fixture_driver())
                    .base_url(url)
                    .chat_completion(vec![], &LlmCallConfig::new("test-model"))
                    .await
                    .unwrap_err()
            })
    });
    request_seen_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    // The next request reached the server, but it cannot receive the refusal
    // until after all connection tasks owned by the old runtime are aborted.
    drop(old_runtime);
    release_tx.send(()).unwrap();
    let error = caller.join().unwrap();
    assert_eq!(
        error.llm_error_kind(),
        Some(everruns_contracts::LlmErrorKind::Refusal)
    );
    assert!(!error.is_transient_llm_error());
    futures::executor::block_on(server.verify());
}

#[tokio::test]
async fn observed_refusal_is_not_reconnected_after_a_body_transport_failure() {
    use everruns_contracts::http_transport::{
        HeaderMap, HttpError, HttpRequest, HttpResponse, HttpTransport, StatusCode,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct BrokenBody(AtomicUsize);
    #[async_trait::async_trait]
    impl HttpTransport for BrokenBody {
        async fn execute(&self, _: HttpRequest) -> Result<HttpResponse, HttpError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let frame = format!(
                "data: {}\n\n",
                json!({"type":"response.refusal.delta","delta":REASON})
            );
            Ok(HttpResponse::new(
                StatusCode::OK,
                HeaderMap::new(),
                Box::pin(futures::stream::iter([
                    Ok(frame.into_bytes().into()),
                    Err(HttpError::new("connection reset after refusal", true)),
                ])),
            ))
        }
    }
    let transport = Arc::new(BrokenBody(AtomicUsize::new(0)));
    let error = Provider::new(
        "test",
        OpenResponsesProtocolChatDriver::new().with_transport(transport.clone()),
    )
    .base_url("https://refusal-fixture.invalid/responses")
    .chat_completion(vec![], &LlmCallConfig::new("test-model"))
    .await
    .unwrap_err();
    assert_eq!(
        error.llm_error_kind(),
        Some(everruns_contracts::LlmErrorKind::Refusal)
    );
    assert!(error.to_string().contains(REASON));
    assert!(!error.is_transient_llm_error());
    assert_eq!(transport.0.load(Ordering::SeqCst), 1);
}
