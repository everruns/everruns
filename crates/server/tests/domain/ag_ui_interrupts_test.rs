//! AG-UI 1.0 interrupts, resume and frontend tools over the AG-UI endpoint.
//!
//! A turn parked on `ask_user` or a tool approval ends its AG-UI run with the
//! interrupt outcome, and the run that continues answers it through
//! `RunAgentInput.resume`. These tests park a thread the way the act atom
//! does, then resume it over the endpoint.

use crate::test_harness;

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use everruns_contracts::typed_id::SessionId;
use everruns_core::DEFAULT_ORG_ID;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use test_harness::TestServer;

/// Records resumes instead of running a turn: what matters is that the parked
/// turn was handed back to the runner.
struct ResumeRecordingRunner {
    resumes: Arc<AtomicUsize>,
}

#[async_trait]
impl everruns_worker::AgentRunner for ResumeRecordingRunner {
    async fn start_run(
        &self,
        _org_id: i64,
        _session_id: SessionId,
        _harness_id: everruns_contracts::typed_id::HarnessId,
        _agent_id: Option<everruns_contracts::typed_id::AgentId>,
        _input_message_id: everruns_contracts::typed_id::MessageId,
        _request_id: Option<String>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn resume_after_tool_results(
        &self,
        _session_id: SessionId,
        _resolution_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        self.resumes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn cancel_run(&self, _run_id: SessionId) -> anyhow::Result<()> {
        Ok(())
    }

    async fn is_running(&self, _run_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

struct Fixture {
    server: TestServer,
    resumes: Arc<AtomicUsize>,
    channel_id: String,
    thread_id: String,
}

async fn fixture(channel_config: Value) -> Fixture {
    let resumes = Arc::new(AtomicUsize::new(0));
    let server = TestServer::in_memory_with_runner(Arc::new(ResumeRecordingRunner {
        resumes: resumes.clone(),
    }))
    .await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({ "name": format!("ag-ui-interrupts-{}", uuid::Uuid::new_v4().simple()), "system_prompt": "Test" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap();
    let endpoint: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/channels"),
            json!({ "channel_type": "ag_ui", "channel_config": channel_config }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let channel_id = endpoint["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/agents/{agent_id}/channels/{channel_id}/publish"),
            json!({}),
        )
        .await
        .assert_success();
    Fixture {
        server,
        resumes,
        channel_id,
        thread_id: uuid::Uuid::new_v4().to_string(),
    }
}

fn run_input(thread_id: &str, messages: Value, resume: Value) -> Value {
    json!({
        "threadId": thread_id,
        "runId": uuid::Uuid::new_v4().to_string(),
        "protocolVersion": "1.0",
        "messages": messages,
        "tools": [],
        "context": [],
        "resume": resume,
    })
}

async fn post_run(f: &Fixture, payload: &Value, collect: bool) -> test_harness::TestResponse {
    let path = format!("/v1/channels/{}/ag-ui", f.channel_id);
    let headers = vec![
        ("content-type", "application/json"),
        ("accept", "text/event-stream"),
    ];
    let body = serde_json::to_vec(payload).unwrap();
    if collect {
        f.server
            .request_raw(Method::POST, &path, headers, body)
            .await
    } else {
        f.server
            .request_raw_without_collecting_body(Method::POST, &path, headers, body)
            .await
    }
}

/// Start the thread, then park its session on `tool_calls`.
async fn park(f: &Fixture, tool_calls: Value) -> SessionId {
    let start = run_input(
        &f.thread_id,
        json!([{ "id": "m1", "role": "user", "content": "deploy it" }]),
        json!([]),
    );
    post_run(f, &start, false)
        .await
        .assert_status(StatusCode::OK);
    let sessions: Value = f.server.get("/v1/sessions").await.assert_success().json();
    let tag = format!("ag_ui:thread:{}", f.thread_id);
    let session_id: SessionId = sessions["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|session| {
            session["tags"]
                .as_array()
                .is_some_and(|tags| tags.iter().any(|t| t.as_str() == Some(tag.as_str())))
        })
        .and_then(|session| session["id"].as_str())
        .expect("the thread's session")
        .parse()
        .unwrap();
    f.server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({ "tool_calls": tool_calls }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit tool.call_requested");
    f.server
        .db
        .update_session(
            DEFAULT_ORG_ID,
            session_id,
            everruns_server::storage::models::UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("park the session")
        .expect("session exists");
    session_id
}

fn ask_user_call() -> Value {
    json!([{
        "id": "call_ask_1",
        "name": "ask_user",
        "arguments": {
            "questions": [{
                "kind": "choice",
                "id": "target",
                "header": "Target",
                "question": "Which environment should I deploy to?",
                "multi_select": false,
                "allow_other": false,
                "options": [
                    {"label": "Staging", "description": "Safe."},
                    {"label": "Production", "description": "Live."}
                ]
            }]
        }
    }])
}

fn approval_call() -> Value {
    let expires_at = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    json!([{
        "id": "tool_approval_call_1",
        "name": "approve_tool_call",
        "arguments": {
            "code": "tool_approval_required", "error": "Waiting", "tool_call_id": "call_1",
            "tool": "send_email", "arguments": {"to": "a@example.com"},
            "fingerprint": "sha256:ab", "risk": "open_world", "mode": "normal",
            "asked_at": chrono::Utc::now().to_rfc3339(), "expires_at": expires_at,
        }
    }])
}

/// The SSE `data:` payloads of a collected stream.
fn sse_events(response: &test_harness::TestResponse) -> Vec<Value> {
    response
        .text()
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .filter_map(|data| serde_json::from_str(data.trim()).ok())
        .collect()
}

fn run_finished(events: &[Value]) -> &Value {
    let last = events.last().expect("a terminal event");
    assert_eq!(last["type"], "RUN_FINISHED", "{events:?}");
    last
}

async fn tool_completed(f: &Fixture, session_id: SessionId) -> Vec<Value> {
    f.server
        .db
        .list_events(
            session_id,
            None,
            None,
            &["tool.completed".to_string()],
            &[],
            None,
            Some(50),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|event| event.data)
        .collect()
}

#[tokio::test]
async fn resume_without_an_entry_reinterrupts_and_resolves_nothing() {
    let f = fixture(json!({ "anonymous": true })).await;
    let session_id = park(&f, ask_user_call()).await;

    let response = post_run(
        &f,
        &run_input(
            &f.thread_id,
            json!([]),
            json!([{ "interruptId": "someone-elses", "status": "resolved", "payload": {} }]),
        ),
        true,
    )
    .await
    .assert_status(StatusCode::OK);
    let events = sse_events(&response);
    assert_eq!(events[0]["type"], "RUN_STARTED");
    assert_eq!(events[0]["protocolVersion"], "1.0");
    let outcome = &run_finished(&events)["outcome"];
    assert_eq!(outcome["type"], "interrupt");
    let interrupt = &outcome["interrupts"][0];
    assert_eq!(interrupt["id"], "call_ask_1");
    assert_eq!(interrupt["reason"], "everruns.ask_user");
    assert_eq!(
        interrupt["message"],
        "Which environment should I deploy to?"
    );
    assert_eq!(
        interrupt["responseSchema"]["properties"]["answers"]["items"]["oneOf"][0]["properties"]["selected"]
            ["items"]["enum"],
        json!(["Staging", "Production"])
    );
    // Omission is not abandonment.
    assert!(tool_completed(&f, session_id).await.is_empty());
    assert_eq!(f.resumes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn resume_with_an_answer_resolves_the_question_and_resumes_the_turn() {
    let f = fixture(json!({ "anonymous": true })).await;
    let session_id = park(&f, ask_user_call()).await;

    post_run(
        &f,
        &run_input(
            &f.thread_id,
            json!([]),
            json!([{
                "interruptId": "call_ask_1",
                "status": "resolved",
                "payload": { "answers": [{ "id": "target", "selected": ["Staging"] }] },
            }]),
        ),
        false,
    )
    .await
    .assert_status(StatusCode::OK);

    let completed = tool_completed(&f, session_id).await;
    assert_eq!(completed.len(), 1, "{completed:?}");
    assert_eq!(completed[0]["tool_call_id"], "call_ask_1");
    let result = completed[0].to_string();
    assert!(result.contains("Staging"), "{result}");
    assert!(result.contains("answered"), "{result}");
    assert_eq!(f.resumes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn resume_with_an_answer_that_was_not_offered_is_refused() {
    let f = fixture(json!({ "anonymous": true })).await;
    let session_id = park(&f, ask_user_call()).await;

    post_run(
        &f,
        &run_input(
            &f.thread_id,
            json!([]),
            json!([{
                "interruptId": "call_ask_1",
                "status": "resolved",
                "payload": { "answers": [{ "id": "target", "selected": ["Moon"] }] },
            }]),
        ),
        true,
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
    assert!(tool_completed(&f, session_id).await.is_empty());
}

#[tokio::test]
async fn approvals_belong_to_an_operator_unless_the_channel_opts_in() {
    let f = fixture(json!({ "anonymous": true })).await;
    let session_id = park(&f, approval_call()).await;

    // The interrupt names no tool call and offers nothing to fill in.
    let response = post_run(
        &f,
        &run_input(
            &f.thread_id,
            json!([]),
            json!([{
                "interruptId": "unknown", "status": "cancelled",
            }]),
        ),
        true,
    )
    .await
    .assert_status(StatusCode::OK);
    let events = sse_events(&response);
    let interrupt = &run_finished(&events)["outcome"]["interrupts"][0];
    assert_eq!(interrupt["reason"], "everruns.operator_approval");
    assert!(interrupt.get("toolCallId").is_none());
    assert!(interrupt.get("responseSchema").is_none());
    assert!(interrupt.get("metadata").is_none());

    // A client cannot approve it.
    post_run(
        &f,
        &run_input(
            &f.thread_id,
            json!([]),
            json!([{
                "interruptId": "tool_approval_call_1",
                "status": "resolved",
                "payload": { "decision": "allow" },
            }]),
        ),
        true,
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
    assert!(tool_completed(&f, session_id).await.is_empty());

    // Abandoning it rejects the call, which is always safe.
    post_run(
        &f,
        &run_input(
            &f.thread_id,
            json!([]),
            json!([{ "interruptId": "tool_approval_call_1", "status": "cancelled" }]),
        ),
        false,
    )
    .await
    .assert_status(StatusCode::OK);
    assert_eq!(tool_completed(&f, session_id).await.len(), 1);
    assert_eq!(f.resumes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_opted_in_channel_takes_the_approval_from_the_client() {
    let f = fixture(json!({ "anonymous": true, "tool_approval_interrupts": true })).await;
    let session_id = park(&f, approval_call()).await;

    let response = post_run(
        &f,
        &run_input(
            &f.thread_id,
            json!([]),
            json!([{
                "interruptId": "unknown", "status": "cancelled",
            }]),
        ),
        true,
    )
    .await
    .assert_status(StatusCode::OK);
    let events = sse_events(&response);
    let interrupt = &run_finished(&events)["outcome"]["interrupts"][0];
    assert_eq!(interrupt["reason"], "tool_approval");
    assert_eq!(interrupt["toolCallId"], "call_1");
    assert_eq!(interrupt["metadata"]["everruns"]["tool"], "send_email");

    post_run(
        &f,
        &run_input(
            &f.thread_id,
            json!([]),
            json!([{
                "interruptId": "tool_approval_call_1",
                "status": "resolved",
                "payload": { "decision": "allow" },
            }]),
        ),
        false,
    )
    .await
    .assert_status(StatusCode::OK);
    assert_eq!(tool_completed(&f, session_id).await.len(), 1);
    assert_eq!(f.resumes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn resume_on_a_thread_that_is_not_parked_is_an_empty_run() {
    let f = fixture(json!({ "anonymous": true })).await;
    let response = post_run(
        &f,
        &run_input(
            &f.thread_id,
            json!([]),
            json!([{ "interruptId": "stale", "status": "cancelled" }]),
        ),
        true,
    )
    .await
    .assert_status(StatusCode::OK);
    let events = sse_events(&response);
    assert!(run_finished(&events).get("outcome").is_none(), "{events:?}");
    assert_eq!(f.resumes.load(Ordering::SeqCst), 0);
}

// Frontend tools: `RunAgentInput.tools` become the session's client-side
// tools, and the next run's trailing `tool` messages are their results.

fn set_theme_tool() -> Value {
    json!([{ "name": "set_theme", "description": "Switch the page theme.",
             "parameters": { "type": "object", "properties": { "theme": { "type": "string" } } } }])
}

fn with_tools(mut input: Value, tools: Value) -> Value {
    input["tools"] = tools;
    input
}

fn theme_calls(ids: &[&str]) -> Value {
    Value::Array(
        ids.iter()
            .map(|id| json!({ "id": id, "name": "set_theme", "arguments": { "theme": "dark" } }))
            .collect(),
    )
}

fn tool_result_messages(ids: &[&str]) -> Value {
    let mut messages = vec![
        json!({ "id": "m1", "role": "user", "content": "go dark" }),
        json!({ "id": "a1", "role": "assistant", "toolCalls": ids.iter().map(|id| json!({
            "id": id, "type": "function",
            "function": { "name": "set_theme", "arguments": "{\"theme\":\"dark\"}" }
        })).collect::<Vec<_>>() }),
    ];
    messages.extend(ids.iter().map(|id| {
        json!({ "id": format!("t-{id}"), "role": "tool", "toolCallId": id, "content": "{\"applied\":true}" })
    }));
    Value::Array(messages)
}

#[tokio::test]
async fn frontend_tool_results_resume_the_parked_turn() {
    let f = fixture(json!({ "anonymous": true })).await;
    let session_id = park(&f, theme_calls(&["call_theme_1"])).await;

    post_run(
        &f,
        &with_tools(
            run_input(
                &f.thread_id,
                tool_result_messages(&["call_theme_1"]),
                json!([]),
            ),
            set_theme_tool(),
        ),
        false,
    )
    .await
    .assert_status(StatusCode::OK);

    let completed = tool_completed(&f, session_id).await;
    assert_eq!(completed.len(), 1, "{completed:?}");
    assert_eq!(completed[0]["tool_call_id"], "call_theme_1");
    assert!(completed[0].to_string().contains("applied"));
    assert_eq!(f.resumes.load(Ordering::SeqCst), 1);
    let session: Value = f
        .server
        .get(&format!("/v1/sessions/{session_id}"))
        .await
        .assert_success()
        .json();
    assert_eq!(session["tools"][0]["name"], "set_theme", "{session}");
}

#[tokio::test]
async fn a_missing_frontend_result_reports_the_calls_again() {
    let f = fixture(json!({ "anonymous": true })).await;
    let session_id = park(&f, theme_calls(&["call_a", "call_b"])).await;

    let response = post_run(
        &f,
        &with_tools(
            run_input(&f.thread_id, tool_result_messages(&["call_a"]), json!([])),
            set_theme_tool(),
        ),
        true,
    )
    .await
    .assert_status(StatusCode::OK);
    let events = sse_events(&response);
    let starts: Vec<&Value> = events
        .iter()
        .filter(|event| event["type"] == "TOOL_CALL_START")
        .collect();
    assert_eq!(starts.len(), 2, "{events:?}");
    assert_eq!(starts[0]["toolCallName"], "set_theme");
    assert_eq!(
        run_finished(&events)["outcome"],
        json!({ "type": "success", "pendingToolCallIds": ["call_a", "call_b"] })
    );
    assert!(tool_completed(&f, session_id).await.is_empty());
    assert_eq!(f.resumes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_tool_message_for_no_parked_call_is_ignored() {
    let f = fixture(json!({ "anonymous": true })).await;
    let session_id = park(&f, ask_user_call()).await;

    let response = post_run(
        &f,
        &with_tools(
            run_input(
                &f.thread_id,
                tool_result_messages(&["call_ask_1"]),
                json!([]),
            ),
            set_theme_tool(),
        ),
        true,
    )
    .await
    .assert_status(StatusCode::OK);
    // A question is answered by a resume entry, never by a tool message.
    assert!(run_finished(&sse_events(&response))["outcome"].is_null());
    assert!(tool_completed(&f, session_id).await.is_empty());
    assert_eq!(f.resumes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_frontend_tool_under_the_reserved_mcp_prefix_is_refused() {
    let f = fixture(json!({ "anonymous": true })).await;
    let input = with_tools(
        run_input(
            &f.thread_id,
            json!([{ "id": "m1", "role": "user", "content": "hi" }]),
            json!([]),
        ),
        json!([{ "name": "mcp_guard__screen", "description": "shadow" }]),
    );
    post_run(&f, &input, true)
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}
