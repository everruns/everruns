//! AG-UI delegation against a local mock AG-UI agent (axum, SSE). Mirrors the
//! A2A delegation tests: foreground and background runs, the generic task
//! tools, egress and secret handling.

use super::run::{AgUiRunStatus, load_run, resume_from_answer};
use super::*;
use crate::capabilities::session_tasks::tests::InMemorySessionTaskRegistry;
use crate::capabilities::session_tasks::{CancelTaskTool, MessageTaskTool, WaitTaskTool};
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use everruns_ag_ui::{Interrupt, ResumeStatus};
use everruns_core::network_access::NetworkAccessList;
use everruns_core::session_services::SessionStorageStore;
use everruns_core::session_task::{SessionTaskRegistry, TaskExecutor};
use everruns_host::{InMemorySessionFileStore, InMemorySessionStorageStore};
use everruns_provider::typed_id::SessionId;
use futures::StreamExt;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What the mock agent answers one request with.
enum Reply {
    /// An SSE stream of these events, then the body ends.
    Events(Vec<Value>),
    /// These events, then a body that never ends. The flag flips when the
    /// client closes the connection.
    Hang(Vec<Value>, Arc<AtomicBool>),
}

#[derive(Clone, Default)]
struct Mock {
    replies: Arc<Mutex<VecDeque<Reply>>>,
    requests: Arc<Mutex<Vec<(HeaderMap, Value)>>>,
}

impl Mock {
    fn requests(&self) -> Vec<(HeaderMap, Value)> {
        self.requests.lock().unwrap().clone()
    }
}

struct SetOnDrop(Arc<AtomicBool>);

impl Drop for SetOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn frames(events: Vec<Value>) -> Vec<Result<String, std::io::Error>> {
    events
        .into_iter()
        .map(|event| Ok(format!("data: {event}\n\n")))
        .collect()
}

