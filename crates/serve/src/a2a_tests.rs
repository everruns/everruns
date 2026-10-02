//! `POST /v1/e/{agent}/a2a`: A2A 1.0 JSON-RPC over the same host, with the
//! Agent Card under it. A child of `wire_tests`, so it shares its server and
//! test agents.

use super::*;

/// Replies `pong` to every message, with no tools.
#[agent]
fn echoer() -> Agent {
    Agent::builder()
        .model("sim")
        .description("Answers every message with pong.")
        .instructions("Reply pong.")
        .tools(Vec::<String>::new())
        .offline(sim::script([sim::reply("pong")]))
        .build()
}

fn send_body(method: &str, text: &str, context: Option<&str>) -> Value {
    let mut message = json!({
        "messageId": uuid::Uuid::new_v4().to_string(),
        "role": "ROLE_USER",
        "parts": [{ "text": text }],
    });
    if let Some(context) = context {
        message["contextId"] = json!(context);
    }
    json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": { "message": message } })
}

impl Server {
    /// One JSON-RPC call to `agent`'s endpoint, as an A2A 1.0 client sends it.
    async fn a2a(&self, agent: &str, body: Value) -> reqwest::Response {
        self.client
            .post(format!("{}/v1/e/{agent}/a2a", self.base))
            .header("A2A-Version", "1.0")
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    /// A blocking `SendMessage`; the task it returns.
    async fn send_message(&self, agent: &str, text: &str, context: Option<&str>) -> Value {
        let response = self
            .a2a(agent, send_body("SendMessage", text, context))
            .await;
        assert_eq!(response.status(), 200);
        let body: Value = tokio::time::timeout(Duration::from_secs(10), response.json())
            .await
            .expect("the task settles")
            .unwrap();
        assert!(body["error"].is_null(), "{body}");
        body["result"]["task"].clone()
    }
}

fn reply(task: &Value) -> &str {
    let artifact = &task["artifacts"][0];
    assert_eq!(artifact["name"], "response", "{task}");
    artifact["parts"][0]["text"].as_str().unwrap()
}

#[tokio::test]
async fn the_agent_card_points_at_the_endpoint() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let response = server
        .get("/v1/e/echoer/a2a/.well-known/agent-card.json")
        .await;
    assert_eq!(response.status(), 200);
    let card: Value = response.json().await.unwrap();
    assert_eq!(card["name"], "echoer");
    assert_eq!(card["description"], "Answers every message with pong.");
    let interface = &card["supportedInterfaces"][0];
    assert_eq!(
        interface["url"],
        format!("{}/v1/e/echoer/a2a", server.base),
        "{card}"
    );
    assert_eq!(interface["protocolBinding"], "JSONRPC");
    assert_eq!(interface["protocolVersion"], "1.0");
    assert_eq!(card["capabilities"]["streaming"], true);
    // The card parses as the SDK's own type, which a client resolves.
    serde_json::from_value::<::a2a::AgentCard>(card).unwrap();
}

#[tokio::test]
async fn send_message_returns_the_completed_task_and_a_context_keeps_its_session() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host.clone()).await;

    let task = server.send_message("echoer", "ping", Some("c1")).await;
    assert_eq!(task["status"]["state"], "TASK_STATE_COMPLETED", "{task}");
    assert_eq!(task["contextId"], "c1");
    assert_eq!(reply(&task), "pong");
    let session = host.thread_session("a2a:echoer", "c1").unwrap().unwrap();
    assert_eq!(server.session(&session).await["agent_name"], "echoer");

    // A follow-up on the same context is a new task on the same session.
    let again = server
        .send_message("echoer", "ping again", Some("c1"))
        .await;
    assert_ne!(again["id"], task["id"]);
    assert_eq!(reply(&again), "pong");
    assert_eq!(
        host.thread_session("a2a:echoer", "c1").unwrap().unwrap(),
        session
    );
    let events = server.wait_turns(&session, 2).await;
    let inputs = events
        .iter()
        .filter(|e| e["type"] == "input.message")
        .count();
    assert_eq!(inputs, 2, "{:?}", kinds(&events));

    // GetTask reads the finished task back from the task store.
    let got: Value = server
        .a2a(
            "echoer",
            json!({ "jsonrpc": "2.0", "id": 2, "method": "GetTask", "params": { "id": task["id"] } }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(got["result"]["status"]["state"], "TASK_STATE_COMPLETED");

    // No context: the SDK mints one, and it gets a session of its own.
    let fresh = server.send_message("echoer", "ping", None).await;
    let context = fresh["contextId"].as_str().unwrap();
    assert_ne!(context, "c1");
    let other = host.thread_session("a2a:echoer", context).unwrap().unwrap();
    assert_ne!(other, session);
}

#[tokio::test]
async fn streaming_emits_working_then_the_reply_then_completed() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let response = server
        .a2a(
            "echoer",
            send_body("SendStreamingMessage", "ping", Some("s1")),
        )
        .await;
    assert_eq!(response.status(), 200);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let body = tokio::time::timeout(Duration::from_secs(10), response.text())
        .await
        .expect("the stream ends")
        .unwrap();
    let results: Vec<Value> = body
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| serde_json::from_str::<Value>(data.trim()).unwrap()["result"].clone())
        .collect();
    let shape: Vec<String> = results
        .iter()
        .map(
            |result| match result.as_object().and_then(|o| o.keys().next()) {
                Some(key) if key == "statusUpdate" => format!(
                    "status:{}",
                    result[key]["status"]["state"].as_str().unwrap()
                ),
                Some(key) => key.clone(),
                None => "?".into(),
            },
        )
        .collect();
    assert_eq!(
        shape,
        [
            "status:TASK_STATE_WORKING",
            "artifactUpdate",
            "status:TASK_STATE_COMPLETED"
        ],
        "{body}"
    );
    let artifact = &results[1]["artifactUpdate"]["artifact"];
    assert_eq!(artifact["parts"][0]["text"], "pong");
}

