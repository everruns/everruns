//! MCP 2026-07-28 Tasks extension (SEP-2663) on `/mcp`.
//!
//! A task is the turn one `agent_run` / `session_send_message` started. These
//! tests pin the server's side of the wire: the handle, the lifecycle read from
//! the event log, terminal immutability, the acknowledgements and the errors.
//! `mcp_tasks_interop_test.rs` drives the same surface with the official
//! TypeScript client and validates it against the published schema.

use super::*;
use crate::channel_a2a_pact_test::server as replying_server;
use std::time::Duration;

/// Per-request `_meta` declaring the Tasks extension, as a 2026 client sends it.
fn tasks_opt_in_meta() -> Value {
    json!({
        "io.modelcontextprotocol/clientCapabilities": {
            "extensions": { "io.modelcontextprotocol/tasks": {} }
        }
    })
}

/// tools/call with the 2026 protocol header and the Tasks opt-in.
async fn task_tool_call(server: &TestServer, tool: &str, arguments: Value) -> Value {
    mcp_call_with_headers(
        server,
        "tools/call",
        json!({ "name": tool, "arguments": arguments, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await
}

async fn task_method(server: &TestServer, method: &str, params: Value) -> Value {
    let mut params = params;
    params["_meta"] = tasks_opt_in_meta();
    mcp_call_with_headers(
        server,
        method,
        params,
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await
}

async fn tasks_get(server: &TestServer, task_id: &str) -> Value {
    task_method(server, "tasks/get", json!({ "taskId": task_id })).await
}

/// Poll `tasks/get` until the task is terminal.
async fn settled(server: &TestServer, task_id: &str) -> Value {
    for _ in 0..200 {
        let resp = tasks_get(server, task_id).await;
        if matches!(
            resp["result"]["status"].as_str(),
            Some("completed" | "cancelled" | "failed")
        ) {
            return resp["result"].clone();
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("task {task_id} never settled");
}

fn created_task_id(resp: &Value) -> String {
    assert_eq!(resp["result"]["resultType"], "task", "{resp}");
    resp["result"]["taskId"]
        .as_str()
        .unwrap_or_else(|| panic!("taskId missing: {resp}"))
        .to_string()
}

/// `{org}.{session}.{message}` → the session id.
fn session_of(task_id: &str) -> String {
    task_id.split('.').nth(1).unwrap().to_string()
}

fn assert_task_fields(task: &Value) {
    for field in ["taskId", "status", "createdAt", "lastUpdatedAt"] {
        assert!(task[field].is_string(), "{field} missing: {task}");
    }
    assert!(task["ttlMs"].is_u64(), "ttlMs missing: {task}");
    assert!(
        task["pollIntervalMs"].is_u64(),
        "pollIntervalMs missing: {task}"
    );
    for field in ["createdAt", "lastUpdatedAt"] {
        chrono::DateTime::parse_from_rfc3339(task[field].as_str().unwrap())
            .unwrap_or_else(|_| panic!("{field} is not RFC 3339: {task}"));
    }
}

async fn emit_event(server: &TestServer, session_id: &str, event_type: &str, data: Value) {
    server
        .db
        .create_event(everruns_server::storage::CreateEventRow {
            session_id: session_id.parse().unwrap(),
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data,
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit event");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_2026_initialize_advertises_the_extension_and_2025_does_not() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call(
        &server,
        "initialize",
        json!({ "protocolVersion": MCP_PROTOCOL_VERSION_2026 }),
    )
    .await;
    assert!(
        resp["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/tasks"].is_object(),
        "{resp}"
    );
    for version in [MCP_PROTOCOL_VERSION_LATEST, MCP_PROTOCOL_VERSION_FALLBACK] {
        let resp = mcp_call(&server, "initialize", json!({ "protocolVersion": version })).await;
        assert!(
            resp["result"]["capabilities"].get("extensions").is_none(),
            "{version}: {resp}"
        );
    }
}

/// The call answers with a CreateTaskResult in place of the tool result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_run_answers_with_a_task_for_the_turn_it_started() {
    let server = TestServer::in_memory().await;
    let resp = task_tool_call(&server, "agent_run", json!({ "message": "Hello" })).await;

    let task_id = created_task_id(&resp);
    let parts: Vec<&str> = task_id.split('.').collect();
    assert_eq!(parts.len(), 3, "{task_id}");
    assert_eq!(parts[0], DEFAULT_ORG_PUBLIC_ID);
    assert!(parts[1].starts_with("session_"), "{task_id}");
    assert!(parts[2].starts_with("message_"), "{task_id}");
    assert_task_fields(&resp["result"]);
    assert!(
        resp["result"].get("content").is_none(),
        "a task replaces the result: {resp}"
    );
}

/// Only a client that declared the extension gets a task.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_client_that_did_not_declare_tasks_gets_the_plain_result() {
    let server = TestServer::in_memory().await;
    let undeclared = mcp_call_with_headers(
        &server,
        "tools/call",
        json!({ "name": "agent_run", "arguments": { "message": "Hi" } }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;
    assert_eq!(undeclared["result"]["resultType"], "complete");
    assert!(undeclared["result"].get("taskId").is_none());

    let legacy = mcp_call_with_headers(
        &server,
        "tools/call",
        json!({ "name": "agent_run", "arguments": { "message": "Hi" }, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_LATEST)],
    )
    .await;
    assert!(!tool_is_error(&legacy), "{}", tool_text(&legacy));
    assert!(legacy["result"].get("resultType").is_none());
    assert!(legacy["result"].get("taskId").is_none());
}

/// A finished turn is a completed task whose result is the turn's reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_finished_turn_completes_the_task_with_its_reply() {
    let (server, _runner) = replying_server().await;
    let resp = task_tool_call(&server, "agent_run", json!({ "message": "Hello" })).await;
    let task_id = created_task_id(&resp);

    let task = settled(&server, &task_id).await;
    assert_eq!(task["resultType"], "complete");
    assert_eq!(task["status"], "completed", "{task}");
    assert_task_fields(&task);
    assert_eq!(task["result"]["resultType"], "complete");
    assert_eq!(task["result"]["content"][0]["text"], "Reply 1");
    assert_eq!(task["result"]["structuredContent"]["output"], "Reply 1");
    assert_eq!(
        task["result"]["structuredContent"]["session_id"],
        session_of(&task_id)
    );
    assert!(task["result"].get("isError").is_none(), "{task}");
}

/// Terminal states never change: the next message is a new task, and the old
/// handle keeps its own reply.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_message_is_its_own_task_and_a_finished_one_never_changes() {
    let (server, _runner) = replying_server().await;
    let first =
        created_task_id(&task_tool_call(&server, "agent_run", json!({ "message": "one" })).await);
    let first_done = settled(&server, &first).await;

    let second = created_task_id(
        &task_tool_call(
            &server,
            "session_send_message",
            json!({ "session_id": session_of(&first), "message": "two" }),
        )
        .await,
    );
    assert_ne!(first, second);
    assert_eq!(session_of(&first), session_of(&second));
    let second_done = settled(&server, &second).await;
    assert_eq!(second_done["result"]["content"][0]["text"], "Reply 2");

    let first_again = tasks_get(&server, &first).await["result"].clone();
    assert_eq!(first_again["status"], "completed");
    assert_eq!(first_again["result"]["content"][0]["text"], "Reply 1");
    assert_eq!(first_again["lastUpdatedAt"], first_done["lastUpdatedAt"]);
}

/// A failed or stopped turn is a tool error: `completed` with `isError`, since
/// the spec reserves `failed` for JSON-RPC errors.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_turn_completes_with_a_tool_error() {
    for (event_type, data, expected) in [
        (
            "turn.failed",
            json!({ "error": "provider unavailable" }),
            "The turn failed: provider unavailable",
        ),
        (
            "turn.sealed",
            json!({ "reason": "no forward progress" }),
            "The turn was stopped: no forward progress",
        ),
    ] {
        let server = TestServer::in_memory().await;
        let task_id = created_task_id(
            &task_tool_call(&server, "agent_run", json!({ "message": "Hello" })).await,
        );
        emit_event(&server, &session_of(&task_id), event_type, data).await;

        let task = tasks_get(&server, &task_id).await["result"].clone();
        assert_eq!(task["status"], "completed", "{task}");
        assert_eq!(task["result"]["isError"], true, "{task}");
        assert_eq!(task["result"]["content"][0]["text"], expected);
        assert_eq!(task["statusMessage"], expected);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_cancelled_turn_is_a_cancelled_task() {
    let server = TestServer::in_memory().await;
    let task_id =
        created_task_id(&task_tool_call(&server, "agent_run", json!({ "message": "Hi" })).await);
    emit_event(
        &server,
        &session_of(&task_id),
        "turn.cancelled",
        json!({ "reason": "User requested cancellation" }),
    )
    .await;

    let task = tasks_get(&server, &task_id).await["result"].clone();
    assert_eq!(task["status"], "cancelled", "{task}");
    assert!(task.get("result").is_none(), "{task}");
}

/// An open turn is `working`, and says why in words.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_turn_is_working_with_a_status_message() {
    let server = TestServer::in_memory().await;
    let task_id =
        created_task_id(&task_tool_call(&server, "agent_run", json!({ "message": "Hi" })).await);
    let task = tasks_get(&server, &task_id).await["result"].clone();
    assert_eq!(task["status"], "working", "{task}");
    assert!(task["statusMessage"].is_string(), "{task}");
    assert!(task.get("result").is_none(), "{task}");
}

/// The structured result a task reported rides in `structuredContent`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reported_structured_result_is_part_of_the_task_result() {
    let (server, _runner) = replying_server().await;
    let task_id =
        created_task_id(&task_tool_call(&server, "agent_run", json!({ "message": "Hi" })).await);
    settled(&server, &task_id).await;
    let plain = tasks_get(&server, &task_id).await["result"].clone();
    assert!(
        plain["result"]["structuredContent"]
            .get("structured_result")
            .is_none()
    );

    let expected = json!({ "verdict": "ok", "score": 42 });
    seed_structured_result(&server, &session_of(&task_id), expected.clone()).await;
    let task = tasks_get(&server, &task_id).await["result"].clone();
    assert_eq!(
        task["result"]["structuredContent"]["structured_result"],
        expected
    );
}

/// One answer for malformed, foreign and missing handles.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_task_is_invalid_params() {
    let server = TestServer::in_memory().await;
    let task_id =
        created_task_id(&task_tool_call(&server, "agent_run", json!({ "message": "Hi" })).await);
    let session = session_of(&task_id);
    let message = task_id.split('.').nth(2).unwrap();
    for unknown in [
        // The pre-turn handle shape: a bare session id.
        session.clone(),
        format!(
            "{DEFAULT_ORG_PUBLIC_ID}.{session}.{}",
            everruns_contracts::typed_id::MessageId::new()
        ),
        // An org the caller is not a member of.
        format!("org_000000000000000000000000000000ff.{session}.{message}"),
        "garbage".to_string(),
    ] {
        for method in ["tasks/get", "tasks/cancel", "tasks/update"] {
            let resp = task_method(
                &server,
                method,
                json!({ "taskId": unknown, "inputResponses": {} }),
            )
            .await;
            assert_eq!(resp["error"]["code"], -32602, "{method} {unknown}: {resp}");
        }
    }
    let missing = task_method(&server, "tasks/get", json!({})).await;
    assert_eq!(missing["error"]["code"], -32602, "{missing}");
}

/// A 2026 client that did not declare the extension gets -32021; the methods
/// do not exist for 2025 clients at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tasks_methods_need_the_declared_extension_and_the_2026_protocol() {
    let server = TestServer::in_memory().await;
    let task_id =
        created_task_id(&task_tool_call(&server, "agent_run", json!({ "message": "Hi" })).await);
    for method in ["tasks/get", "tasks/cancel", "tasks/update"] {
        let undeclared = mcp_call_with_headers(
            &server,
            method,
            json!({ "taskId": task_id, "inputResponses": {} }),
            vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
        )
        .await;
        assert_eq!(
            undeclared["error"]["code"], -32021,
            "{method}: {undeclared}"
        );

        let legacy = mcp_call_with_headers(
            &server,
            method,
            json!({ "taskId": task_id, "_meta": tasks_opt_in_meta() }),
            vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_LATEST)],
        )
        .await;
        assert_eq!(legacy["error"]["code"], -32601, "{method}: {legacy}");
    }
}