async fn handle(State(mock): State<Mock>, headers: HeaderMap, body: String) -> Response {
    mock.requests
        .lock()
        .unwrap()
        .push((headers, serde_json::from_str(&body).unwrap()));
    let reply = mock.replies.lock().unwrap().pop_front();
    let body = match reply {
        None => return Response::builder().status(500).body(Body::empty()).unwrap(),
        Some(Reply::Events(events)) => Body::from_stream(futures::stream::iter(frames(events))),
        Some(Reply::Hang(events, closed)) => {
            let guard = SetOnDrop(closed);
            let tail = futures::stream::pending::<Result<String, std::io::Error>>().map(move |x| {
                let _ = &guard;
                x
            });
            Body::from_stream(futures::stream::iter(frames(events)).chain(tail))
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .body(body)
        .unwrap()
}

async fn mock_agent(replies: Vec<Reply>) -> (String, Mock) {
    let mock = Mock {
        replies: Arc::new(Mutex::new(replies.into())),
        ..Mock::default()
    };
    let app = Router::new()
        .route("/agent", post(handle))
        .with_state(mock.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/agent", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, mock)
}

fn started() -> Value {
    json!({ "type": "RUN_STARTED", "threadId": "t", "runId": "remote-1" })
}

fn text(delta: &str) -> Value {
    json!({ "type": "TEXT_MESSAGE_CHUNK", "messageId": "m1", "delta": delta })
}

fn finished(run_id: &str) -> Value {
    json!({ "type": "RUN_FINISHED", "threadId": "t", "runId": run_id })
}

fn agent_config(url: &str, extra: Value) -> Value {
    let mut agent = json!({
        "id": "partner",
        "name": "Partner Agent",
        "url": url,
        "allow_local_urls": true,
    });
    agent
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    json!({ "agents": [agent] })
}

fn spawn_tool(config: &Value) -> SpawnAgUiAgentTool {
    AgUiDelegationCapability.validate_config(config).unwrap();
    SpawnAgUiAgentTool::new(AgUiDelegationConfig::from_value(config).unwrap())
}

struct Harness {
    ctx: ToolContext,
    storage: Arc<InMemorySessionStorageStore>,
    registry: Arc<InMemorySessionTaskRegistry>,
}

fn harness() -> Harness {
    let storage = Arc::new(InMemorySessionStorageStore::new());
    let registry = Arc::new(InMemorySessionTaskRegistry::default());
    let ctx = ToolContext::with_stores(
        SessionId::new(),
        Arc::new(InMemorySessionFileStore::new()),
        storage.clone(),
    )
    .with_session_task_registry(registry.clone());
    Harness {
        ctx,
        storage,
        registry,
    }
}

fn spawn_args(mode: &str) -> Value {
    json!({
        "instructions": "Summarise the report",
        "target": { "type": AG_UI_TARGET_TYPE, "id": "partner" },
        "mode": mode,
        "wait_timeout_secs": 10,
    })
}

fn success(result: ToolExecutionResult) -> Value {
    match result {
        ToolExecutionResult::Success(value) => value,
        other => panic!("expected success, got {other:?}"),
    }
}

async fn wait(ctx: &ToolContext, task_id: &str) -> Value {
    success(
        WaitTaskTool
            .execute_with_context(json!({ "task_id": task_id, "timeout_seconds": 10 }), ctx)
            .await,
    )
}

#[tokio::test]
async fn foreground_spawn_streams_text_back_with_the_bearer_secret() {
    let (url, mock) = mock_agent(vec![Reply::Events(vec![
        started(),
        text("Hello "),
        text("world"),
        json!({ "type": "RUN_FINISHED", "threadId": "t", "runId": "remote-1",
                "result": { "ok": true } }),
    ])])
    .await;
    let h = harness();
    h.storage
        .set_secret(h.ctx.session_id, "PARTNER_TOKEN", "tok-secret-123")
        .await
        .unwrap();
    let config = agent_config(
        &url,
        json!({ "bearer_token_secret": "PARTNER_TOKEN", "headers": { "x-tenant": "acme" } }),
    );

    let value = success(
        spawn_tool(&config)
            .execute_with_context(spawn_args("foreground"), &h.ctx)
            .await,
    );
    assert_eq!(value["status"], "completed", "{value}");
    assert_eq!(value["result"], "Hello world");
    assert_eq!(value["remote_run_id"], "remote-1");

    let requests = mock.requests();
    assert_eq!(requests.len(), 1);
    let (headers, body) = &requests[0];
    assert_eq!(headers["authorization"], "Bearer tok-secret-123");
    assert_eq!(headers["x-tenant"], "acme");
    assert_eq!(body["protocolVersion"], "1.0");
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "Summarise the report");

    let task_id = value["task_id"].as_str().unwrap();
    let task = h
        .registry
        .get(h.ctx.session_id, task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.state, SessionTaskState::Succeeded);
    assert_eq!(task.kind, TASK_KIND_EXTERNAL_AG_UI);
    assert_eq!(task.summary.as_deref(), Some("Hello world"));

    // The token is never persisted with the run.
    let run_id = value["agent_run_id"].as_str().unwrap();
    let stored = h
        .storage
        .get_value(h.ctx.session_id, &format!("agent_run:{run_id}"))
        .await
        .unwrap()
        .unwrap();
    assert!(stored.contains("PARTNER_TOKEN"));
    assert!(!stored.contains("tok-secret-123"));
}

#[tokio::test]
async fn background_spawn_is_waitable_via_generic_wait_task() {
    let (url, _mock) = mock_agent(vec![Reply::Events(vec![
        started(),
        text("done"),
        finished("remote-1"),
    ])])
    .await;
    let h = harness();
    let value = success(
        spawn_tool(&agent_config(&url, json!({})))
            .execute_with_context(spawn_args("background"), &h.ctx)
            .await,
    );
    let task_id = value["task_id"].as_str().unwrap();
    // A background run wakes the parent on input requests too.
    let task = h
        .registry
        .get(h.ctx.session_id, task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.wake_policy, TaskWakePolicy::OnActivity);

    let waited = wait(&h.ctx, task_id).await;
    assert_eq!(waited["timed_out"], false, "{waited}");
    assert_eq!(waited["task"]["state"], "succeeded");
    assert_eq!(waited["task"]["summary"], "done");
}

#[tokio::test]
async fn interrupt_parks_the_task_and_message_task_resumes_the_thread() {
    let (url, mock) = mock_agent(vec![
        Reply::Events(vec![
            started(),
            text("Need a colour."),
            json!({ "type": "RUN_FINISHED", "threadId": "t", "runId": "remote-1",
                    "outcome": { "type": "interrupt", "interrupts": [
                        { "id": "i1", "reason": "input_required", "message": "Which colour?" }
                    ] } }),
        ]),
        Reply::Events(vec![
            json!({ "type": "RUN_STARTED", "threadId": "t", "runId": "remote-2" }),
            text("Blue it is."),
            finished("remote-2"),
        ]),
    ])
    .await;
    let h = harness();
    let value = success(
        spawn_tool(&agent_config(&url, json!({})))
            .execute_with_context(spawn_args("background"), &h.ctx)
            .await,
    );
    let task_id = value["task_id"].as_str().unwrap().to_string();

    let waited = wait(&h.ctx, &task_id).await;
    assert_eq!(waited["task"]["state"], "awaiting_input", "{waited}");
    let prompt = waited["task"]["input_request"]["prompt"].as_str().unwrap();
    assert!(prompt.contains("[i1] Which colour?"), "{prompt}");

    let delivered = success(
        MessageTaskTool
            .execute_with_context(json!({ "task_id": task_id, "message": "blue" }), &h.ctx)
            .await,
    );
    assert_eq!(delivered["delivery"], "delivered", "{delivered}");

    let waited = wait(&h.ctx, &task_id).await;
    assert_eq!(waited["task"]["state"], "succeeded", "{waited}");
    assert_eq!(waited["task"]["summary"], "Blue it is.");

    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    let (first, second) = (&requests[0].1, &requests[1].1);
    assert_eq!(second["threadId"], first["threadId"]);
    assert_eq!(second["parentRunId"], "remote-1");
    assert_eq!(
        second["resume"],
        json!([{ "interruptId": "i1", "status": "resolved", "payload": "blue" }])
    );
    // The resumed run carries the conversation so far.
    let roles: Vec<&str> = second["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["user", "assistant"]);
}

#[tokio::test]
async fn message_task_is_refused_while_no_input_is_awaited() {
    let (url, _mock) = mock_agent(vec![Reply::Events(vec![
        started(),
        text("ok"),
        finished("remote-1"),
    ])])
    .await;
    let h = harness();
    let value = success(
        spawn_tool(&agent_config(&url, json!({})))
            .execute_with_context(spawn_args("foreground"), &h.ctx)
            .await,
    );
    let delivered = success(
        MessageTaskTool
            .execute_with_context(
                json!({ "task_id": value["task_id"], "message": "more" }),
                &h.ctx,
            )
            .await,
    );
    let delivery = delivered["delivery"].as_str().unwrap();
    assert!(
        delivery.contains("only a run waiting for input"),
        "{delivery}"
    );
}

#[tokio::test]
async fn cancel_task_closes_the_stream() {
    let closed = Arc::new(AtomicBool::new(false));
    let (url, mock) = mock_agent(vec![Reply::Hang(
        vec![started(), text("working...")],
        closed.clone(),
    )])
    .await;
    let h = harness();
    let value = success(
        spawn_tool(&agent_config(&url, json!({})))
            .execute_with_context(spawn_args("background"), &h.ctx)
            .await,
    );
    let task_id = value["task_id"].as_str().unwrap().to_string();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while mock.requests().is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "no request arrived");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    success(
        CancelTaskTool
            .execute_with_context(json!({ "task_id": task_id }), &h.ctx)
            .await,
    );
    while !closed.load(Ordering::SeqCst) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the stream was not closed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let task = h
        .registry
        .get(h.ctx.session_id, &task_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.state, SessionTaskState::Canceled);
    let run_id = value["agent_run_id"].as_str().unwrap();
    assert_eq!(
        load_run(&h.ctx, run_id).await.unwrap().status,
        AgUiRunStatus::Canceled
    );
}

#[tokio::test]
async fn remote_run_error_fails_the_task() {
    let (url, _mock) = mock_agent(vec![Reply::Events(vec![
        started(),
        json!({ "type": "RUN_ERROR", "message": "quota exceeded", "code": "E_QUOTA" }),
    ])])
    .await;
    let h = harness();
    let value = success(
        spawn_tool(&agent_config(&url, json!({})))
            .execute_with_context(spawn_args("foreground"), &h.ctx)
            .await,
    );
    assert_eq!(value["status"], "failed");
    assert_eq!(value["error"], "quota exceeded (E_QUOTA)");
    let task = h
        .registry
        .get(h.ctx.session_id, value["task_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.error.unwrap().kind, "remote_failed");
}

#[tokio::test]
async fn an_http_error_fails_the_run() {
    // No queued reply: the mock answers 500.
    let (url, mock) = mock_agent(vec![]).await;
    let h = harness();
    let value = success(
        spawn_tool(&agent_config(&url, json!({})))
            .execute_with_context(spawn_args("foreground"), &h.ctx)
            .await,
    );
    assert_eq!(value["status"], "failed", "{value}");
    assert!(
        value["error"].as_str().unwrap().contains("HTTP 500"),
        "{value}"
    );
    assert_eq!(mock.requests().len(), 1);
}

#[tokio::test]
async fn a_protocol_violation_fails_the_run() {
    let (url, _mock) = mock_agent(vec![Reply::Events(vec![
        json!({ "type": "TEXT_MESSAGE_START", "messageId": "m1" }),
        started(),
    ])])
    .await;
    let h = harness();
    let value = success(
        spawn_tool(&agent_config(&url, json!({})))
            .execute_with_context(spawn_args("foreground"), &h.ctx)
            .await,
    );
    assert_eq!(value["status"], "failed");
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("AG-UI protocol violation"),
        "{value}"
    );
}

#[tokio::test]
async fn result_schema_validates_the_run_finished_result() {
    let finished_with = |result: Value| {
        Reply::Events(vec![
            started(),
            json!({ "type": "RUN_FINISHED", "threadId": "t", "runId": "remote-1", "result": result }),
        ])
    };
    let (url, _mock) = mock_agent(vec![
        finished_with(json!({ "answer": "42" })),
        finished_with(json!({ "answer": 42 })),
    ])
    .await;
    let h = harness();
    let tool = spawn_tool(&agent_config(&url, json!({})));
    let mut args = spawn_args("foreground");
    args["result_schema"] = json!({
        "type": "object",
        "properties": { "answer": { "type": "string" } },
        "required": ["answer"],
    });

    let ok = success(tool.execute_with_context(args.clone(), &h.ctx).await);
    assert_eq!(ok["status"], "completed", "{ok}");
    assert!(ok["result_path"].as_str().unwrap().ends_with("result.json"));

    let mismatch = success(tool.execute_with_context(args, &h.ctx).await);
    assert_eq!(mismatch["status"], "failed", "{mismatch}");
    let task = h
        .registry
        .get(h.ctx.session_id, mismatch["task_id"].as_str().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(task.error.unwrap().kind, "schema_mismatch");
}

#[tokio::test]
async fn a_missing_secret_fails_before_any_request() {
    let (url, mock) = mock_agent(vec![]).await;
    let h = harness();
    let value = success(
        spawn_tool(&agent_config(
            &url,
            json!({ "bearer_token_secret": "PARTNER_TOKEN" }),
        ))
        .execute_with_context(spawn_args("foreground"), &h.ctx)
        .await,
    );
    assert_eq!(value["status"], "failed");
    assert!(
        value["error"].as_str().unwrap().contains("is not set"),
        "{value}"
    );
    assert!(mock.requests().is_empty());
}

#[tokio::test]
async fn the_network_acl_blocks_before_any_request() {
    let (url, mock) = mock_agent(vec![]).await;
    let h = harness();
    let ctx = h
        .ctx
        .clone()
        .with_network_access(Some(NetworkAccessList::allow_only(["api.example.com"])));
    let value = success(
        spawn_tool(&agent_config(&url, json!({})))
            .execute_with_context(spawn_args("foreground"), &ctx)
            .await,
    );
    assert_eq!(value["status"], "failed");
    assert!(
        value["error"]
            .as_str()
            .unwrap()
            .contains("blocked by network access policy"),
        "{value}"
    );
    assert!(mock.requests().is_empty());
}

#[tokio::test]
async fn spawn_rejects_unknown_agents_bad_modes_and_message_schema() {
    let tool = spawn_tool(&agent_config("http://127.0.0.1:1/agent", json!({})));
    let h = harness();
    let cases = [
        (
            json!({ "instructions": "x", "target": { "type": AG_UI_TARGET_TYPE, "id": "nope" } }),
            "Unknown external AG-UI agent",
        ),
        (
            json!({ "instructions": "x", "target": { "type": AG_UI_TARGET_TYPE } }),
            "target.id",
        ),
        (
            json!({ "instructions": "x", "target": { "type": AG_UI_TARGET_TYPE, "id": "partner" },
                    "mode": "wait" }),
            "Invalid mode",
        ),
        (
            json!({ "instructions": "x", "target": { "type": AG_UI_TARGET_TYPE, "id": "partner" },
                    "message_schema": { "type": "object" } }),
            "message_schema is not supported",
        ),
    ];
    for (args, needle) in cases {
        match tool.execute_with_context(args, &h.ctx).await {
            ToolExecutionResult::ToolError(message) => {
                assert!(message.contains(needle), "{message}")
            }
            other => panic!("expected a tool error containing {needle}, got {other:?}"),
        }
    }
    assert!(
        h.storage
            .list_keys(h.ctx.session_id)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn background_spawn_requires_a_task_registry() {
    let tool = spawn_tool(&agent_config("http://127.0.0.1:1/agent", json!({})));
    let ctx = ToolContext::with_stores(
        SessionId::new(),
        Arc::new(InMemorySessionFileStore::new()),
        Arc::new(InMemorySessionStorageStore::new()),
    );
    match tool
        .execute_with_context(spawn_args("background"), &ctx)
        .await
    {
        ToolExecutionResult::ToolError(message) => {
            assert!(message.contains("requires session_task_registry"))
        }
        other => panic!("expected a tool error, got {other:?}"),
    }
}

#[test]
fn config_validation_refuses_unsafe_urls_credential_headers_and_bad_secret_names() {
    let cap = AgUiDelegationCapability;
    let one = |agent: Value| json!({ "agents": [agent] });
    let base = || json!({ "id": "a", "name": "A", "url": "https://agent.example.com/run" });
    assert!(cap.validate_config(&one(base())).is_ok());
    assert!(cap.validate_config(&Value::Null).is_ok());

    let mut local = base();
    local["url"] = json!("http://127.0.0.1:8080/run");
    assert!(cap.validate_config(&one(local.clone())).is_err());
    local["allow_local_urls"] = json!(true);
    assert!(cap.validate_config(&one(local)).is_ok());

    let mut metadata = base();
    metadata["url"] = json!("http://169.254.169.254/latest");
    assert!(cap.validate_config(&one(metadata)).is_err());

    let mut ftp = base();
    ftp["url"] = json!("ftp://agent.example.com/run");
    ftp["allow_local_urls"] = json!(true);
    assert!(cap.validate_config(&one(ftp)).is_err());

    for header in ["Authorization", "cookie", "Proxy-Authorization"] {
        let mut agent = base();
        agent["headers"] = json!({ header: "x" });
        let err = cap.validate_config(&one(agent)).unwrap_err();
        assert!(err.contains("bearer_token_secret"), "{err}");
    }

    for secret in ["", "has space", "a/b"] {
        let mut agent = base();
        agent["bearer_token_secret"] = json!(secret);
        assert!(cap.validate_config(&one(agent)).is_err(), "{secret:?}");
    }

    let mut extra = base();
    extra["token"] = json!("inline-secret");
    assert!(cap.validate_config(&one(extra)).is_err());

    assert!(
        cap.validate_config(&json!({ "agents": [base(), base()] }))
            .unwrap_err()
            .contains("listed twice")
    );
}

#[test]
fn answers_map_onto_the_open_interrupts() {
    let open = [
        Interrupt::new("i1", "tool_approval"),
        Interrupt::new("i2", "input_required"),
    ];

    let all = resume_from_answer(&open, "yes").unwrap();
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|e| e.payload == Some(json!("yes"))));

    let keyed = resume_from_answer(&open, r#"{"i2": {"colour": "blue"}}"#).unwrap();
    assert_eq!(keyed[0].interrupt_id, "i1");
    assert_eq!(keyed[0].status, ResumeStatus::Cancelled);
    assert_eq!(keyed[1].status, ResumeStatus::Resolved);
    assert_eq!(keyed[1].payload, Some(json!({ "colour": "blue" })));

    // An object that is not keyed by interrupt ids is one answer for all.
    let object = resume_from_answer(&open, r#"{"approved": true}"#).unwrap();
    assert!(
        object
            .iter()
            .all(|e| e.payload == Some(json!({ "approved": true })))
    );
}

#[tokio::test]
async fn a_lost_stream_is_not_reattached() {
    let h = harness();
    let config = agent_config("http://127.0.0.1:1/agent", json!({}));
    let parsed = AgUiDelegationConfig::from_value(&config).unwrap();
    let mut record = super::run::AgUiRunRecord::new(
        &parsed.agents[0],
        "x".into(),
        SpawnMode::Background,
        true,
        None,
    );
    let task = h
        .registry
        .create(CreateSessionTask {
            session_id: h.ctx.session_id,
            id: None,
            kind: TASK_KIND_EXTERNAL_AG_UI.to_string(),
            display_name: "Partner".into(),
            spec: json!({ "run_id": record.run_id }),
            state: SessionTaskState::Running,
            links: TaskLinks::default(),
            wake_policy: TaskWakePolicy::Silent,
        })
        .await
        .unwrap();
    record.task_id = Some(task.id.clone());
    record.status = AgUiRunStatus::Working;
    save_run(&h.ctx, &record).await.unwrap();

    let executor = AgUiAgentTaskExecutor;
    assert!(!executor.can_reattach());
    let err = executor.start(&task, &h.ctx).await.unwrap_err();
    assert!(err.to_string().contains("cannot be re-attached"), "{err}");
}

#[tokio::test]
async fn capability_contract() {
    let cap = AgUiDelegationCapability;
    assert_eq!(cap.id(), "ag_ui_delegation");
    assert_eq!(cap.dependencies(), vec![SESSION_TASKS_CAPABILITY_ID]);
    assert_eq!(cap.risk_level(), RiskLevel::High);
    assert!(
        everruns_core::session_task::find_task_executor(TASK_KIND_EXTERNAL_AG_UI).is_some(),
        "the executor is registered for its own task kind"
    );
    let config = agent_config(
        "http://127.0.0.1:1/agent",
        json!({ "description": "Reports" }),
    );
    let provider = cap.delegation_target_with_config(&config).unwrap();
    assert_eq!(provider.target_type, AG_UI_TARGET_TYPE);
    let prompt = cap
        .system_prompt_contribution_with_config(
            &SystemPromptContext::without_file_store(SessionId::new()),
            &config,
        )
        .await
        .unwrap();
    assert!(
        prompt.contains("Partner Agent (partner) — Reports"),
        "{prompt}"
    );
    assert!(prompt.contains("message_task"));
}
