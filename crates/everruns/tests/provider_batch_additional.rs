//! End-to-end native batch protocol peers. No paid provider requests.
#![cfg(all(feature = "mistral", feature = "fireworks", feature = "groq"))]
use everruns::{
    LlmCallConfig, Provider,
    batch::{Batch, BatchError, BatchItemError, BatchRequest, BatchStatus, BatchTicket},
    llm::{Message, MessageRole},
    providers::{
        fireworks::{Fireworks, FireworksBatchConfig},
        groq::Groq,
        mistral::Mistral,
    },
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, Request, Respond, ResponseTemplate,
    matchers::{method, path, path_regex},
};
fn request(id: &str) -> BatchRequest {
    let mut config = LlmCallConfig::new("test-model");
    config.max_tokens = Some(64);
    BatchRequest {
        id: id.into(),
        messages: vec![Message::text(MessageRole::User, "private input")],
        config,
    }
}
fn completion() -> Value {
    json!({"id":"completion","model":"test-model","choices":[{"message":{"role":"assistant","content":"positive"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13}})
}
fn success(id: &str) -> Value {
    json!({"custom_id":id,"response":{"status_code":200,"body":completion()},"error":null})
}
fn failure(id: &str) -> Value {
    json!({"custom_id":id,"response":{"status_code":429},"error":null})
}
fn mistral_job(status: &str) -> Value {
    json!({"id":"019b2bd7-96e7-7219-8c0b-45a73da50088","endpoint":"/v1/chat/completions","status":status,"total_requests":2,"completed_requests":if status=="SUCCESS" {2}else{0},"succeeded_requests":if status=="SUCCESS" {1}else{0},"failed_requests":if status=="SUCCESS" {1}else{0}})
}
const MJOB: &str = "/v1/batch/jobs/019b2bd7-96e7-7219-8c0b-45a73da50088";
async fn mock(server: &MockServer, verb: &str, p: &str, value: Value) {
    Mock::given(method(verb))
        .and(path(p))
        .respond_with(ResponseTemplate::new(200).set_body_json(value))
        .mount(server)
        .await;
}
async fn mistral_ticket(server: &MockServer) -> BatchTicket {
    mock(server, "POST", "/v1/batch/jobs", mistral_job("QUEUED")).await;
    Batch::new(
        Mistral::new("private-key").base_url(format!("{}/v1", server.uri())),
        "tenant/account",
    )
    .submit_requests(vec![request("one"), request("two")])
    .await
    .unwrap()
}
fn mistral_client(server: &MockServer) -> Batch {
    Batch::new(
        Mistral::new("rotated-key").base_url(format!("{}/v1", server.uri())),
        "tenant/account",
    )
}
#[tokio::test]
async fn mistral_native_inline_submission_resumes_and_maps_inline_or_file_results() {
    for files in [false, true] {
        let server = MockServer::start().await;
        let ticket = mistral_ticket(&server).await;
        let saved = serde_json::to_string(&ticket).unwrap();
        for secret in ["private input", "private-key", &server.uri()] {
            assert!(!saved.contains(secret));
        }
        let ticket: BatchTicket = serde_json::from_str(&saved).unwrap();
        let posted = server.received_requests().await.unwrap();
        let body: Value = serde_json::from_slice(&posted[0].body).unwrap();
        assert_eq!(body["endpoint"], "/v1/chat/completions");
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["requests"][0]["body"]["max_tokens"], 64);
        assert!(
            body["requests"][0]["body"]
                .get("max_completion_tokens")
                .is_none()
        );
        assert_eq!(
            posted.len(),
            1,
            "inline submit has a single external effect"
        );
        server.reset().await;
        let mut job = mistral_job("SUCCESS");
        if files {
            job["output_file"] = json!("019b2bd7-96e7-7219-8c0b-45a73da50089");
            Mock::given(method("GET"))
                .and(path(
                    "/v1/files/019b2bd7-96e7-7219-8c0b-45a73da50089/content",
                ))
                .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                    "{}\n{}\n",
                    failure("two"),
                    success("one")
                )))
                .mount(&server)
                .await;
        } else {
            job["outputs"] = json!([failure("two"), success("one")]);
        }
        mock(&server, "GET", MJOB, job).await;
        for _ in 0..2 {
            let rows = mistral_client(&server).collect(&ticket).await.unwrap();
            assert_eq!(
                rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
                ["one", "two"]
            );
            assert_eq!(rows[0].result.as_ref().unwrap().text, "positive");
            assert_eq!(
                rows[1].result.as_ref().unwrap_err(),
                &BatchItemError::RateLimited
            );
        }
        assert!(
            server.received_requests().await.unwrap().iter().all(|r| r
                .headers
                .get("authorization")
                .unwrap()
                == "Bearer rotated-key")
        );
    }
}
#[tokio::test]
async fn mistral_cancel_and_expiry_preserve_finished_rows_using_verified_counts() {
    for (status, reason) in [
        ("CANCELLED", BatchItemError::Cancelled),
        ("TIMEOUT_EXCEEDED", BatchItemError::Expired),
    ] {
        let server = MockServer::start().await;
        let ticket = mistral_ticket(&server).await;
        server.reset().await;
        let mut job = mistral_job(status);
        job["completed_requests"] = json!(1);
        job["succeeded_requests"] = json!(1);
        job["outputs"] = json!([success("one")]);
        mock(&server, "GET", MJOB, job).await;
        let rows = mistral_client(&server).collect(&ticket).await.unwrap();
        assert!(rows[0].result.is_ok());
        assert_eq!(rows[1].result.as_ref().unwrap_err(), &reason);
    }
    let server = MockServer::start().await;
    let ticket = mistral_ticket(&server).await;
    server.reset().await;
    mock(&server, "GET", MJOB, mistral_job("RUNNING")).await;
    mock(
        &server,
        "POST",
        &format!("{MJOB}/cancel"),
        mistral_job("CANCELLATION_REQUESTED"),
    )
    .await;
    assert_eq!(
        mistral_client(&server).cancel(&ticket).await.unwrap(),
        BatchStatus::Cancelling
    );
}
#[tokio::test]
async fn mistral_rejects_missing_duplicate_unknown_results_and_unsafe_remote_ids() {
    for rows in [
        json!([success("one")]),
        json!([success("one"), success("one")]),
        json!([success("one"), failure("unknown")]),
    ] {
        let server = MockServer::start().await;
        let ticket = mistral_ticket(&server).await;
        server.reset().await;
        let mut job = mistral_job("SUCCESS");
        job["outputs"] = rows;
        mock(&server, "GET", MJOB, job).await;
        let result = mistral_client(&server).collect(&ticket).await;
        assert!(
            matches!(result, Err(BatchError::Protocol(_))),
            "unexpected result: {result:?}"
        );
    }
    let server = MockServer::start().await;
    let ticket = mistral_ticket(&server).await;
    server.reset().await;
    let mut job = mistral_job("SUCCESS");
    job["output_file"] = json!("../private");
    mock(&server, "GET", MJOB, job).await;
    assert!(matches!(
        mistral_client(&server).collect(&ticket).await,
        Err(BatchError::Protocol(_))
    ));
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.url.path() == MJOB),
        "unsafe file IDs must never trigger an artifact request"
    );
}
#[tokio::test]
async fn mistral_uncertain_submission_is_never_replayed_and_reads_can_be_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_string("private detail"))
        .expect(1)
        .mount(&server)
        .await;
    let error = mistral_client(&server)
        .submit_requests(vec![request("one")])
        .await
        .unwrap_err();
    assert_eq!(error, BatchError::SubmissionUncertain);
    assert!(!error.retryable());
    server.reset().await;
    let ticket = mistral_ticket(&server).await;
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        mistral_client(&server)
            .poll(&ticket)
            .await
            .unwrap_err()
            .retryable()
    );
    server.reset().await;
    mock(&server, "GET", MJOB, mistral_job("QUEUED")).await;
    assert_eq!(
        mistral_client(&server).poll(&ticket).await.unwrap(),
        BatchStatus::Processing
    );
    assert_eq!(
        Batch::new(
            Mistral::new("key").base_url(format!("{}/v1", server.uri())),
            "other tenant"
        )
        .poll(&ticket)
        .await
        .unwrap_err(),
        BatchError::ScopeMismatch
    );
}
#[derive(Clone)]
struct Dataset;
impl Respond for Dataset {
    fn respond(&self, r: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&r.body).unwrap();
        ResponseTemplate::new(200).set_body_json(json!({"name":format!("accounts/account-1/datasets/{}",body["datasetId"].as_str().unwrap())}))
    }
}
#[derive(Clone)]
struct Job;
impl Respond for Job {
    fn respond(&self, r: &Request) -> ResponseTemplate {
        let id = r
            .url
            .query_pairs()
            .find(|(k, _)| k == "batchInferenceJobId")
            .unwrap()
            .1
            .to_string();
        ResponseTemplate::new(200).set_body_json(json!({"name":format!("accounts/account-1/batchInferenceJobs/{id}"),"state":"JOB_STATE_PENDING"}))
    }
}
fn fireworks(server: &MockServer, artifacts: &MockServer, account: &str) -> Provider {
    let provider: Provider = Fireworks::new("private-key")
        .base_url(format!("{}/inference/v1", server.uri()))
        .batch(
            FireworksBatchConfig::new(account)
                .unwrap()
                .artifact_origin(&artifacts.uri())
                .unwrap(),
        )
        .into();
    provider.header("x-api-key", "private-header")
}
async fn fireworks_ticket(server: &MockServer, artifacts: &MockServer) -> BatchTicket {
    Mock::given(method("POST"))
        .and(path("/v1/accounts/account-1/datasets"))
        .respond_with(Dataset)
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(
            r"^/v1/accounts/account-1/datasets/everruns-[a-z0-9-]+:upload$",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/accounts/account-1/batchInferenceJobs"))
        .respond_with(Job)
        .mount(server)
        .await;
    Batch::new(fireworks(server, artifacts, "account-1"), "tenant/account")
        .submit_requests(vec![request("one"), request("two")])
        .await
        .unwrap()
}
fn fireworks_job(ticket: &BatchTicket) -> Value {
    json!({"name":ticket.job_id(),"state":"JOB_STATE_COMPLETED","outputDatasetId":"accounts/account-1/datasets/output"})
}
#[tokio::test]
async fn fireworks_native_dataset_job_resumes_and_collects_without_leaking_auth_to_storage() {
    let server = MockServer::start().await;
    let artifacts = MockServer::start().await;
    let ticket = fireworks_ticket(&server, &artifacts).await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    let upload = requests
        .iter()
        .find(|r| r.url.path().ends_with(":upload"))
        .unwrap();
    let text = std::str::from_utf8(&upload.body).unwrap();
    assert!(text.contains("\"max_tokens\":64"));
    assert!(!text.contains("max_completion_tokens"));
    assert!(!text.contains("\"model\""));
    let saved = serde_json::to_string(&ticket).unwrap();
    assert!(!saved.contains("private"));
    assert!(!saved.contains(&server.uri()));
    let ticket: BatchTicket = serde_json::from_str(&saved).unwrap();
    server.reset().await;
    mock(
        &server,
        "GET",
        &format!("/v1/{}", ticket.job_id()),
        fireworks_job(&ticket),
    )
    .await;
    mock(&server,"GET","/v1/accounts/account-1/datasets/output:getDownloadEndpoint",json!({"filenameToSignedUrls":{"part-1":format!("{}/results?signature=secret",artifacts.uri()),"part-2":format!("{}/errors?signature=secret",artifacts.uri())}})).await;
    Mock::given(path("/results"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("{}\n", success("one"))))
        .mount(&artifacts)
        .await;
    Mock::given(path("/errors"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("{}\n", failure("two"))))
        .mount(&artifacts)
        .await;
    let client = Batch::new(
        fireworks(&server, &artifacts, "account-1"),
        "tenant/account",
    );
    for _ in 0..2 {
        let results = client.collect(&ticket).await.unwrap();
        assert_eq!(results[0].id, "one");
        assert!(results[0].result.is_ok());
        assert_eq!(
            results[1].result.as_ref().unwrap_err(),
            &BatchItemError::RateLimited
        );
    }
    for r in artifacts.received_requests().await.unwrap() {
        assert!(r.headers.get("authorization").is_none());
        assert!(r.headers.get("x-api-key").is_none());
    }
    assert_eq!(
        client.cancel(&ticket).await.unwrap_err(),
        BatchError::UnsupportedOperation("Fireworks cancellation")
    );
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET")
    );
    assert_eq!(
        Batch::new(
            fireworks(&server, &artifacts, "account-2"),
            "tenant/account"
        )
        .poll(&ticket)
        .await
        .unwrap_err(),
        BatchError::InvalidTicket
    );
}
#[tokio::test]
async fn fireworks_rejects_untrusted_signed_urls_redirects_and_oversized_artifacts() {
    for mode in ["untrusted", "redirect", "oversized"] {
        let server = MockServer::start().await;
        let artifacts = MockServer::start().await;
        let ticket = fireworks_ticket(&server, &artifacts).await;
        server.reset().await;
        mock(
            &server,
            "GET",
            &format!("/v1/{}", ticket.job_id()),
            fireworks_job(&ticket),
        )
        .await;
        let url = if mode == "untrusted" {
            "http://169.254.169.254/latest/meta-data".into()
        } else {
            format!("{}/result?signature=private", artifacts.uri())
        };
        mock(
            &server,
            "GET",
            "/v1/accounts/account-1/datasets/output:getDownloadEndpoint",
            json!({"filenameToSignedUrls":{"result":url}}),
        )
        .await;
        Mock::given(path("/result"))
            .respond_with(if mode == "redirect" {
                ResponseTemplate::new(302).insert_header("location", "http://169.254.169.254")
            } else {
                ResponseTemplate::new(200).set_body_string("x".repeat(2048))
            })
            .mount(&artifacts)
            .await;
        let err = Batch::new(
            fireworks(&server, &artifacts, "account-1"),
            "tenant/account",
        )
        .max_response_bytes(1024)
        .collect(&ticket)
        .await
        .unwrap_err();
        assert!(!err.to_string().contains("signature"));
        match mode {
            "untrusted" => assert!(matches!(err, BatchError::Protocol(_))),
            "redirect" => assert_eq!(err, BatchError::Http { status: 302 }),
            _ => assert_eq!(err, BatchError::ResponseTooLarge),
        }
    }
}
#[tokio::test]
async fn fireworks_uncertain_staging_is_never_replayed_or_deleted() {
    for stage in 0..3 {
        let server = MockServer::start().await;
        let artifacts = MockServer::start().await;
        if stage > 0 {
            Mock::given(method("POST"))
                .and(path("/v1/accounts/account-1/datasets"))
                .respond_with(Dataset)
                .mount(&server)
                .await;
        }
        if stage > 1 {
            Mock::given(method("POST"))
                .and(path_regex(
                    r"^/v1/accounts/account-1/datasets/everruns-[a-z0-9-]+:upload$",
                ))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .mount(&server)
                .await;
        }
        let pattern = match stage {
            0 => r"^/v1/accounts/account-1/datasets$",
            1 => r"^/v1/accounts/account-1/datasets/.*:upload$",
            _ => r"^/v1/accounts/account-1/batchInferenceJobs$",
        };
        Mock::given(method("POST"))
            .and(path_regex(pattern))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            Batch::new(
                fireworks(&server, &artifacts, "account-1"),
                "tenant/account"
            )
            .submit_requests(vec![request("one")])
            .await
            .unwrap_err(),
            BatchError::SubmissionUncertain
        );
        let calls = server.received_requests().await.unwrap();
        assert_eq!(calls.len(), stage + 1);
        assert!(calls.iter().all(|r| r.method == "POST"));
    }
}
#[tokio::test]
async fn fireworks_terminal_failure_never_invents_per_item_results() {
    for state in [
        "JOB_STATE_FAILED",
        "JOB_STATE_CANCELLED",
        "JOB_STATE_EXPIRED",
    ] {
        let server = MockServer::start().await;
        let artifacts = MockServer::start().await;
        let ticket = fireworks_ticket(&server, &artifacts).await;
        server.reset().await;
        let mut job = fireworks_job(&ticket);
        job["state"] = json!(state);
        mock(&server, "GET", &format!("/v1/{}", ticket.job_id()), job).await;
        assert!(matches!(
            Batch::new(
                fireworks(&server, &artifacts, "account-1"),
                "tenant/account"
            )
            .collect(&ticket)
            .await,
            Err(BatchError::JobFailed { .. })
        ));
        assert!(artifacts.received_requests().await.unwrap().is_empty());
    }
}
#[test]
fn explicit_account_and_artifact_policy_are_required_and_keys_are_redacted() {
    let provider: Provider = Fireworks::new("private").into();
    assert!(!provider.supports_batch());
    assert!(format!("{:?}", Fireworks::new("private")).contains("REDACTED"));
    for account in ["..", "account/path", "account?key=x"] {
        assert!(FireworksBatchConfig::new(account).is_err());
    }
    for origin in [
        "http://storage.example",
        "https://storage.example/path",
        "https://secret@storage.example",
        "https://storage.example?secret",
    ] {
        assert!(
            FireworksBatchConfig::new("account")
                .unwrap()
                .artifact_origin(origin)
                .is_err()
        );
    }
    assert!(Provider::from(Groq::new("key")).supports_batch());
}
#[tokio::test]
async fn groq_first_class_chat_uses_native_non_streaming_and_discovery_identity() {
    use everruns::Model;
    let server = MockServer::start().await;
    mock(&server, "POST", "/openai/v1/chat/completions", completion()).await;
    let response = Model::new(
        "test-model",
        Groq::new("key").base_url(format!("{}/openai/v1", server.uri())),
    )
    .completion()
    .user("hello")
    .max_tokens(64)
    .send()
    .await
    .unwrap();
    assert_eq!(response.text, "positive");
    let calls = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&calls[0].body).unwrap();
    assert_eq!(body["max_tokens"], 64);
    let mut registry = everruns::drivers::DriverRegistry::new();
    everruns::drivers::groq::register_driver(&mut registry);
    assert!(registry.has_driver(&everruns::DriverId::Groq));
    assert_eq!(
        everruns::DriverId::for_base_url("https://api.groq.com/openai/v1"),
        Some(everruns::DriverId::Groq)
    );
}