/// Responses under keys that are not outstanding are ignored, and success is
/// an empty acknowledgement.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_without_outstanding_input_is_acknowledged_and_ignored() {
    let server = TestServer::in_memory().await;
    let task_id =
        created_task_id(&task_tool_call(&server, "agent_run", json!({ "message": "Hi" })).await);
    let resp = task_method(
        &server,
        "tasks/update",
        json!({ "taskId": task_id, "inputResponses": { "ask_user:nope": { "action": "cancel" } } }),
    )
    .await;
    assert_eq!(
        resp["result"],
        json!({ "resultType": "complete" }),
        "{resp}"
    );

    let malformed = task_method(&server, "tasks/update", json!({ "taskId": task_id })).await;
    assert_eq!(malformed["error"]["code"], -32602, "{malformed}");
}

/// Cancel is acknowledged with an empty result, also for a finished task.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_is_acknowledged_with_an_empty_result() {
    let (server, _runner) = replying_server().await;
    let task_id =
        created_task_id(&task_tool_call(&server, "agent_run", json!({ "message": "Hi" })).await);
    settled(&server, &task_id).await;
    let resp = task_method(&server, "tasks/cancel", json!({ "taskId": task_id })).await;
    assert_eq!(
        resp["result"],
        json!({ "resultType": "complete" }),
        "{resp}"
    );
    assert_eq!(
        tasks_get(&server, &task_id).await["result"]["status"],
        "completed"
    );
}

