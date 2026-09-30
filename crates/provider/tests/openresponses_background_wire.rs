// OpenAI Responses background mode on the wire (EVE-1116).
//
// A long call runs with `background: true` so a dropped connection does not
// lose it: the driver re-attaches with `GET /responses/{id}?stream=true&
// starting_after=N` instead of posting (and paying for) the call again, and an
// abandoned response is cancelled rather than left generating.

use everruns_provider::driver_registry::{LlmCallConfig, LlmStreamEvent, Message, MessageRole};
use everruns_provider::model::ReasoningEffort;
use everruns_provider::{
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
