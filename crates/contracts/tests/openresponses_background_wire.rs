#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "http")]

// OpenAI Responses background mode on the wire (EVE-1116).
//
// A long call runs with `background: true` so a dropped connection does not
// lose it: the driver re-attaches with `GET /responses/{id}?stream=true&
// starting_after=N` instead of posting (and paying for) the call again, and an
// abandoned response is cancelled rather than left generating.

use everruns_contracts::driver_registry::{LlmCallConfig, LlmStreamEvent, Message, MessageRole};
use everruns_contracts::model::ReasoningEffort;
use everruns_contracts::{
    BearerAuth, OPENAI_BACKGROUND_OPTION, OpenResponsesProtocolChatDriver, Provider,
};
use futures::StreamExt;
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ID: &str = "resp_bg";

fn sse(events: &[Value]) -> String {
    events.iter().map(|e| format!("data: {e}\n\n")).collect()
}

fn created(seq: u64) -> Value {
    json!({"type":"response.created","sequence_number":seq,"response":{
        "id":ID,"object":"response","created_at":1780000000,"status":"queued",
        "model":"gpt-6-astra","output":[]}})
}

fn delta(seq: u64, text: &str) -> Value {
    json!({"type":"response.output_text.delta","sequence_number":seq,"item_id":"msg_1",
        "output_index":0,"content_index":0,"delta":text})
}

fn completed(seq: u64) -> Value {
    json!({"type":"response.completed","sequence_number":seq,"response":{
        "id":ID,"object":"response","created_at":1780000000,"status":"completed",
        "model":"gpt-6-astra","output":[],
        "usage":{"input_tokens":5,"output_tokens":2,"total_tokens":7}}})
}

fn provider(server: &MockServer, background_mode: bool) -> Provider {
    Provider::new(
        "openai",
        OpenResponsesProtocolChatDriver::new().with_background_mode(background_mode),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(BearerAuth::new("test-key"))
}

async fn mount_post(server: &MockServer, events: &[Value]) {
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse(events), "text/event-stream"))
        .mount(server)
        .await;
}

fn config(effort: Option<ReasoningEffort>) -> LlmCallConfig {
    let mut config = LlmCallConfig::new("gpt-6-astra");
    config.reasoning_effort = effort;
    config
}

async fn posted_body(server: &MockServer) -> Value {
    let requests = server.received_requests().await.unwrap();
    let post = requests
        .iter()
        .find(|r| r.method.as_str() == "POST")
        .expect("a POST");
    serde_json::from_slice(&post.body).unwrap()
}

#[tokio::test]
async fn long_reasoning_runs_in_background_unless_opted_out() {
    let forced_off = {
        let mut config = config(Some(ReasoningEffort::Max));
        config
            .driver_options
            .insert(OPENAI_BACKGROUND_OPTION.into(), json!(false));
        config
    };
    let forced_on = {
        let mut config = config(Some(ReasoningEffort::Low));
        config
            .driver_options
            .insert(OPENAI_BACKGROUND_OPTION.into(), json!(true));
        config
    };
    for (config, background_mode, expected) in [
        (config(Some(ReasoningEffort::Xhigh)), true, true),
        (config(Some(ReasoningEffort::Max)), true, true),
        (config(Some(ReasoningEffort::High)), true, false),
        (config(None), true, false),
        (forced_off, true, false),
        (forced_on, true, true),
        // Endpoints without the resume API never get it.
        (config(Some(ReasoningEffort::Max)), false, false),
    ] {
        let server = MockServer::start().await;
        mount_post(&server, &[created(0), delta(1, "hi"), completed(2)]).await;
        provider(&server, background_mode)
            .chat_completion(vec![Message::text(MessageRole::User, "hi")], &config)
            .await
            .unwrap();
        let body = posted_body(&server).await;
        if expected {
            assert_eq!(body["background"], true, "{body}");
            assert_eq!(body["store"], true, "{body}");
        } else {
            assert!(body.get("background").is_none(), "{body}");
            assert!(body.get("store").is_none(), "{body}");
        }
    }
}