/// Only a schema-bound result counts: another task's result file never leaks
/// into the task result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_non_schema_result_file_is_not_a_structured_result() {
    let (server, _runner) = replying_server().await;
    let task_id =
        created_task_id(&task_tool_call(&server, "agent_run", json!({ "message": "Hi" })).await);
    settled(&server, &task_id).await;
    seed_non_schema_result_path(&server, &session_of(&task_id), json!({ "internal": "x" })).await;
    let task = tasks_get(&server, &task_id).await["result"].clone();
    assert!(
        task["result"]["structuredContent"]
            .get("structured_result")
            .is_none(),
        "{task}"
    );
}

/// Seed a deterministic structured result (EVE-678) on `session_id`: create a
/// `result_schema`-bound task owned by the session, point it at a result file,
/// and write that file into the session's workspace VFS.
async fn seed_structured_result(server: &TestServer, session_id: &str, result: Value) {
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskState, SessionTaskUpdate, new_session_task, task_result_path,
    };
    use everruns_server::storage::CreateSessionFileRow;

    let sid = session_id.parse::<SessionId>().expect("valid session id");
    let session = server
        .db
        .get_session(DEFAULT_ORG_ID, sid)
        .await
        .expect("get_session")
        .expect("session exists");

    let task = new_session_task(
        CreateSessionTask {
            session_id: sid,
            id: None,
            kind: "subagent".to_string(),
            display_name: "structured result".to_string(),
            spec: json!({ "result_schema": { "type": "object" } }),
            state: SessionTaskState::Running,
            links: Default::default(),
            wake_policy: Default::default(),
        },
        chrono::Utc::now(),
    );
    server
        .db
        .create_session_task(&task)
        .await
        .expect("create task");

    let path = task_result_path(&task.id);
    server
        .db
        .update_session_task(
            sid,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                result_path: Some(path.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("set result_path");

    server
        .db
        .create_session_file(CreateSessionFileRow {
            session_id: SessionId::from_uuid(session.workspace_id),
            path,
            content: Some(serde_json::to_vec(&result).unwrap()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .expect("write result file");
}

async fn seed_non_schema_result_path(server: &TestServer, session_id: &str, result: Value) {
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskState, SessionTaskUpdate, new_session_task,
    };
    use everruns_server::storage::CreateSessionFileRow;

    let sid = session_id.parse::<SessionId>().expect("valid session id");
    let session = server
        .db
        .get_session(DEFAULT_ORG_ID, sid)
        .await
        .expect("get_session")
        .expect("session exists");

    let task = new_session_task(
        CreateSessionTask {
            session_id: sid,
            id: None,
            kind: "background_tool".to_string(),
            display_name: "background result".to_string(),
            spec: json!({ "tool": "internal_scan" }),
            state: SessionTaskState::Running,
            links: Default::default(),
            wake_policy: Default::default(),
        },
        chrono::Utc::now(),
    );
    server
        .db
        .create_session_task(&task)
        .await
        .expect("create non-schema task");

    let path = format!("/.background/{}/result.json", task.id);
    server
        .db
        .update_session_task(
            sid,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                result_path: Some(path.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("set non-schema result_path");

    server
        .db
        .create_session_file(CreateSessionFileRow {
            session_id: SessionId::from_uuid(session.workspace_id),
            path,
            content: Some(serde_json::to_vec(&result).unwrap()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .expect("write non-schema result file");
}
