//! The public capability reaches the runtime and preserves session history.
#![cfg(all(feature = "anthropic", feature = "builtins"))]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use everruns::hosting::{AgentBuild, MemoryHosting};
use everruns::{Agent, Engine, SessionId, providers::anthropic::Anthropic};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn registered_capability_continues_and_replays_without_losing_session_history() {
    let server = MockServer::start().await;
    let count = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST")).and(path("/v1/messages"))
        .respond_with(move |request: &Request| {
            let index = count.fetch_add(1, Ordering::SeqCst);
            if index == 2 {
                return ResponseTemplate::new(404).insert_header("connection","close")
                    .set_body_json(json!({"error":{"type":"not_found_error","details":{"error_code":"thread_not_found"}}}));
            }
            let message = json!({"id":format!("msg_{index}"),"type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"47"}],"stop_reason":"end_turn","usage":{"input_tokens":10,"output_tokens":1}});
            let request: Value = request.body_json().unwrap();
            let response = ResponseTemplate::new(200).insert_header("connection","close");
            if request["stream"] == false { return response.set_body_json(message); }
            response.set_body_raw(format!(
                "event: message_start\ndata: {}\n\nevent: content_block_start\ndata: {}\n\nevent: content_block_delta\ndata: {}\n\nevent: content_block_stop\ndata: {}\n\nevent: message_delta\ndata: {}\n\nevent: message_stop\ndata: {{}}\n\n",
                json!({"message":{"id":format!("msg_{index}"),"usage":{"input_tokens":10}}}),
                json!({"index":0,"content_block":{"type":"text","text":""}}),
                json!({"index":0,"delta":{"type":"text_delta","text":"47"}}),
                json!({"index":0}),
                json!({"delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
            ),"text/event-stream")
        }).mount(&server).await;
    let agent = Agent::builder()
        .instructions("Remember numbers.")
        .provider(Anthropic::new("fixture-key").base_url(format!("{}/v1", server.uri())))
        .model("claude-sonnet-4-5")
        .capability("claude_message_threads")
        .build()
        .unwrap();
    let build = AgentBuild::new("threads", "v1");
    let engine = Engine::builder()
        .hosting(MemoryHosting::default())
        .register(build.clone(), agent)
        .build();
    let session = engine
        .create_registered(SessionId::new(), &build)
        .await
        .unwrap();
    for prompt in ["Remember 47.", "What number?", "Again?"] {
        session.send(prompt).await.unwrap().wait().await.unwrap();
    }
    let requests = server.received_requests().await.unwrap();
    let bodies = requests
        .iter()
        .map(|request| request.body_json::<Value>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(bodies.len(), 4);
    assert_eq!(bodies[0]["thread"]["type"], "create");
    assert_eq!(bodies[1]["thread"]["type"], "continue");
    assert_eq!(bodies[2]["thread"]["type"], "continue");
    assert_eq!(bodies[3]["thread"]["type"], "create");
    assert!(bodies[1].get("system").is_none());
    assert!(bodies[3].get("system").is_some());
    let replay = bodies[3]["messages"].to_string();
    for prompt in ["Remember 47.", "What number?", "Again?"] {
        assert!(replay.contains(prompt));
    }
    assert!(bodies[1]["messages"].to_string().len() < bodies[3]["messages"].to_string().len());
    println!(
        "PASS: Framework capability -> create -> continue -> expired continuation -> full replay, all three user turns retained"
    );
}
