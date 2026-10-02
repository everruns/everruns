//! `AgUiHandler`: the ready-made axum AG-UI route, with authorization,
//! thread resolution, SSE framing and keepalive.
#![cfg(feature = "ag-ui-axum")]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use everruns::ag_ui::{
    AgUiCaller, AgUiHandler, AgUiThreads, SSE_KEEPALIVE, StaticToken, Unauthorized,
};
use everruns::{Agent, Engine, FunctionTool, LlmSimConfig, Model, ToolCall};
use futures::StreamExt;
use serde_json::{Value, json};
use tower::ServiceExt;

fn agent(model: Model) -> Agent {
    Agent::builder()
        .instructions("Be brief.")
        .model(model)
        .build()
        .expect("valid agent")
}

fn router(handler: AgUiHandler) -> Router {
    Router::new().route("/ag-ui", handler.route())
}

fn body(thread: &str, text: &str) -> String {
    json!({
        "threadId": thread,
        "runId": "r1",
        "protocolVersion": "1.0",
        "messages": [{ "id": "m1", "role": "user", "content": text }],
    })
    .to_string()
}

fn request(token: Option<&str>, body: impl Into<Body>) -> Request<Body> {
    let mut request = Request::post("/ag-ui").header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    request.body(body.into()).unwrap()
}

