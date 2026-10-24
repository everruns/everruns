//! The official MCP TypeScript client against `/mcp` Tasks (2026-07-28).
//!
//! There is no official Tasks conformance suite for servers, so the closest
//! outside check is the reference client: `@modelcontextprotocol/client`
//! negotiates the protocol and `@modelcontextprotocol/ext-tasks` runs each tool
//! call as a task, validating every response against the published schema.
//! `scripts/mcp-tasks-interop/driver.ts` runs the scenarios; this test serves
//! `/mcp` on a real socket with a stand-in runner, so no model or worker is
//! needed. Runs only when `MCP_TASKS_INTEROP_DIR` names an installed driver
//! directory: `scripts/mcp-tasks-interop.sh`.
//!
//! The second test points Everruns' own MCP client at the same server, so the
//! client half is checked against a server the reference client accepts.

use crate::test_harness;

use async_trait::async_trait;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{EventContext, EventRequest, OutputMessageCompletedData};
use everruns_core::host::{TurnBackend, TurnInput, TurnRequest, TurnTicket};
use everruns_core::message::RuntimeMessage;
use everruns_server::services::EventService;
use everruns_server::storage::StorageBackend;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use test_harness::TestServer;

/// Turns 1 and 2 answer "Reply N". Turn 3 parks on an `ask_user` question set,
/// as the engine does, and its resumption answers "Answered".
#[derive(Default)]
struct ScriptedRunner {
    wiring: OnceLock<(Arc<EventService>, Arc<StorageBackend>)>,
    turns: AtomicUsize,
    sessions: std::sync::Mutex<Vec<SessionId>>,
}

impl ScriptedRunner {
    fn reply(
        &self,
        session_id: SessionId,
        turn_id: everruns_contracts::typed_id::TurnId,
        text: String,
    ) {
        let (events, _) = self.wiring.get().expect("runner wired").clone();
        tokio::spawn(async move {
            let output = OutputMessageCompletedData::new(RuntimeMessage::assistant(text));
            events
                .emit(EventRequest::new(session_id, EventContext::empty(), output))
                .await
                .unwrap();
            let completed: everruns_core::events::TurnCompletedData =
                serde_json::from_value(json!({ "turn_id": turn_id, "iterations": 1 })).unwrap();
            events
                .emit(EventRequest::new(
                    session_id,
                    EventContext::empty(),
                    completed,
                ))
                .await
                .unwrap();
        });
    }

