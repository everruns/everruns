#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! MCP Tasks extension, client side (knowledge/integrations/mcp.md).
//!
//! A scripted [`EgressService`] plays a `2026-07-28` server that turns every
//! `tools/call` into a task, and asserts the client's half: declare the
//! extension, poll `tasks/get` with the task id in the param and the `Mcp-Name`
//! header, answer `input_required` once through `tasks/update`, read the
//! result or the failure, and cancel a task it stops waiting for.

use async_trait::async_trait;
use everruns_core::mcp::form_elicitation::FormElicitationPending;
use everruns_core::mcp::{
    FormAnswer, FormAnswerAction, FormAnswerStore, HttpTransport, McpConnection, McpTransport,
    StoredFormAnswer, StoredFormAnswers,
};
use everruns_core::{
    EgressRequest, EgressResponse, EgressResult, EgressService, EgressStreamResponse, McpContent,
    McpElicitationPolicy,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Public IP literal: passes SSRF validation without DNS, never actually dialed.
const URL: &str = "http://8.8.8.8/mcp";

/// How each task the scripted server creates behaves.
#[derive(Clone, Copy, PartialEq)]
enum Script {
    /// One `working` poll, then completed.
    Completes,
    /// Asks a form until answered, keeps showing the answered form for one
    /// more poll, then completes with the answer.
    AsksForm,
    Fails,
    CancelledByServer,
    /// Never finishes.
    RunsForever,
}

struct Request {
    body: Value,
    mcp_name: Option<String>,
}

struct TaskServer {
    script: Script,
    requests: Mutex<Vec<Request>>,
    polls: Mutex<BTreeMap<String, usize>>,
    answers: Mutex<BTreeMap<String, Value>>,
}

impl TaskServer {
    fn new(script: Script) -> Arc<Self> {
        Arc::new(Self {
            script,
            requests: Mutex::new(Vec::new()),
            polls: Mutex::new(BTreeMap::new()),
            answers: Mutex::new(BTreeMap::new()),
        })
    }

    fn requests(&self, method: &str) -> Vec<(Value, Option<String>)> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.body["method"] == method)
            .map(|request| (request.body.clone(), request.mcp_name.clone()))
            .collect()
    }

    fn task(&self, task_id: &str) -> Value {
        let poll = {
            let mut polls = self.polls.lock().unwrap();
            let count = polls.entry(task_id.to_string()).or_default();
            *count += 1;
            *count
        };
        let answer = self.answers.lock().unwrap().get(task_id).cloned();
        let base = json!({
            "resultType": "complete",
            "taskId": task_id,
            "createdAt": "2026-10-23T00:00:00Z",
            "lastUpdatedAt": "2026-10-23T00:00:00Z",
            "ttlMs": 60_000,
            "pollIntervalMs": 1,
        });
        let state = match (self.script, poll) {
            (Script::RunsForever, _) | (Script::Completes, 1) => {
                json!({ "status": "working", "statusMessage": "Building" })
            }
            (Script::Completes, _) => completed("built"),
            (Script::AsksForm, _) => match answer {
                None => asking(),
                // The server is slower to take the answer in than the client
                // is to poll: it still shows the form once.
                Some(_) if poll <= 2 => asking(),
                Some(answer) => completed(&answer.to_string()),
            },
            (Script::Fails, _) => json!({
                "status": "failed",
                "error": { "code": -32603, "message": "the build machine is gone" }
            }),
            (Script::CancelledByServer, _) => {
                json!({ "status": "cancelled", "statusMessage": "An operator stopped it" })
            }
        };
        let mut task = base;
        task.as_object_mut()
            .unwrap()
            .extend(state.as_object().unwrap().clone());
        task
    }
}

fn completed(text: &str) -> Value {
    json!({
        "status": "completed",
        "result": {
            "resultType": "complete",
            "content": [{ "type": "text", "text": text }],
            "isError": false
        }
    })
}