#[tokio::test]
async fn dropped_connection_resumes_after_the_last_event_without_a_second_call() {
    let server = MockServer::start().await;
    // The first connection closes mid-answer, before any terminal event.
    mount_post(&server, &[created(0), delta(1, "Hello")]).await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/responses/{ID}")))
        .and(query_param("stream", "true"))
        .and(query_param("starting_after", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            sse(&[delta(2, ", world"), completed(3)]),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let response = provider(&server, true)
        .chat_completion(
            vec![Message::text(MessageRole::User, "hi")],
            &config(Some(ReasoningEffort::Max)),
        )
        .await
        .expect("the resumed stream completes the call");
    assert_eq!(response.text, "Hello, world");
    assert_eq!(response.metadata.response_id.as_deref(), Some(ID));

    let requests = server.received_requests().await.unwrap();
    let posts = requests
        .iter()
        .filter(|r| r.method.as_str() == "POST")
        .count();
    assert_eq!(posts, 1, "the call must not be posted (and billed) twice");
    assert!(
        !requests.iter().any(|r| r.url.path().ends_with("/cancel")),
        "a completed response is not cancelled"
    );
}

#[tokio::test]
async fn abandoned_background_response_is_cancelled() {
    let server = MockServer::start().await;
    mount_post(&server, &[created(0), delta(1, "Hel"), delta(2, "lo")]).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/responses/{ID}/cancel")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"id":ID,"status":"cancelled"})),
        )
        .expect(1)
        .mount(&server)
        .await;

    let mut stream = provider(&server, true)
        .chat_completion_stream(
            vec![Message::text(MessageRole::User, "hi")],
            &config(Some(ReasoningEffort::Max)),
        )
        .await
        .unwrap();
    // The turn is cancelled after the first delta: the consumer drops the stream.
    let first = stream.next().await.unwrap().unwrap();
    assert!(matches!(first, LlmStreamEvent::TextDelta(_)));
    drop(stream);

    // The cancel runs on a spawned task.
    for _ in 0..50 {
        let requests = server.received_requests().await.unwrap();
        if requests.iter().any(|r| r.url.path().ends_with("/cancel")) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the abandoned background response was not cancelled");
}

#[tokio::test]
async fn completed_background_response_is_not_cancelled() {
    let server = MockServer::start().await;
    mount_post(&server, &[created(0), delta(1, "done"), completed(2)]).await;
    provider(&server, true)
        .chat_completion(
            vec![Message::text(MessageRole::User, "hi")],
            &config(Some(ReasoningEffort::Max)),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let requests = server.received_requests().await.unwrap();
    assert!(!requests.iter().any(|r| r.url.path().ends_with("/cancel")));
}

#[tokio::test]
async fn rejected_background_mode_falls_back_to_the_foreground() {
    use wiremock::matchers::body_partial_json;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_partial_json(json!({"background": true})))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":{
            "message":"Background mode is not available for organizations with zero data retention.",
            "type":"invalid_request_error","param":"background","code":null}})))
        .expect(1)
        .mount(&server)
        .await;
    mount_post(&server, &[created(0), delta(1, "ok"), completed(2)]).await;

    let response = provider(&server, true)
        .chat_completion(
            vec![Message::text(MessageRole::User, "hi")],
            &config(Some(ReasoningEffort::Max)),
        )
        .await
        .expect("the foreground retry succeeds");
    assert_eq!(response.text, "ok");
    let requests = server.received_requests().await.unwrap();
    let retry: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
    assert!(retry.get("background").is_none(), "{retry}");
    assert!(retry.get("store").is_none(), "{retry}");
}

// ---------------------------------------------------------------------------
// Re-attaching across a worker restart (EVE-1134)
// ---------------------------------------------------------------------------

use everruns_contracts::background_call::{
    BackgroundCallContext, BackgroundResponseJournal, BackgroundResponseRecord,
};
use std::sync::{Arc, Mutex};
use wiremock::matchers::query_param_is_missing;

/// Stands in for the turn's durable checkpoint, which outlives the process.
#[derive(Default)]
struct MemoryJournal {
    record: Mutex<Option<BackgroundResponseRecord>>,
    fail_writes: bool,
}