async fn text(response: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// The `data:` payloads of an SSE body, after checking every frame is one
/// unnamed `data: <json>` line or the keepalive comment.
fn frames(body: &str) -> Vec<Value> {
    assert!(body.ends_with("\n\n"), "{body:?}");
    body.split_terminator("\n\n")
        .filter(|frame| *frame != ": keepalive")
        .map(|frame| {
            assert!(!frame.contains('\n'), "one line per frame: {frame:?}");
            let data = frame.strip_prefix("data: ").expect("a data frame");
            serde_json::from_str(data).expect("JSON payload")
        })
        .collect()
}

#[tokio::test]
async fn a_bearer_token_is_required_before_the_body_is_read() {
    let threads = AgUiThreads::new(Engine::new(), agent(Model::simulated("Hello.")));
    let app = router(AgUiHandler::new(
        threads.clone(),
        StaticToken::bearer("s3cret"),
    ));

    for token in [
        None,
        Some("guess"),
        Some("s3cre"),
        Some("s3cret2"),
        Some(""),
    ] {
        // A malformed body still answers 401: authorization comes first.
        let response = app
            .clone()
            .oneshot(request(token, "{not json"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{token:?}");
        assert_eq!(response.headers()["www-authenticate"], "Bearer");
        assert_eq!(
            response.headers()["content-type"],
            "application/problem+json"
        );
    }
    let raw = Request::post("/ag-ui")
        .header("authorization", "Basic czNjcmV0")
        .body(Body::from(body("t", "Hi")))
        .unwrap();
    let response = app.clone().oneshot(raw).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // Nothing was created for a refused request.
    assert!(threads.session("t").await.unwrap().created());

    let custom = router(AgUiHandler::new(
        AgUiThreads::new(Engine::new(), agent(Model::simulated("Hello."))),
        StaticToken::header("x-api-key".parse().unwrap(), "k"),
    ));
    let response = custom
        .clone()
        .oneshot(request(Some("k"), body("t", "Hi")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get("www-authenticate").is_none());
    let keyed = Request::post("/ag-ui")
        .header("x-api-key", "k")
        .body(Body::from(body("t", "Hi")))
        .unwrap();
    assert_eq!(
        custom.oneshot(keyed).await.unwrap().status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn bad_input_is_a_400_before_the_stream_opens() {
    let app = router(AgUiHandler::new(
        AgUiThreads::new(Engine::new(), agent(Model::simulated("Hello."))),
        StaticToken::bearer("t"),
    ));
    let cases = [
        "{not json".to_string(),
        json!({ "threadId": 7 }).to_string(),
        body("bad/thread", "Hi"),
        json!({ "threadId": "t", "runId": "r", "messages": [] }).to_string(),
        json!({ "threadId": "t", "runId": "r", "messages": [
            { "id": "a1", "role": "assistant", "content": "hi" }
        ] })
        .to_string(),
    ];
    for case in cases {
        let response = app
            .clone()
            .oneshot(request(Some("t"), case.clone()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{case}");
        let problem: Value = serde_json::from_str(&text(response).await).unwrap();
        assert_eq!(problem["status"], 400);
        assert!(problem["detail"].is_string());
    }
}

#[tokio::test]
async fn a_run_streams_as_sse_frames_end_to_end() {
    let threads = AgUiThreads::new(Engine::new(), agent(Model::simulated("Hello.")));
    let app = router(AgUiHandler::new(threads.clone(), StaticToken::bearer("t")));

    let response = app
        .clone()
        .oneshot(request(Some("t"), body("thread-1", "Hi")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let sse = tokio::time::timeout(Duration::from_secs(10), text(response))
        .await
        .expect("the run ends");
    let events = frames(&sse);
    let kinds: Vec<&str> = events
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect();
    assert_eq!(
        kinds,
        [
            "RUN_STARTED",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
            "RUN_FINISHED"
        ]
    );
    assert_eq!(events[0]["threadId"], "thread-1");
    assert_eq!(events[0]["protocolVersion"], "1.0");
    assert_eq!(events[2]["delta"], "Hello.");

    // The thread continues on the same session.
    let session = threads.session("thread-1").await.unwrap();
    assert!(!session.created());
    let again = app
        .oneshot(request(Some("t"), body("thread-1", "Again")))
        .await
        .unwrap();
    text(again).await;
    let history = session.session().history().page().await.unwrap();
    assert_eq!(history.messages.len(), 4);
}

#[tokio::test]
async fn an_authorizer_scopes_threads_to_its_caller() {
    let threads = AgUiThreads::new(Engine::new(), agent(Model::simulated("Hello.")));
    let by_user = |headers: &HeaderMap| {
        headers
            .get("x-user")
            .and_then(|value| value.to_str().ok())
            .map(AgUiCaller::scoped)
            .ok_or_else(Unauthorized::new)
    };
    let app = router(AgUiHandler::new(threads.clone(), by_user));
    for user in ["alice", "bob"] {
        let request = Request::post("/ag-ui")
            .header("x-user", user)
            .body(Body::from(body("shared", "Hi")))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        text(response).await;
    }
    let alice = threads.session_in("alice", "shared").await.unwrap();
    let bob = threads.session_in("bob", "shared").await.unwrap();
    assert!(!alice.created() && !bob.created());
    assert_ne!(alice.session().session_id(), bob.session().session_id());

    let anonymous = Request::post("/ag-ui")
        .body(Body::from(body("shared", "Hi")))
        .unwrap();
    let response = app.oneshot(anonymous).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get("www-authenticate").is_none());
}

#[tokio::test(start_paused = true)]
async fn a_quiet_stream_sends_keepalive_comments() {
    // A tool that holds the turn until the test releases it.
    let release = Arc::new(tokio::sync::Notify::new());
    let held = release.clone();
    let wait = FunctionTool::new(
        "wait",
        "Wait for the operator.",
        json!({ "type": "object", "properties": {} }),
        move |_args: Value| {
            let held = held.clone();
            async move {
                held.notified().await;
                Ok::<_, String>(json!({ "done": true }))
            }
        },
    );
    let model =
        Model::simulated_with_config(LlmSimConfig::fixed("Done.").with_tool_call_sequence(vec![
            vec![ToolCall {
                id: "call_1".into(),
                name: "wait".into(),
                arguments: json!({}),
            }],
            vec![],
        ]));
    let agent = Agent::builder()
        .instructions("Wait first.")
        .model(model)
        .tool(wait)
        .build()
        .unwrap();
    let app = router(AgUiHandler::new(
        AgUiThreads::new(Engine::new(), agent),
        StaticToken::bearer("t"),
    ));

    let response = app
        .oneshot(request(Some("t"), body("t", "Go")))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let started = tokio::time::Instant::now();
    let mut chunks = response.into_body().into_data_stream();
    let mut body = String::new();
    let mut keepalives = 0;
    while keepalives < 2 {
        let chunk = chunks.next().await.expect("the stream stays open").unwrap();
        let chunk = String::from_utf8(chunk.to_vec()).unwrap();
        if chunk == ": keepalive\n\n" {
            keepalives += 1;
        }
        body.push_str(&chunk);
    }
    // Two keepalives take two quiet intervals of (paused) time.
    assert!(
        started.elapsed() >= SSE_KEEPALIVE * 2,
        "{:?}",
        started.elapsed()
    );
    release.notify_one();
    while let Some(chunk) = chunks.next().await {
        body.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
    }
    let events = frames(&body);
    assert_eq!(events.first().unwrap()["type"], "RUN_STARTED");
    assert_eq!(events.last().unwrap()["type"], "RUN_FINISHED");
    assert!(
        events
            .iter()
            .any(|event| event["type"] == "TEXT_MESSAGE_CONTENT" && event["delta"] == "Done.")
    );
}