fn asking() -> Value {
    json!({
        "status": "input_required",
        "inputRequests": {
            "deploy": {
                "method": "elicitation/create",
                "params": {
                    "mode": "form",
                    "message": "Where should this release go?",
                    "requestedSchema": {
                        "type": "object",
                        "properties": {
                            "environment": { "type": "string", "enum": ["staging", "production"] }
                        },
                        "required": ["environment"]
                    }
                }
            }
        }
    })
}

fn rpc(id: &Value, result: Value) -> EgressResponse {
    EgressResponse {
        status: 200,
        headers: BTreeMap::new(),
        body: serde_json::to_vec(&json!({ "jsonrpc": "2.0", "id": id, "result": result })).unwrap(),
    }
}

#[async_trait]
impl EgressService for TaskServer {
    async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
        let mcp_name = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("mcp-name"))
            .map(|(_, value)| value.clone());
        self.requests.lock().unwrap().push(Request {
            body: body.clone(),
            mcp_name,
        });
        let id = &body["id"];
        let task_id = body["params"]["taskId"].as_str().unwrap_or_default();
        Ok(match body["method"].as_str().unwrap_or_default() {
            "tools/call" => {
                let task_id = format!("task-{}", self.requests("tools/call").len());
                rpc(
                    id,
                    json!({
                        "resultType": "task",
                        "taskId": task_id,
                        "status": "working",
                        "createdAt": "2026-10-23T00:00:00Z",
                        "lastUpdatedAt": "2026-10-23T00:00:00Z",
                        "ttlMs": 60_000,
                        "pollIntervalMs": 1,
                    }),
                )
            }
            "tasks/get" => rpc(id, self.task(task_id)),
            "tasks/update" => {
                if let Some(answer) = body["params"]["inputResponses"].get("deploy") {
                    self.answers
                        .lock()
                        .unwrap()
                        .insert(task_id.to_string(), answer.clone());
                }
                rpc(id, json!({ "resultType": "complete" }))
            }
            _ => rpc(id, json!({ "resultType": "complete" })),
        })
    }

    async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
        unimplemented!("not used by MCP transport")
    }
}

/// Answers recorded by the API that collected them.
#[derive(Default)]
struct Answers {
    records: Mutex<Vec<StoredFormAnswer>>,
}

#[async_trait]
impl FormAnswerStore for Answers {
    async fn take_form_answer(
        &self,
        server: &str,
        tool: &str,
    ) -> anyhow::Result<Option<StoredFormAnswer>> {
        let mut records = self.records.lock().unwrap();
        let found = records
            .iter()
            .position(|record| record.server == server && record.tool == tool);
        Ok(found.map(|index| records.remove(index)))
    }
}

fn transport(server: &Arc<TaskServer>) -> HttpTransport {
    HttpTransport::new(server.clone())
}

fn connection() -> McpConnection {
    McpConnection::http("ci", URL)
}

async fn call(transport: &HttpTransport, connection: &McpConnection) -> anyhow::Result<String> {
    let result = transport
        .call_tool(connection, "build", json!({ "ref": "main" }), None, None)
        .await?;
    match result.content.first() {
        Some(McpContent::Text { text, .. }) => Ok(text.clone()),
        other => panic!("unexpected content {other:?}"),
    }
}