#[tokio::test]
async fn an_approval_parks_the_task_until_the_v1_api_answers_it() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = Arc::new(serve(host.clone()).await);

    // `tester`'s first turn calls `shout`; its second needs an approval.
    let first = server.send_message("tester", "go", Some("p1")).await;
    assert_eq!(reply(&first), "shouted");
    let session = host.thread_session("a2a:tester", "p1").unwrap().unwrap();

    let pending = tokio::spawn({
        let server = server.clone();
        async move { server.send_message("tester", "again", Some("p1")).await }
    });
    let waiting = server
        .wait_until(&session, |s| s["status"] == "waitingfortoolresults")
        .await;
    let call = waiting["pending_approvals"][0]["tool_call_id"]
        .as_str()
        .unwrap()
        .to_string();
    host.resolve_approval(&session, &call, true).unwrap();
    let second = pending.await.unwrap();
    assert_eq!(second["status"]["state"], "TASK_STATE_COMPLETED");
    assert_eq!(reply(&second), "guarded done");
}

#[tokio::test]
async fn a_context_keeps_its_session_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let session = {
        let host = Host::new(app(), Mode::Dev, Some(dir.path().to_path_buf())).unwrap();
        let server = serve(host.clone()).await;
        server.send_message("echoer", "ping", Some("r1")).await;
        server._task.abort();
        host.thread_session("a2a:echoer", "r1").unwrap().unwrap()
    };
    let host = Host::new(app(), Mode::Dev, Some(dir.path().to_path_buf())).unwrap();
    let server = serve(host.clone()).await;
    let task = server.send_message("echoer", "ping", Some("r1")).await;
    assert_eq!(reply(&task), "pong");
    assert_eq!(
        host.thread_session("a2a:echoer", "r1").unwrap().unwrap(),
        session
    );
    server.wait_turns(&session, 2).await;
}

#[tokio::test]
async fn unknown_agents_bad_input_and_old_versions_are_errors() {
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;

    let unknown = server
        .a2a("nobody", send_body("SendMessage", "hi", None))
        .await;
    assert_eq!(unknown.status(), 404);
    assert_eq!(
        unknown.headers()["content-type"],
        "application/problem+json"
    );
    let card = server
        .get("/v1/e/nobody/a2a/.well-known/agent-card.json")
        .await;
    assert_eq!(card.status(), 404);

    // No text or data part: a JSON-RPC error, not a turn.
    let empty: Value = server
        .a2a("echoer", send_body("SendMessage", " ", None))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(empty["error"]["code"], -32602, "{empty}");

    // No `A2A-Version` header means 0.3, which this endpoint does not speak.
    let old: Value = server
        .post("/v1/e/echoer/a2a", send_body("SendMessage", "hi", None))
        .await
        .json()
        .await
        .unwrap();
    assert!(old["error"].is_object(), "{old}");
}

#[tokio::test]
async fn the_manifest_and_agent_card_list_the_endpoints() {
    let manifest = app().manifest();
    for route in [
        "POST /v1/e/echoer/a2a",
        "GET /v1/e/echoer/a2a/.well-known/agent-card.json",
    ] {
        assert!(
            manifest.routes.contains(&route.to_string()),
            "{:?}",
            manifest.routes
        );
    }
    let host = Host::new(app(), Mode::Eval, None).unwrap();
    let server = serve(host).await;
    let card: Value = server.get("/v1/agent").await.json().await.unwrap();
    assert_eq!(card["a2a"]["echoer"], "/v1/e/echoer/a2a");
    assert_eq!(card["a2a"]["tester"], "/v1/e/tester/a2a");
}