    fn ask(&self, session_id: SessionId) {
        let (_, db) = self.wiring.get().expect("runner wired").clone();
        tokio::spawn(async move {
            db.create_event(everruns_server::storage::CreateEventRow {
                session_id,
                event_type: "tool.call_requested".to_string(),
                ts: chrono::Utc::now(),
                context: json!({}),
                data: json!({
                    "tool_calls": [{
                        "id": "call_ask",
                        "name": "ask_user",
                        "arguments": { "questions": questions(), "timeout_seconds": 300 }
                    }]
                }),
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
            db.update_session(
                everruns_core::DEFAULT_ORG_ID,
                session_id,
                everruns_server::storage::UpdateSession {
                    status: Some("waiting_for_tool_results".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        });
    }
}

#[async_trait]
impl TurnBackend for ScriptedRunner {
    async fn start_turn(
        &self,
        request: TurnRequest,
    ) -> everruns_contracts::error::Result<TurnTicket> {
        self.sessions.lock().unwrap().push(request.session_id);
        if matches!(request.input, TurnInput::RecordedToolResults { .. }) {
            self.reply(request.session_id, request.turn_id, "Answered".to_string());
        } else {
            let n = self.turns.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 3 {
                self.ask(request.session_id);
            } else {
                self.reply(request.session_id, request.turn_id, format!("Reply {n}"));
            }
        }
        Ok(TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(&self, _session_id: SessionId) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

fn questions() -> Value {
    json!([
        {
            "kind": "choice",
            "id": "target",
            "header": "Target",
            "question": "Which environment should I deploy to?",
            "multi_select": false,
            "allow_other": false,
            "options": [
                {"label": "Staging", "description": "Safe, reversible.", "default": true},
                {"label": "Production", "description": "Live traffic."}
            ]
        },
        {
            "kind": "choice",
            "id": "areas",
            "header": "Areas",
            "question": "Which areas should I touch?",
            "multi_select": true,
            "allow_other": false,
            "options": [
                {"label": "API", "description": "Server routes."},
                {"label": "UI", "description": "The web app."}
            ]
        }
    ])
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn official_ts_client_drives_mcp_tasks() {
    use tokio::io::{AsyncBufReadExt, BufReader};

    let Ok(dir) = std::env::var("MCP_TASKS_INTEROP_DIR") else {
        return;
    };
    let runner = Arc::new(ScriptedRunner::default());
    let (server, base_url) = TestServer::serving_in_memory_with_runner(runner.clone()).await;
    runner
        .wiring
        .set((server.event_service.clone(), server.db.clone()))
        .ok();

    let mut child = tokio::process::Command::new("node")
        .args(["--experimental-strip-types", "--no-warnings", "driver.ts"])
        .current_dir(&dir)
        .env("MCP_SERVER", &base_url)
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("run node");
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let mut passed = Vec::new();
    let mut session_id = None;
    let mut ok = false;
    while let Some(line) = lines.next_line().await.unwrap() {
        println!("{line}");
        if let Some(id) = line.strip_prefix("SESSION ") {
            session_id = Some(id.parse::<SessionId>().unwrap());
        }
        if let Some(check) = line.strip_prefix("PASS ") {
            passed.push(check.to_string());
        }
        ok |= line == "OK";
    }
    let status = child.wait().await.unwrap();
    assert!(status.success() && ok, "driver failed after {passed:?}");
    assert_eq!(passed.len(), 6, "{passed:?}");

    // The client's form answer went through the shared resolution operation
    // and reached the parked turn as the ask_user result.
    let results = server
        .db
        .list_events(
            session_id.expect("driver printed the session"),
            None,
            None,
            &["tool.completed".to_string()],
            &[],
            None,
            Some(10),
        )
        .await
        .unwrap();
    assert_eq!(results.len(), 1, "{results:?}");
    let outcome: Value =
        serde_json::from_str(results[0].data["result"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(outcome["status"], "answered", "{outcome}");
    assert_eq!(outcome["answers"][0]["selected"], json!(["Production"]));
}

/// Hands the client's requests, already past its SSRF check against a public
/// placeholder address, to the test server on loopback.
struct LoopbackEgress {
    base_url: String,
    http: reqwest::Client,
}

#[async_trait]
impl everruns_core::EgressService for LoopbackEgress {
    async fn send(
        &self,
        request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
        let path = request.url.trim_start_matches("http://8.8.8.8");
        let mut outbound = self.http.post(format!("{}{path}", self.base_url));
        for (name, value) in &request.headers {
            outbound = outbound.header(name, value);
        }
        let response = outbound.body(request.body).send().await.unwrap();
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_str().unwrap_or("").to_string()))
            .collect();
        let body = response.bytes().await.unwrap().to_vec();
        Ok(everruns_core::EgressResponse {
            status,
            headers,
            body,
        })
    }

    async fn send_stream(
        &self,
        request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        let response = self.send(request).await?;
        Ok(everruns_core::EgressStreamResponse {
            status: response.status,
            headers: response.headers,
            body: Box::pin(futures::stream::iter([Ok(response.body)])),
        })
    }
}

/// Answers the deploy question the way a person would.
struct PicksProduction;

#[async_trait]
impl everruns_core::mcp::FormElicitationHandler for PicksProduction {
    async fn answer_form(
        &self,
        elicitation: &everruns_core::mcp::FormElicitation,
    ) -> anyhow::Result<everruns_core::mcp::FormOutcome> {
        assert!(
            elicitation
                .schema
                .properties
                .iter()
                .any(|property| property.name == "target"),
            "{elicitation:?}"
        );
        let content = json!({ "target": "Production", "areas__0": true, "areas__1": false });
        Ok(everruns_core::mcp::FormOutcome::Accept(
            content.as_object().unwrap().clone(),
        ))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn everruns_client_drives_everruns_tasks() {
    use everruns_core::mcp::{HttpTransport, McpConnection, McpTransport};

    let runner = Arc::new(ScriptedRunner::default());
    let (server, base_url) = TestServer::serving_in_memory_with_runner(runner.clone()).await;
    runner
        .wiring
        .set((server.event_service.clone(), server.db.clone()))
        .ok();
    let transport = HttpTransport::new(Arc::new(LoopbackEgress {
        base_url,
        http: reqwest::Client::new(),
    }))
    .with_form_elicitation_handler(Arc::new(PicksProduction));
    let connection = McpConnection::http("everruns", "http://8.8.8.8/mcp")
        .with_elicitation_policy(everruns_core::McpElicitationPolicy::UrlAndForm);
    let text = |result: everruns_core::McpToolCallResult| match result.content.first() {
        Some(everruns_core::McpContent::Text { text, .. }) => text.clone(),
        other => panic!("unexpected content {other:?}"),
    };

    let first = transport
        .call_tool(
            &connection,
            "agent_run",
            json!({ "message": "one" }),
            None,
            None,
        )
        .await
        .expect("agent_run settles as a task");
    assert_eq!(text(first), "Reply 1");
    let session_id = runner.sessions.lock().unwrap()[0].to_string();

    let second = transport
        .call_tool(
            &connection,
            "session_send_message",
            json!({ "session_id": session_id, "message": "two" }),
            None,
            None,
        )
        .await
        .expect("the next message is its own task");
    assert_eq!(text(second), "Reply 2");

    // The third turn asks; the client answers the form through tasks/update and
    // the resumed turn's reply is the tool result.
    let third = transport
        .call_tool(
            &connection,
            "session_send_message",
            json!({ "session_id": session_id, "message": "three" }),
            None,
            None,
        )
        .await
        .expect("the question is answered and the turn finishes");
    assert_eq!(text(third), "Answered");
}
