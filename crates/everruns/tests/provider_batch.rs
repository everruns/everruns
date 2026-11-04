//! Offline acceptance tests of the public Framework API and real HTTP driver.
//! Dedicated servers isolate timed-out responses and shared-client keepalive connections.
#![cfg(feature = "anthropic")]

use std::time::Duration;

use everruns::{
    LlmCallConfig, Model,
    batch::{Batch, BatchError, BatchItemError, BatchRequest, BatchStatus, BatchTicket},
    providers::anthropic::Anthropic,
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

fn client(server: &MockServer, scope: &str) -> Batch {
    Batch::new(
        Anthropic::new("test-key").base_url(format!("{}/v1", server.uri())),
        scope,
    )
}
fn request() -> everruns::llm::Completion {
    Model::from("claude-sonnet-5")
        .completion()
        .system("Classify the text.")
        .user("private prompt")
        .max_tokens(64)
}
fn job(state: &str) -> Value {
    json!({"id":"msgbatch_test", "processing_status":state, "results_url":"https://attacker.invalid/results"})
}
async fn submit(server: &MockServer) -> BatchTicket {
    Mock::given(method("POST"))
        .and(path("/v1/messages/batches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(job("in_progress")))
        .mount(server)
        .await;
    client(server, "tenant-a/account-a")
        .request("first", request())
        .unwrap()
        .request("second", request())
        .unwrap()
        .submit()
        .await
        .unwrap()
}
async fn state(server: &MockServer, name: &str) {
    Mock::given(method("GET"))
        .and(path("/v1/messages/batches/msgbatch_test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(job(name)))
        .mount(server)
        .await;
}
fn success(id: &str, text: &str) -> Value {
    json!({"custom_id":id,"result":{"type":"succeeded","message":{
        "id":"msg_1", "model":"claude-sonnet-5", "role":"assistant", "type":"message",
        "content":[{"type":"text","text":text}], "stop_reason":"end_turn",
        "usage":{"input_tokens":10,"output_tokens":4,"cache_read_input_tokens":20,"cache_creation_input_tokens":30,"cache_creation":{"ephemeral_1h_input_tokens":5}}
    }}})
}
async fn results(server: &MockServer, rows: Vec<Value>) {
    let body = rows
        .into_iter()
        .map(|row| row.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    Mock::given(method("GET"))
        .and(path("/v1/messages/batches/msgbatch_test/results"))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn submit_persist_reconstruct_poll_collect_in_submission_order() {
    let server = MockServer::builder().start().await;
    let ticket = submit(&server).await;
    let saved = serde_json::to_string(&ticket).unwrap();
    for private in [
        "test-key",
        "private prompt",
        "Classify the text",
        &server.uri(),
    ] {
        assert!(!saved.contains(private), "ticket leaked private input");
    }
    assert!(!format!("{ticket:?}").contains("tenant-a"));
    let resumed: BatchTicket = serde_json::from_str(&saved).unwrap();
    assert_eq!(resumed.request_ids(), &["first", "second"]);
    state(&server, "ended").await;
    results(
        &server,
        vec![success("second", "two"), success("first", "one")],
    )
    .await;
    let batch = client(&server, "tenant-a/account-a");
    assert_eq!(batch.poll(&resumed).await.unwrap(), BatchStatus::Ended);
    for _ in 0..2 {
        let output = batch.collect(&resumed).await.unwrap();
        assert_eq!(
            output.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        let first = output[0].result.as_ref().unwrap();
        assert_eq!(first.text, "one");
        assert_eq!(first.metadata.total_tokens, Some(64));
        assert_eq!(first.metadata.prompt_tokens, Some(10));
        assert_eq!(first.metadata.cache_creation_1h_tokens, Some(5));
        assert_eq!(
            first.metadata.provider_finish_reason.as_deref(),
            Some("end_turn")
        );
    }
    let calls = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&calls[0].body).unwrap();
    assert_eq!(body["requests"][0]["params"]["stream"], false);
    assert_eq!(body["requests"][0]["params"]["max_tokens"], 64);
    assert_eq!(
        body["requests"][0]["params"]["system"],
        "Classify the text."
    );
    assert_eq!(calls.iter().filter(|r| r.method == "POST").count(), 1);
}

#[tokio::test]
async fn cancellation_keeps_completed_results_and_is_repeatable_after_end() {
    let server = MockServer::builder().start().await;
    let ticket = submit(&server).await;
    state(&server, "in_progress").await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/batches/msgbatch_test/cancel"))
        .respond_with(ResponseTemplate::new(200).set_body_json(job("canceling")))
        .mount(&server)
        .await;
    assert_eq!(
        client(&server, "tenant-a/account-a")
            .cancel(&ticket)
            .await
            .unwrap(),
        BatchStatus::Cancelling
    );
    assert_eq!(
        client(&server, "tenant-a/account-a")
            .collect(&ticket)
            .await
            .unwrap_err(),
        BatchError::NotReady
    );
    server.reset().await;
    state(&server, "ended").await;
    results(
        &server,
        vec![
            success("first", "already finished"),
            json!({"custom_id":"second","result":{"type":"canceled"}}),
        ],
    )
    .await;
    let batch = client(&server, "tenant-a/account-a");
    assert_eq!(batch.cancel(&ticket).await.unwrap(), BatchStatus::Ended);
    let output = batch.collect(&ticket).await.unwrap();
    assert_eq!(output[0].result.as_ref().unwrap().text, "already finished");
    assert_eq!(
        output[1].result.as_ref().unwrap_err(),
        &BatchItemError::Cancelled
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET")
    );
}

#[tokio::test]
async fn missing_duplicate_and_unknown_ids_fail_collection() {
    for rows in [
        vec![success("first", "a")],
        vec![success("first", "a"), success("first", "b")],
        vec![success("first", "a"), success("stranger", "b")],
    ] {
        let server = MockServer::builder().start().await;
        let ticket = submit(&server).await;
        state(&server, "ended").await;
        results(&server, rows).await;
        let error = client(&server, "tenant-a/account-a")
            .collect(&ticket)
            .await
            .unwrap_err();
        assert!(matches!(error, BatchError::Protocol(_)), "{error:?}");
    }
}

#[tokio::test]
async fn tenant_account_provider_endpoint_and_ticket_version_are_checked_before_io() {
    let server = MockServer::builder().start().await;
    let ticket = submit(&server).await;
    let initial = server.received_requests().await.unwrap().len();
    for scope in ["tenant-b/account-a", "tenant-a/account-b"] {
        let batch = client(&server, scope);
        assert_eq!(
            batch.poll(&ticket).await.unwrap_err(),
            BatchError::ScopeMismatch
        );
        assert_eq!(
            batch.cancel(&ticket).await.unwrap_err(),
            BatchError::ScopeMismatch
        );
        assert_eq!(
            batch.collect(&ticket).await.unwrap_err(),
            BatchError::ScopeMismatch
        );
    }
    let other = MockServer::builder().start().await;
    assert_eq!(
        client(&other, "tenant-a/account-a")
            .poll(&ticket)
            .await
            .unwrap_err(),
        BatchError::ScopeMismatch
    );
    assert!(other.received_requests().await.unwrap().is_empty());
    for (field, value) in [
        ("version", json!(99)),
        ("job_id", json!("../../secrets?x=y")),
        ("request_ids", json!(["first", "first"])),
        ("provider", json!("other")),
        ("format", json!("other-v2")),
    ] {
        let mut edited = serde_json::to_value(&ticket).unwrap();
        edited[field] = value;
        let edited: BatchTicket = serde_json::from_value(edited).unwrap();
        assert!(matches!(
            client(&server, "tenant-a/account-a").poll(&edited).await,
            Err(BatchError::InvalidTicket | BatchError::ScopeMismatch)
        ));
    }
    assert_eq!(server.received_requests().await.unwrap().len(), initial);
}

#[tokio::test]
async fn credentials_are_resolved_from_the_current_client_and_can_rotate() {
    let server = MockServer::builder().start().await;
    let ticket = submit(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/messages/batches/msgbatch_test"))
        .and(header("x-api-key", "rotated-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(job("ended")))
        .mount(&server)
        .await;
    let rotated = Batch::new(
        Anthropic::new("rotated-key").base_url(format!("{}/v1", server.uri())),
        "tenant-a/account-a",
    );
    assert_eq!(rotated.poll(&ticket).await.unwrap(), BatchStatus::Ended);
    let calls = server.received_requests().await.unwrap();
    assert_eq!(calls[0].headers.get("x-api-key").unwrap(), "test-key");
    assert_eq!(calls[1].headers.get("x-api-key").unwrap(), "rotated-key");
}

#[tokio::test]
async fn unknown_creation_outcomes_are_never_retried() {
    for response in [
        ResponseTemplate::new(500).set_body_string("private provider error"),
        ResponseTemplate::new(200).set_body_string("broken"),
        ResponseTemplate::new(200)
            .set_body_json(job("in_progress"))
            .set_delay(Duration::from_millis(200)),
    ] {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages/batches"))
            .respond_with(response)
            .mount(&server)
            .await;
        let error = client(&server, "tenant-a/account-a")
            .timeout(Duration::from_millis(30))
            .request("one", request())
            .unwrap()
            .submit()
            .await
            .unwrap_err();
        assert_eq!(error, BatchError::SubmissionUncertain);
        assert!(!error.retryable());
        assert!(!error.to_string().contains("private"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn poll_failures_can_retry_the_same_ticket_without_recreating_the_job() {
    let server = MockServer::builder().start().await;
    let ticket = submit(&server).await;
    Mock::given(method("GET"))
        .and(path("/v1/messages/batches/msgbatch_test"))
        .respond_with(ResponseTemplate::new(503).set_body_string("private prompt test-key"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    let batch = client(&server, "tenant-a/account-a");
    let error = batch.poll(&ticket).await.unwrap_err();
    assert_eq!(error, BatchError::Http { status: 503 });
    assert!(error.retryable());
    assert!(!error.to_string().contains("test-key"));
    state(&server, "ended").await;
    assert_eq!(batch.poll(&ticket).await.unwrap(), BatchStatus::Ended);
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn expired_and_rejected_items_are_distinct_from_job_errors() {
    for (kind, expected) in [
        ("expired", BatchItemError::Expired),
        ("errored", BatchItemError::Server),
    ] {
        let server = MockServer::builder().start().await;
        let ticket = submit(&server).await;
        state(&server, "ended").await;
        results(&server,vec![success("first","ok"),json!({"custom_id":"second","result":{"type":kind,"error":{"type":"error","error":{"type":"api_error","message":"private"}}}})]).await;
        let output = client(&server, "tenant-a/account-a")
            .collect(&ticket)
            .await
            .unwrap();
        assert_eq!(output[1].result.as_ref().unwrap_err(), &expected);
    }
}

#[tokio::test]
async fn local_validation_and_unsupported_providers_never_send() {
    let server = MockServer::builder().start().await;
    assert_eq!(
        client(&server, "scope").submit().await.unwrap_err(),
        BatchError::InvalidRequest("batch has no requests")
    );
    for id in ["", "two words", "../one", "α", &"a".repeat(65)] {
        assert!(client(&server, "scope").request(id, request()).is_err());
    }
    assert!(
        client(&server, "scope")
            .request("one", request())
            .unwrap()
            .request("one", request())
            .is_err()
    );
    assert!(
        client(&server, "scope")
            .request("one", Model::simulated("answer").completion().user("hello"))
            .is_err()
    );
    assert!(
        client(&server, "scope")
            .request("one", Model::from("claude-sonnet-5").completion())
            .is_err()
    );
    assert!(matches!(
        client(&server, "scope")
            .request("one", request().first_token_timeout(Duration::from_secs(1)))
            .unwrap()
            .submit()
            .await,
        Err(BatchError::InvalidRequest(_))
    ));
    assert!(matches!(
        client(&server, "scope")
            .request("one", request().capture_request(true))
            .unwrap()
            .submit()
            .await,
        Err(BatchError::InvalidRequest(_))
    ));
    assert!(matches!(
        client(&server, "scope")
            .request(
                "one",
                request().response_format(everruns::llm::ResponseFormat::json_schema(
                    "answer",
                    json!({"type":"object"})
                ))
            )
            .unwrap()
            .submit()
            .await,
        Err(BatchError::InvalidRequest(_))
    ));
    let provider = everruns::Provider::new("no-batch", NoBatch);
    assert_eq!(
        Batch::new(provider, "scope")
            .request("one", request())
            .unwrap()
            .submit()
            .await
            .unwrap_err(),
        BatchError::Unsupported
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}
struct NoBatch;
#[async_trait::async_trait]
impl everruns::ChatDriver for NoBatch {
    async fn chat_completion_stream(
        &self,
        _: &everruns::ProviderEndpoint,
        _: Vec<everruns::llm::Message>,
        _: &everruns::LlmCallConfig,
    ) -> Result<everruns::LlmResponseStream, everruns::AgentLoopError> {
        unreachable!()
    }
}

#[tokio::test]
async fn bounded_reads_and_redirects_fail_without_leaking_data() {
    let server = MockServer::builder().start().await;
    let ticket = submit(&server).await;
    state(&server, "ended").await;
    results(
        &server,
        vec![success("first", &"x".repeat(1000)), success("second", "b")],
    )
    .await;
    assert_eq!(
        client(&server, "tenant-a/account-a")
            .max_response_bytes(512)
            .collect(&ticket)
            .await
            .unwrap_err(),
        BatchError::ResponseTooLarge
    );
    server.reset().await;
    let sink = MockServer::builder().start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", sink.uri()))
        .mount(&server)
        .await;
    assert_eq!(
        client(&server, "tenant-a/account-a")
            .poll(&ticket)
            .await
            .unwrap_err(),
        BatchError::Http { status: 302 }
    );
    assert!(sink.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn definite_rejections_and_provider_retention_are_not_uncertain_creation() {
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).set_body_string("private provider details"))
        .mount(&server)
        .await;
    let error = client(&server, "scope")
        .request("one", request())
        .unwrap()
        .submit()
        .await
        .unwrap_err();
    assert_eq!(error, BatchError::Http { status: 429 });
    assert!(error.retryable());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    server.reset().await;
    let ticket = submit(&server).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let error = client(&server, "tenant-a/account-a")
        .poll(&ticket)
        .await
        .unwrap_err();
    assert_eq!(error, BatchError::Http { status: 404 });
    assert!(!error.retryable());
}

#[tokio::test]
async fn malformed_result_files_fail_without_partial_success() {
    for body in [
        "not json",
        "{\"custom_id\":\"first\",\"result\":{\"type\":\"future_state\"}}",
        "{\"custom_id\":\"first\",\"result\":{\"type\":\"succeeded\",\"message\":{}}}",
    ] {
        let server = MockServer::builder().start().await;
        let ticket = submit(&server).await;
        state(&server, "ended").await;
        Mock::given(method("GET"))
            .and(path("/v1/messages/batches/msgbatch_test/results"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(&server)
            .await;
        assert!(matches!(
            client(&server, "tenant-a/account-a").collect(&ticket).await,
            Err(BatchError::Protocol(_))
        ));
    }
}

#[tokio::test]
async fn nested_provider_error_envelopes_keep_actionable_item_categories() {
    for (kind, expected) in [
        ("invalid_request_error", BatchItemError::InvalidRequest),
        ("authentication_error", BatchItemError::Authentication),
        ("permission_error", BatchItemError::Permission),
        ("rate_limit_error", BatchItemError::RateLimited),
        ("api_error", BatchItemError::Server),
        ("overloaded_error", BatchItemError::Server),
        ("future_error", BatchItemError::Other),
    ] {
        let server = MockServer::builder().start().await;
        let ticket = submit(&server).await;
        state(&server, "ended").await;
        results(&server, vec![success("first", "ok"), json!({"custom_id":"second","result":{"type":"errored","error":{"type":"error","error":{"type":kind,"message":"private"}}}})]).await;
        let output = client(&server, "tenant-a/account-a")
            .collect(&ticket)
            .await
            .unwrap();
        assert_eq!(output[1].result.as_ref().unwrap_err(), &expected);
    }
}

#[tokio::test]
async fn reconciled_jobs_recover_a_ticket_without_resubmission() {
    let server = MockServer::builder().start().await;
    state(&server, "ended").await;
    results(
        &server,
        vec![success("second", "two"), success("first", "one")],
    )
    .await;
    let batch = client(&server, "tenant-a/account-a");
    let ticket = batch
        .recover("msgbatch_test", ["first", "second"])
        .await
        .unwrap();
    let output = batch.collect(&ticket).await.unwrap();
    assert_eq!(output[0].id, "first");
    assert_eq!(output[0].result.as_ref().unwrap().text, "one");
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.method == "GET")
    );
    let count = server.received_requests().await.unwrap().len();
    assert!(
        batch
            .recover("msgbatch_test", ["same", "same"])
            .await
            .is_err()
    );
    assert!(
        batch
            .recover("msgbatch_test", Vec::<String>::new())
            .await
            .is_err()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), count);
}

fn message_request(id: &str, text: &str) -> BatchRequest {
    use everruns::llm::{Message, MessageRole};
    let mut config = LlmCallConfig::new("claude-sonnet-5");
    config.max_tokens = Some(64);
    BatchRequest {
        id: id.into(),
        messages: vec![
            Message::text(MessageRole::System, "Classify the text."),
            Message::text(MessageRole::User, text),
        ],
        config,
    }
}

#[tokio::test]
async fn message_lists_submit_and_collect_through_the_same_client() {
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/batches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(job("in_progress")))
        .mount(&server)
        .await;
    state(&server, "ended").await;
    results(
        &server,
        vec![success("second", "two"), success("first", "one")],
    )
    .await;
    let batch = client(&server, "tenant-a/account-a");
    let ticket = batch
        .submit_requests(vec![
            message_request("first", "Delivery arrived early."),
            message_request("second", "The replacement does not turn on."),
        ])
        .await
        .unwrap();
    let saved = serde_json::to_vec(&ticket).unwrap();
    let ticket: BatchTicket = serde_json::from_slice(&saved).unwrap();
    let collected = batch.collect(&ticket).await.unwrap();
    assert_eq!(collected[0].id, "first");
    assert_eq!(collected[0].result.as_ref().unwrap().text, "one");
    assert_eq!(collected[1].id, "second");
    let calls = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&calls[0].body).unwrap();
    assert_eq!(body["requests"][0]["params"]["model"], "claude-sonnet-5");
    assert_eq!(body["requests"][0]["params"]["max_tokens"], 64);
    assert_eq!(
        body["requests"][0]["params"]["system"],
        "Classify the text."
    );
    assert_eq!(
        body["requests"][0]["params"]["messages"][0]["content"][0]["text"],
        "Delivery arrived early."
    );
    assert_eq!(
        body["requests"][1]["params"]["messages"][0]["content"][0]["text"],
        "The replacement does not turn on."
    );
}

#[tokio::test]
async fn message_list_validation_rejects_the_whole_batch_before_io() {
    let server = MockServer::builder().start().await;
    let batch = client(&server, "tenant-a/account-a");
    let mut no_messages = message_request("empty", "text");
    no_messages.messages.clear();
    for requests in [
        vec![],
        vec![
            message_request("duplicate", "one"),
            message_request("duplicate", "two"),
        ],
        vec![
            message_request("ok", "one"),
            message_request("../invalid", "two"),
        ],
        vec![message_request("ok", "one"), no_messages],
    ] {
        assert!(matches!(
            batch.submit_requests(requests).await,
            Err(BatchError::InvalidRequest(_))
        ));
    }
    let mut per_request_auth = message_request("auth", "text");
    per_request_auth
        .config
        .extra_headers
        .push(("x-api-key".into(), "other-account-key".into()));
    assert!(matches!(
        batch.submit_requests(vec![per_request_auth]).await,
        Err(BatchError::InvalidRequest(_))
    ));
    let buffered = batch.request("buffered", request()).unwrap();
    assert!(matches!(
        buffered
            .submit_requests(vec![message_request("explicit", "two")])
            .await,
        Err(BatchError::InvalidRequest(_))
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn message_list_submission_preserves_multipart_messages() {
    use everruns::{
        LlmContentPart,
        llm::{Message, MessageRole},
    };
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/batches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(job("in_progress")))
        .mount(&server)
        .await;
    let mut item = message_request("image", "unused");
    let messages: Vec<everruns::llm::Message> = vec![Message::parts(
        MessageRole::User,
        vec![
            LlmContentPart::text("Describe this image."),
            LlmContentPart::Image {
                url: "data:image/png;base64,aW1n".into(),
            },
        ],
    )];
    item.messages = messages;
    client(&server, "tenant-a/account-a")
        .submit_requests(vec![item])
        .await
        .unwrap();
    let calls = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&calls[0].body).unwrap();
    let content = &body["requests"][0]["params"]["messages"][0]["content"];
    assert_eq!(content[0]["text"], "Describe this image.");
    assert_eq!(
        content[1],
        json!({"type":"image", "source":{"type":"base64", "media_type":"image/png", "data":"aW1n"}})
    );
}

#[tokio::test]
async fn message_list_submission_keeps_uncertain_creations_non_retryable() {
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/batches"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let error = client(&server, "tenant-a/account-a")
        .submit_requests(vec![message_request("first", "text")])
        .await
        .unwrap_err();
    assert_eq!(error, BatchError::SubmissionUncertain);
    assert!(!error.retryable());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn message_lists_accept_converted_core_conversations() {
    use everruns_core::{RuntimeMessage, llm_conversions::llm_message_from_message};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/batches"))
        .respond_with(ResponseTemplate::new(200).set_body_json(job("in_progress")))
        .mount(&server)
        .await;
    let conversation = [
        RuntimeMessage::system("Existing core instructions."),
        RuntimeMessage::user("Existing core user input."),
    ];
    let mut item = message_request("core", "unused");
    item.messages = conversation.iter().map(llm_message_from_message).collect();
    client(&server, "tenant-a/account-a")
        .submit_requests(vec![item])
        .await
        .unwrap();
    let calls = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&calls[0].body).unwrap();
    assert_eq!(
        body["requests"][0]["params"]["system"],
        "Existing core instructions."
    );
    assert_eq!(body["requests"][0]["params"]["messages"][0]["role"], "user");
    assert_eq!(
        body["requests"][0]["params"]["messages"][0]["content"][0]["text"],
        "Existing core user input."
    );
}