#[async_trait::async_trait]
impl BackgroundResponseJournal for MemoryJournal {
    async fn load(&self) -> everruns_contracts::error::Result<Option<BackgroundResponseRecord>> {
        Ok(self.record.lock().unwrap().clone())
    }
    async fn store(
        &self,
        record: Option<&BackgroundResponseRecord>,
    ) -> everruns_contracts::error::Result<()> {
        if self.fail_writes {
            return Err(everruns_contracts::error::AgentLoopError::store(
                "journal unavailable",
            ));
        }
        *self.record.lock().unwrap() = record.cloned();
        Ok(())
    }
}

impl MemoryJournal {
    fn record(&self) -> Option<BackgroundResponseRecord> {
        self.record.lock().unwrap().clone()
    }
}

fn durable_config(journal: &Arc<MemoryJournal>) -> LlmCallConfig {
    let mut config = config(Some(ReasoningEffort::Max));
    config.background_call = BackgroundCallContext::default().with_journal(journal.clone());
    config
}

fn messages() -> Vec<Message> {
    vec![Message::text(MessageRole::User, "hi")]
}

async fn requests_matching(server: &MockServer, method: &str, suffix: &str) -> usize {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.method.as_str() == method && r.url.path().ends_with(suffix))
        .count()
}

/// The first worker reads one delta and dies: its stream is dropped while the
/// response is still running at OpenAI.
async fn die_mid_call(server: &MockServer, config: &LlmCallConfig) {
    mount_post(server, &[created(0), delta(1, "Hel")]).await;
    let mut stream = provider(server, true)
        .chat_completion_stream(messages(), config)
        .await
        .unwrap();
    let first = stream.next().await.unwrap().unwrap();
    assert!(matches!(first, LlmStreamEvent::TextDelta(_)));
    drop(stream);
}

#[tokio::test]
async fn response_id_is_persisted_before_the_first_event_reaches_the_caller() {
    let server = MockServer::start().await;
    let journal = Arc::new(MemoryJournal::default());
    mount_post(&server, &[created(0), delta(1, "Hel")]).await;
    let mut stream = provider(&server, true)
        .chat_completion_stream(messages(), &durable_config(&journal))
        .await
        .unwrap();
    stream.next().await.unwrap().unwrap();
    let record = journal
        .record()
        .expect("the id is durable once events flow");
    assert_eq!(record.response_id, ID);
    assert!(!record.request_fingerprint.is_empty());
    drop(stream);
}

#[tokio::test]
async fn durable_retry_reattaches_instead_of_posting_the_call_again() {
    let server = MockServer::start().await;
    let journal = Arc::new(MemoryJournal::default());
    die_mid_call(&server, &durable_config(&journal)).await;

    // With a durable record the dropped stream is kept alive for the retry.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(requests_matching(&server, "POST", "/cancel").await, 0);
    assert_eq!(journal.record().unwrap().response_id, ID);

    // The retry runs with a fresh parser, so it replays from the first event.
    Mock::given(method("GET"))
        .and(path(format!("/v1/responses/{ID}")))
        .and(query_param("stream", "true"))
        .and(query_param_is_missing("starting_after"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            sse(&[created(0), delta(1, "Hel"), delta(2, "lo"), completed(3)]),
            "text/event-stream",
        ))
        .expect(1)
        .mount(&server)
        .await;
    // The retry rebuilds the request with new per-attempt metadata.
    let mut retry = durable_config(&journal);
    retry.metadata.insert("exec_id".into(), "exec_retry".into());
    let response = provider(&server, true)
        .chat_completion(messages(), &retry)
        .await
        .expect("the re-attached response completes the call");
    assert_eq!(response.text, "Hello");
    assert_eq!(
        requests_matching(&server, "POST", "/v1/responses").await,
        1,
        "the call must not be posted (and billed) twice"
    );
    assert!(
        journal.record().is_none(),
        "a finished response is forgotten"
    );
}