/// Give a cancel spawned from a dropped guard the chance to go out.
async fn settle() {
    for _ in 0..50 {
        tokio::task::yield_now().await;
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn a_task_is_polled_until_it_completes() {
    let server = TaskServer::new(Script::Completes);
    let text = call(&transport(&server), &connection()).await.unwrap();
    assert_eq!(text, "built");

    // The client declared the extension on the call that created the task.
    let (tools_call, _) = server.requests("tools/call").remove(0);
    assert_eq!(
        tools_call["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]["extensions"],
        json!({ "io.modelcontextprotocol/tasks": {} })
    );
    // Each poll names the task twice: the param and the routing header. The
    // declaration rides along, since a server checks it on tasks/* too.
    let polls = server.requests("tasks/get");
    assert_eq!(polls.len(), 2);
    for (poll, mcp_name) in &polls {
        assert_eq!(poll["params"]["taskId"], "task-1");
        assert_eq!(mcp_name.as_deref(), Some("task-1"));
        assert_eq!(
            poll["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]["extensions"],
            json!({ "io.modelcontextprotocol/tasks": {} })
        );
    }
    // A finished task needs no cancel.
    settle().await;
    assert!(server.requests("tasks/cancel").is_empty());
}

#[tokio::test]
async fn a_failed_or_cancelled_task_is_an_error_with_the_servers_reason() {
    let server = TaskServer::new(Script::Fails);
    let error = call(&transport(&server), &connection())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("the build machine is gone"), "{error}");

    let server = TaskServer::new(Script::CancelledByServer);
    let error = call(&transport(&server), &connection())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("cancelled by server 'ci'"), "{error}");
    assert!(error.contains("An operator stopped it"), "{error}");
    settle().await;
    assert!(server.requests("tasks/cancel").is_empty());
}

#[tokio::test]
async fn a_task_past_the_wait_limit_is_cancelled() {
    let server = TaskServer::new(Script::RunsForever);
    let transport = transport(&server).with_task_wait_limit(Duration::from_millis(600));
    let error = call(&transport, &connection())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("still running"), "{error}");
    let cancels = server.requests("tasks/cancel");
    assert_eq!(cancels.len(), 1);
    assert_eq!(cancels[0].0["params"]["taskId"], "task-1");
    assert_eq!(cancels[0].1.as_deref(), Some("task-1"));
}

#[tokio::test]
async fn a_dropped_call_cancels_its_task() {
    let server = TaskServer::new(Script::RunsForever);
    let transport = transport(&server);
    let connection = connection();
    let waited =
        tokio::time::timeout(Duration::from_millis(600), call(&transport, &connection)).await;
    assert!(waited.is_err(), "the task never finishes");
    settle().await;
    assert_eq!(server.requests("tasks/cancel").len(), 1);
}

#[tokio::test]
async fn a_question_without_an_answer_stands_down_and_cancels_the_task() {
    let server = TaskServer::new(Script::AsksForm);
    let answers = Arc::new(Answers::default());
    let transport = transport(&server)
        .with_form_elicitation_handler(Arc::new(StoredFormAnswers::new(answers.clone())));
    let connection = connection().with_elicitation_policy(McpElicitationPolicy::UrlAndForm);

    // Nobody has answered: the call stands down so the turn can ask a person,
    // and the task it leaves behind is cancelled.
    let error = call(&transport, &connection).await.unwrap_err();
    let pending = error
        .downcast_ref::<FormElicitationPending>()
        .expect("the call stands down with questions")
        .clone();
    assert_eq!(pending.message, "Where should this release go?");
    assert!(server.requests("tasks/update").is_empty());
    assert_eq!(server.requests("tasks/cancel").len(), 1);

    // The person answers; the rerun's new task gets the answer, typed, once,
    // although the server shows the answered form again before it completes.
    answers.records.lock().unwrap().push(StoredFormAnswer::new(
        "ci",
        "build",
        &pending.fingerprint,
        FormAnswerAction::Accept,
        BTreeMap::from([(
            "environment".to_string(),
            FormAnswer {
                selected: vec!["production".to_string()],
                other_text: None,
            },
        )]),
        chrono::Utc::now(),
    ));
    let text = call(&transport, &connection).await.unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap(),
        json!({ "action": "accept", "content": { "environment": "production" } })
    );
    let updates = server.requests("tasks/update");
    assert_eq!(updates.len(), 1, "an answered key is not answered again");
    assert_eq!(updates[0].0["params"]["taskId"], "task-2");
    assert_eq!(updates[0].1.as_deref(), Some("task-2"));
    settle().await;
    assert_eq!(server.requests("tasks/cancel").len(), 1);
}

#[tokio::test]
async fn a_form_to_a_server_without_form_mode_fails_and_cancels() {
    // Default policy declares no form mode, so the server is in error asking.
    let server = TaskServer::new(Script::AsksForm);
    let error = call(&transport(&server), &connection())
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("did not declare form mode"), "{error}");
    assert_eq!(server.requests("tasks/cancel").len(), 1);
}
