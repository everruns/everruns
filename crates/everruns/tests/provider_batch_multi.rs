//! Public-API smoke tests use real provider encoders and deterministic HTTP peers.
#![cfg(all(feature = "openai", feature = "gemini"))]
use everruns::{
    LlmCallConfig, Provider,
    batch::{Batch, BatchError, BatchItemError, BatchRequest, BatchStatus, BatchTicket},
    llm::{Message, MessageRole},
    providers::{
        gemini::Gemini,
        openai::{OpenAI, OpenAICompletionsChatDriver},
    },
};
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

#[derive(Clone, Copy, Debug)]
enum Api {
    Responses,
    Completions,
    Gemini,
    #[cfg(feature = "groq")]
    Groq,
}
const APIS: &[Api] = &[
    Api::Responses,
    Api::Completions,
    Api::Gemini,
    #[cfg(feature = "groq")]
    Api::Groq,
];
impl Api {
    fn provider(self, server: &MockServer, key: &str) -> Provider {
        let base = format!("{}/v1", server.uri());
        match self {
            Self::Responses => OpenAI::new(key).base_url(base).into(),
            #[cfg(feature = "groq")]
            Self::Groq => everruns::providers::groq::Groq::new(key)
                .base_url(base)
                .into(),
            Self::Completions => Provider::new("openai", OpenAICompletionsChatDriver::new())
                .base_url(base)
                .auth(everruns::BearerAuth::new(key)),
            Self::Gemini => Gemini::new(key).base_url(base).into(),
        }
    }
    fn client(self, server: &MockServer) -> Batch {
        Batch::new(self.provider(server, "synthetic-key"), "tenant/account")
    }
    fn model(self) -> &'static str {
        match self {
            Self::Gemini => "gemini-4-argon",
            _ => "gpt-6.1-sol",
        }
    }
    fn url(self) -> &'static str {
        match self {
            Self::Responses => "/v1/responses",
            Self::Completions => "/v1/chat/completions",
            #[cfg(feature = "groq")]
            Self::Groq => "/v1/chat/completions",
            Self::Gemini => "",
        }
    }
    fn id(self) -> &'static str {
        match self {
            Self::Gemini => "batches/job_1",
            _ => "batch_test",
        }
    }
    fn submit_path(self) -> String {
        match self {
            Self::Gemini => format!("/v1/models/{}:batchGenerateContent", self.model()),
            _ => "/v1/batches".into(),
        }
    }
    fn job_path(self) -> String {
        match self {
            Self::Gemini => "/v1/batches/job_1".into(),
            _ => "/v1/batches/batch_test".into(),
        }
    }
    fn request(self, id: &str) -> BatchRequest {
        let mut config = LlmCallConfig::new(self.model());
        config.max_tokens = Some(64);
        BatchRequest {
            id: id.into(),
            messages: vec![
                Message::text(MessageRole::System, "Classify."),
                Message::text(MessageRole::User, "private prompt"),
            ],
            config,
        }
    }
    fn job(self, ended: bool) -> Value {
        match self {
            Self::Gemini => {
                json!({"name":self.id(),"done":ended,"metadata":{"model":format!("models/{}",self.model())}})
            }
            _ => {
                json!({"id":self.id(),"endpoint":self.url(),"status":if ended {"completed"} else {"in_progress"},"request_counts":{"total":2,"completed":1,"failed":1},"output_file_id":"file-good","error_file_id":"file-bad"})
            }
        }
    }
    fn success(self, text: &str) -> Value {
        match self {
            Self::Responses => {
                json!({"id":"resp_1","object":"response","created_at":0,"status":"completed","model":self.model(),"output":[{"type":"message","id":"msg_1","status":"completed","role":"assistant","content":[{"type":"output_text","text":text}]}],"usage":{"input_tokens":10,"output_tokens":3,"total_tokens":13}})
            }
            #[cfg(feature = "groq")]
            Self::Groq => Api::Completions.success(text),
            Self::Completions => {
                json!({"id":"chatcmpl_1","model":self.model(),"choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13}})
            }
            Self::Gemini => {
                json!({"modelVersion":self.model(),"candidates":[{"content":{"role":"model","parts":[{"text":text}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":10,"candidatesTokenCount":3,"totalTokenCount":13}})
            }
        }
    }
}
async fn mock(server: &MockServer, verb: &str, p: &str, value: Value) {
    Mock::given(method(verb))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(server)
        .await;
}
async fn creation(api: Api, server: &MockServer) -> BatchTicket {
    if !matches!(api, Api::Gemini) {
        mock(server, "POST", "/v1/files", json!({"id":"file-input"})).await;
    }
    mock(server, "POST", &api.submit_path(), api.job(false)).await;
    api.client(server)
        .submit_requests(vec![api.request("first"), api.request("second")])
        .await
        .unwrap()
}
async fn completed(api: Api, server: &MockServer) {
    let mut job = api.job(true);
    if matches!(api, Api::Gemini) {
        job["response"] = json!({"inlinedResponses":{"inlinedResponses":[{"metadata":{"key":"second"},"error":{"code":8,"message":"secret provider detail"}},{"metadata":{"key":"first"},"response":api.success("positive")}]}});
    } else {
        for (p, row) in [
            (
                "/v1/files/file-good/content",
                json!({"custom_id":"first","response":{"status_code":200,"body":api.success("positive")}}),
            ),
            (
                "/v1/files/file-bad/content",
                json!({"custom_id":"second","response":{"status_code":429,"body":{"error":{"message":"secret provider detail"}}}}),
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ResponseTemplate::new(200).set_body_string(format!("{row}\n")))
                .mount(server)
                .await;
        }
    }
    mock(server, "GET", &api.job_path(), job).await;
}
#[tokio::test]
async fn all_drivers_submit_persist_reconstruct_and_collect_by_id() {
    for &api in APIS {
        let server = MockServer::builder().start().await;
        assert!(api.provider(&server, "synthetic-key").supports_batch());
        let ticket = creation(api, &server).await;
        let saved = serde_json::to_string(&ticket).unwrap();
        for private in [
            "synthetic-key",
            "private prompt",
            "Classify.",
            &server.uri(),
        ] {
            assert!(!saved.contains(private));
        }
        let ticket: BatchTicket = serde_json::from_str(&saved).unwrap();
        completed(api, &server).await;
        let client = Batch::new(api.provider(&server, "rotated-key"), "tenant/account");
        for _ in 0..2 {
            let rows = client.collect(&ticket).await.unwrap();
            assert_eq!(
                rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
                ["first", "second"],
                "{api:?}"
            );
            let response = rows[0].result.as_ref().unwrap();
            assert_eq!(response.text, "positive");
            assert_eq!(response.metadata.total_tokens, Some(13));
            assert_eq!(
                rows[1].result.as_ref().unwrap_err(),
                &BatchItemError::RateLimited
            );
        }
        let calls = server.received_requests().await.unwrap();
        let post = calls
            .iter()
            .find(|r| r.url.path() == api.submit_path())
            .unwrap();
        let body: Value = serde_json::from_slice(&post.body).unwrap();
        if matches!(api, Api::Gemini) {
            assert_eq!(
                body["batch"]["inputConfig"]["requests"]["requests"][0]["metadata"]["key"],
                "first"
            );
            assert_eq!(
                body["batch"]["inputConfig"]["requests"]["requests"][0]["request"]["generationConfig"]
                    ["maxOutputTokens"],
                64
            );
        } else {
            assert_eq!(body["endpoint"], api.url());
            assert_eq!(body["completion_window"], "24h");
            let upload = calls.iter().find(|r| r.url.path() == "/v1/files").unwrap();
            let multipart = std::str::from_utf8(&upload.body).unwrap();
            let lines: Vec<Value> = multipart
                .lines()
                .filter(|l| l.starts_with('{'))
                .map(|l| serde_json::from_str(l).unwrap())
                .collect();
            assert_eq!(lines.len(), 2);
            assert_eq!(lines[0]["custom_id"], "first");
            assert_eq!(lines[0]["body"]["stream"], false);
            assert!(!multipart.contains("\"background\""));
            assert!(multipart.contains("private prompt"));
        }
        let expected_posts = if matches!(api, Api::Gemini) { 1 } else { 2 };
        assert_eq!(
            calls.iter().filter(|r| r.method == "POST").count(),
            expected_posts,
            "dropping submitting clients must preserve owned jobs"
        );
        assert!(calls.iter().filter(|r| r.method == "GET").all(|r| {
            r.headers
                .values()
                .any(|v| v.to_str().unwrap().contains("rotated-key"))
        }));
    }
}
#[tokio::test]
async fn creation_failures_never_replay_upload_or_job_and_sanitize_errors() {
    for &api in APIS {
        for bad in [
            ResponseTemplate::new(503).set_body_string("secret body"),
            ResponseTemplate::new(200)
                .set_body_json(json!({"name":"https://evil.invalid","id":"../escape"})),
            ResponseTemplate::new(200).set_body_string("not JSON"),
        ] {
            let server = MockServer::builder().start().await;
            if !matches!(api, Api::Gemini) {
                mock(&server, "POST", "/v1/files", json!({"id":"file-input"})).await;
            }
            Mock::given(method("POST"))
                .and(path(api.submit_path()))
                .respond_with(bad)
                .expect(1)
                .mount(&server)
                .await;
            let error = api
                .client(&server)
                .submit_requests(vec![api.request("first")])
                .await
                .unwrap_err();
            assert_eq!(error, BatchError::SubmissionUncertain);
            assert!(!error.retryable());
            assert!(!error.to_string().contains("secret"));
            assert_eq!(
                server.received_requests().await.unwrap().len(),
                if matches!(api, Api::Gemini) { 1 } else { 2 }
            );
        }
    }
}
#[tokio::test]
async fn uncertain_upload_never_creates_a_job() {
    for api in [Api::Responses, Api::Completions] {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .and(path("/v1/files"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            api.client(&server)
                .submit_requests(vec![api.request("one")])
                .await
                .unwrap_err(),
            BatchError::SubmissionUncertain
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}
#[tokio::test]
async fn direct_requests_reject_continuations_options_and_mixed_models_before_network() {
    for &api in APIS {
        let server = MockServer::builder().start().await;
        let mut first = api.request("one");
        let mut second = api.request("two");
        second.config.model = "different".into();
        assert!(matches!(
            api.client(&server)
                .submit_requests(vec![first.clone(), second])
                .await,
            Err(BatchError::InvalidRequest(_))
        ));
        first.config.previous_response_id = Some("resp_private".into());
        assert!(matches!(
            api.client(&server).submit_requests(vec![first]).await,
            Err(BatchError::InvalidRequest(_))
        ));
        let mut request = api.request("one");
        request
            .config
            .extra_headers
            .push(("authorization".into(), "override".into()));
        assert!(matches!(
            api.client(&server).submit_requests(vec![request]).await,
            Err(BatchError::InvalidRequest(_))
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}
#[tokio::test]
async fn scoped_tickets_reject_credentials_from_other_tenants_and_formats() {
    for &api in APIS {
        let server = MockServer::builder().start().await;
        let ticket = creation(api, &server).await;
        server.reset().await;
        assert_eq!(
            Batch::new(api.provider(&server, "other-key"), "other-tenant/account")
                .poll(&ticket)
                .await
                .unwrap_err(),
            BatchError::ScopeMismatch
        );
        let other = MockServer::builder().start().await;
        assert_eq!(
            api.client(&other).poll(&ticket).await.unwrap_err(),
            BatchError::ScopeMismatch
        );
        if matches!(api, Api::Responses) {
            assert_eq!(
                Api::Completions
                    .client(&server)
                    .poll(&ticket)
                    .await
                    .unwrap_err(),
                BatchError::ScopeMismatch
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(other.received_requests().await.unwrap().is_empty());
    }
}
#[tokio::test]
async fn cancelled_openai_jobs_keep_successes_and_identify_only_unprocessed_items() {
    for api in [Api::Responses, Api::Completions] {
        for (native, reason) in [
            ("cancelled", BatchItemError::Cancelled),
            ("expired", BatchItemError::Expired),
        ] {
            let server = MockServer::builder().start().await;
            let ticket = creation(api, &server).await;
            server.reset().await;
            let mut job = api.job(true);
            job["status"] = json!(native);
            job["error_file_id"] = Value::Null;
            job["request_counts"]["failed"] = json!(0);
            mock(&server, "GET", &api.job_path(), job.clone()).await;
            Mock::given(method("GET")).and(path("/v1/files/file-good/content")).respond_with(ResponseTemplate::new(200).set_body_string(json!({"custom_id":"second","response":{"status_code":200,"body":api.success("already finished")}}).to_string())).mount(&server).await;
            let rows = api.client(&server).collect(&ticket).await.unwrap();
            assert_eq!(rows[0].result.as_ref().unwrap_err(), &reason);
            assert_eq!(rows[1].result.as_ref().unwrap().text, "already finished");
            server.reset().await;
            job["output_file_id"] = Value::Null;
            mock(&server, "GET", &api.job_path(), job).await;
            assert!(
                matches!(
                    api.client(&server).collect(&ticket).await,
                    Err(BatchError::Protocol(_))
                ),
                "missing successful files must not become cancelled results"
            );
        }
    }
}
#[tokio::test]
async fn gemini_cancellation_is_explicit_and_global_failure_is_not_fake_item_data() {
    let api = Api::Gemini;
    let server = MockServer::builder().start().await;
    let ticket = creation(api, &server).await;
    server.reset().await;
    mock(&server, "GET", &api.job_path(), api.job(false)).await;
    Mock::given(method("POST"))
        .and(path("/v1/batches/job_1:cancel"))
        .and(header("x-goog-api-key", "synthetic-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        api.client(&server).cancel(&ticket).await.unwrap(),
        BatchStatus::Cancelling
    );
    server.reset().await;
    let mut job = api.job(true);
    job["error"] = json!({"code":1,"message":"sensitive detail"});
    mock(&server, "GET", &api.job_path(), job).await;
    assert_eq!(
        api.client(&server).collect(&ticket).await.unwrap_err(),
        BatchError::JobFailed {
            reason: BatchItemError::Cancelled
        }
    );
    assert_eq!(
        api.client(&server).cancel(&ticket).await.unwrap(),
        BatchStatus::Ended
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
async fn retryable_reads_are_explicit_and_redirects_and_size_limits_fail_closed() {
    for &api in APIS {
        let server = MockServer::builder().start().await;
        let ticket = creation(api, &server).await;
        server.reset().await;
        Mock::given(method("GET"))
            .and(path(api.job_path()))
            .respond_with(ResponseTemplate::new(429).set_body_string("private detail"))
            .expect(1)
            .mount(&server)
            .await;
        let error = api.client(&server).poll(&ticket).await.unwrap_err();
        assert_eq!(error, BatchError::Http { status: 429 });
        assert!(error.retryable());
        server.reset().await;
        mock(&server, "GET", &api.job_path(), api.job(true)).await;
        assert_eq!(
            api.client(&server)
                .max_response_bytes(10)
                .poll(&ticket)
                .await
                .unwrap_err(),
            BatchError::ResponseTooLarge
        );
        server.reset().await;
        Mock::given(method("GET"))
            .and(path(api.job_path()))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", "https://attacker.invalid"),
            )
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            api.client(&server).poll(&ticket).await.unwrap_err(),
            BatchError::Http { status: 302 }
        );
    }
}
#[tokio::test]
async fn submit_timeout_is_uncertain_without_retry() {
    let api = Api::Gemini;
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path(api.submit_path()))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(100))
                .set_body_json(api.job(false)),
        )
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        api.client(&server)
            .timeout(Duration::from_millis(10))
            .submit_requests(vec![api.request("one")])
            .await
            .unwrap_err(),
        BatchError::SubmissionUncertain
    );
}

#[tokio::test]
async fn all_drivers_recover_without_creating_work_and_reject_unsafe_resource_ids() {
    for &api in APIS {
        let server = MockServer::builder().start().await;
        mock(&server, "GET", &api.job_path(), api.job(false)).await;
        let ticket = api
            .client(&server)
            .recover(api.id(), ["first", "second"])
            .await
            .unwrap();
        assert_eq!(ticket.job_id(), api.id());
        assert_eq!(
            api.client(&server).poll(&ticket).await.unwrap(),
            BatchStatus::Processing
        );
        assert_eq!(
            api.client(&server)
                .recover("https://attacker.invalid/job", ["one"])
                .await
                .unwrap_err(),
            BatchError::InvalidTicket
        );
        assert_eq!(
            api.client(&server)
                .recover("../private", ["one"])
                .await
                .unwrap_err(),
            BatchError::InvalidTicket
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
}
#[tokio::test]
async fn openai_cancellation_and_global_validation_failure_remain_job_operations() {
    for api in [Api::Responses, Api::Completions] {
        let server = MockServer::builder().start().await;
        let ticket = creation(api, &server).await;
        server.reset().await;
        let mut job = api.job(false);
        job["status"] = json!("cancelling");
        mock(&server, "GET", &api.job_path(), api.job(false)).await;
        mock(&server, "POST", "/v1/batches/batch_test/cancel", job).await;
        assert_eq!(
            api.client(&server).cancel(&ticket).await.unwrap(),
            BatchStatus::Cancelling
        );
        server.reset().await;
        let mut failed = api.job(true);
        failed["status"] = json!("failed");
        failed["errors"] =
            json!({"data":[{"code":"invalid_json_line","message":"sensitive detail"}]});
        mock(&server, "GET", &api.job_path(), failed).await;
        assert_eq!(
            api.client(&server).collect(&ticket).await.unwrap_err(),
            BatchError::JobFailed {
                reason: BatchItemError::InvalidRequest
            }
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.url.path() == api.job_path())
        );
    }
}
#[tokio::test]
async fn provider_result_identity_mismatches_fail_without_partial_results() {
    for &api in APIS {
        for ids in [["first", "first"], ["first", "unknown"], ["first", ""]] {
            let server = MockServer::builder().start().await;
            let ticket = creation(api, &server).await;
            server.reset().await;
            let mut job = api.job(true);
            if matches!(api, Api::Gemini) {
                job["metadata"]["output"] = json!({"inlinedResponses":{"inlinedResponses":ids.into_iter().filter(|id|!id.is_empty()).map(|id|json!({"metadata":{"key":id},"response":api.success("ok")})).collect::<Vec<_>>()}});
            } else {
                job["error_file_id"] = Value::Null;
                job["request_counts"]["completed"] = json!(2);
                job["request_counts"]["failed"] = json!(0);
                let rows=ids.into_iter().filter(|id|!id.is_empty()).map(|id|json!({"custom_id":id,"response":{"status_code":200,"body":api.success("ok")}}).to_string()).collect::<Vec<_>>().join("\n");
                Mock::given(method("GET"))
                    .and(path("/v1/files/file-good/content"))
                    .respond_with(ResponseTemplate::new(200).set_body_string(rows))
                    .mount(&server)
                    .await;
            }
            mock(&server, "GET", &api.job_path(), job).await;
            assert!(
                matches!(
                    api.client(&server).collect(&ticket).await,
                    Err(BatchError::Protocol(_))
                ),
                "{api:?}: {ids:?}"
            );
        }
    }
}
#[tokio::test]
async fn openai_never_downloads_untrusted_file_urls_or_relabels_missing_outputs() {
    let api = Api::Responses;
    let server = MockServer::builder().start().await;
    let ticket = creation(api, &server).await;
    server.reset().await;
    let mut job = api.job(true);
    job["output_file_id"] = json!("https://attacker.invalid/content");
    mock(&server, "GET", &api.job_path(), job).await;
    assert!(matches!(
        api.client(&server).collect(&ticket).await,
        Err(BatchError::Protocol(_))
    ));
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() == api.job_path())
    );
}
#[tokio::test]
async fn gemini_preserves_reasoning_tools_usage_and_inline_error_categories() {
    let api = Api::Gemini;
    let server = MockServer::builder().start().await;
    let ticket = creation(api, &server).await;
    server.reset().await;
    let mut success = api.success("answer");
    success["candidates"][0]["content"]["parts"] = json!([
        {"text":"summary","thought":true,"thoughtSignature":"signed-reasoning"},
        {"text":"answer"},
        {"functionCall":{"name":"lookup","args":{"optional":"as-is"}},"thoughtSignature":"signed-call"}
    ]);
    success["usageMetadata"]["cachedContentTokenCount"] = json!(3);
    success["usageMetadata"]["thoughtsTokenCount"] = json!(2);
    let mut job = api.job(true);
    job["response"] = json!({"inlinedResponses":{"inlinedResponses":[{"metadata":{"key":"first"},"response":success},{"metadata":{"key":"second"},"error":{"code":16,"message":"sensitive credential detail"}}]}});
    mock(&server, "GET", &api.job_path(), job).await;
    let rows = api.client(&server).collect(&ticket).await.unwrap();
    let response = rows[0].result.as_ref().unwrap();
    assert_eq!(response.text, "answer");
    assert!(!response.reasoning.is_empty());
    assert_eq!(response.tool_calls.as_ref().unwrap()[0].name, "lookup");
    assert_eq!(response.metadata.prompt_tokens, Some(7));
    assert_eq!(response.metadata.cache_read_tokens, Some(3));
    assert_eq!(response.metadata.reasoning_tokens, Some(2));
    assert_eq!(
        rows[1].result.as_ref().unwrap_err(),
        &BatchItemError::Authentication
    );
}

#[tokio::test]
async fn dynamically_boxed_drivers_keep_the_batch_capability_and_bound_adapters_fail_closed() {
    use everruns::providers::openai::OpenAIChatDriver;
    let server = MockServer::builder().start().await;
    let driver: Box<dyn everruns_contracts::ChatDriver> = Box::new(OpenAIChatDriver::new());
    let provider = Provider::new("openai", driver)
        .base_url(format!("{}/v1", server.uri()))
        .auth(everruns::BearerAuth::new("synthetic-key"));
    assert!(provider.supports_batch());
    let legacy = provider.clone().into_boxed_driver();
    assert!(
        legacy.batch_driver().is_none(),
        "endpoint-bound adapters must not hide ticket routing"
    );
    let mut registry = everruns_contracts::DriverRegistry::new();
    everruns::providers::openai::register_driver(&mut registry);
    let mut config = everruns_contracts::driver_registry::ProviderConfig::new(
        everruns_contracts::DriverId::OpenAI,
    );
    config.api_key = Some("synthetic-key".into());
    assert!(
        registry
            .create_chat_driver(&config)
            .unwrap()
            .batch_driver()
            .is_none()
    );
    mock(&server, "POST", "/v1/files", json!({"id":"file-input"})).await;
    mock(&server, "POST", "/v1/batches", Api::Responses.job(false)).await;
    let ticket = Batch::new(provider, "tenant/account")
        .submit_requests(vec![Api::Responses.request("one")])
        .await
        .unwrap();
    assert_eq!(ticket.job_id(), "batch_test");
}

#[tokio::test]
async fn openai_valid_refusals_are_item_failures_without_losing_other_results() {
    for api in [Api::Responses, Api::Completions] {
        let server = MockServer::builder().start().await;
        let ticket = creation(api, &server).await;
        server.reset().await;
        let mut refused = api.success("");
        if matches!(api, Api::Responses) {
            refused["output"][0]["content"] =
                json!([{"type":"refusal","refusal":"private refusal detail"}]);
        } else {
            refused["choices"][0]["message"]["refusal"] = json!("private refusal detail");
        }
        let mut job = api.job(true);
        job["error_file_id"] = Value::Null;
        job["request_counts"]["completed"] = json!(2);
        job["request_counts"]["failed"] = json!(0);
        mock(&server, "GET", &api.job_path(), job).await;
        let rows = [
            json!({"custom_id":"first","response":{"status_code":200,"body":refused}}),
            json!({"custom_id":"second","response":{"status_code":200,"body":api.success("still collectable")}}),
        ];
        Mock::given(method("GET"))
            .and(path("/v1/files/file-good/content"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(
                    rows.into_iter()
                        .map(|r| r.to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            )
            .mount(&server)
            .await;
        let rows = api.client(&server).collect(&ticket).await.unwrap();
        assert_eq!(rows[0].result.as_ref().unwrap_err(), &BatchItemError::Other);
        assert_eq!(rows[1].result.as_ref().unwrap().text, "still collectable");
    }
}