#[tokio::test]
async fn record_from_a_different_request_is_cancelled_and_the_call_posted() {
    let server = MockServer::start().await;
    let journal = Arc::new(MemoryJournal::default());
    *journal.record.lock().unwrap() = Some(BackgroundResponseRecord {
        response_id: "resp_stale".into(),
        request_fingerprint: "another-request".into(),
    });
    Mock::given(method("POST"))
        .and(path("/v1/responses/resp_stale/cancel"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"cancelled"})))
        .expect(1)
        .mount(&server)
        .await;
    mount_post(&server, &[created(0), delta(1, "fresh"), completed(2)]).await;

    let response = provider(&server, true)
        .chat_completion(messages(), &durable_config(&journal))
        .await
        .unwrap();
    assert_eq!(response.text, "fresh");
    assert_eq!(requests_matching(&server, "GET", "").await, 0);
    assert!(journal.record().is_none());
}

#[tokio::test]
async fn expired_record_falls_back_to_posting_the_call() {
    let server = MockServer::start().await;
    let journal = Arc::new(MemoryJournal::default());
    die_mid_call(&server, &durable_config(&journal)).await;
    // The stored response is gone (expired or never stored).
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/v1/responses/{ID}")))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    mount_post(&server, &[created(0), delta(1, "again"), completed(2)]).await;

    let response = provider(&server, true)
        .chat_completion(messages(), &durable_config(&journal))
        .await
        .expect("an unrecoverable record does not fail the call");
    assert_eq!(response.text, "again");
    assert_eq!(requests_matching(&server, "POST", "/v1/responses").await, 1);
}

#[tokio::test]
async fn turn_cancel_cancels_the_response_while_its_stream_is_still_read() {
    let server = MockServer::start().await;
    let journal = Arc::new(MemoryJournal::default());
    mount_post(&server, &[created(0), delta(1, "Hel"), delta(2, "lo")]).await;
    Mock::given(method("POST"))
        .and(path(format!("/v1/responses/{ID}/cancel")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"cancelled"})))
        .expect(1)
        .mount(&server)
        .await;
    let (cancel, signal) = tokio::sync::watch::channel(false);
    let mut config = durable_config(&journal);
    config.background_call = config.background_call.clone().with_cancel(signal);

    let mut stream = provider(&server, true)
        .chat_completion_stream(messages(), &config)
        .await
        .unwrap();
    stream.next().await.unwrap().unwrap();
    cancel.send(true).unwrap();
    // The stream is not dropped: the consumer keeps reading, and the cancel
    // still reaches OpenAI before the stream ends.
    while let Some(event) = stream.next().await {
        assert!(
            !matches!(event, Ok(LlmStreamEvent::TextDelta(ref text)) if text == "lo"),
            "no output is forwarded after the cancel"
        );
    }
    assert_eq!(
        requests_matching(&server, "POST", &format!("{ID}/cancel")).await,
        1
    );
    assert!(
        journal.record().is_none(),
        "a cancelled response is not resumed"
    );
}

#[tokio::test]
async fn ownership_loss_without_a_turn_cancel_leaves_the_response_running() {
    let server = MockServer::start().await;
    let journal = Arc::new(MemoryJournal::default());
    let (_cancel, signal) = tokio::sync::watch::channel(false);
    let mut config = durable_config(&journal);
    config.background_call = config.background_call.clone().with_cancel(signal);
    die_mid_call(&server, &config).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(requests_matching(&server, "POST", "/cancel").await, 0);
    assert!(journal.record().is_some());
}

#[tokio::test]
async fn unwritable_journal_keeps_cancelling_abandoned_responses() {
    let server = MockServer::start().await;
    let journal = Arc::new(MemoryJournal {
        fail_writes: true,
        ..Default::default()
    });
    Mock::given(method("POST"))
        .and(path(format!("/v1/responses/{ID}/cancel")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status":"cancelled"})))
        .mount(&server)
        .await;
    die_mid_call(&server, &durable_config(&journal)).await;
    for _ in 0..50 {
        if requests_matching(&server, "POST", "/cancel").await == 1 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("without a durable record the abandoned response must be cancelled");
}
