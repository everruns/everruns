//! `ask_user` over the inbound A2A channel (EVE-1062).
//!
//! A parked question has to survive the round trip twice over: a caller that
//! reads only text still learns what was asked, a caller that reads data gets
//! the same question typed and schema-declared, an answer submitted as a
//! `DataPart` resumes the parked turn, and a credential is never projected as
//! something a remote agent could fill in.
//!
//! Sibling of `app_a2a_integration_test.rs`, which covers the rest of the
//! channel. The setup helpers are duplicated rather than shared because each
//! integration test file is its own crate.

use crate::test_harness;

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use everruns_core::DEFAULT_ORG_ID;
use everruns_provider::typed_id::SessionId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use test_harness::TestServer;

async fn create_app_with_a2a(server: &TestServer, name: &str, message: &str) -> (Value, String) {
    create_app_with_a2a_mode(server, name, message, "shared_session").await
}

async fn create_app_with_a2a_mode(
    server: &TestServer,
    name: &str,
    message: &str,
    session_mode: &str,
) -> (Value, String) {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("{name}-agent"),
                "display_name": format!("{name} agent"),
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let api_key = format!("evr_app_{}", uuid::Uuid::new_v4().simple());
    let api_key_hash = hex::encode(Sha256::digest(api_key.as_bytes()));
    let app = server
        .seed_app_endpoint(
            name,
            agent["id"].as_str().unwrap(),
            "a2a",
            json!({
                "session_mode": session_mode,
                "message": message,
                "agent_card_name": "Inbox triage",
                "agent_card_description": "Triages inbound A2A traffic",
                "api_key_hash": api_key_hash,
                "api_key_prefix": &api_key[..12],
            }),
        )
        .await;
    (app, api_key)
}

async fn publish_app(server: &TestServer, app_id: &str) {
    server.set_app_endpoints_live(app_id, true).await;
}

async fn list_user_message_texts(server: &TestServer, session_id: &str) -> Vec<String> {
    let body: Value = server
        .get(&format!("/v1/sessions/{session_id}/messages"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    body["data"]
        .as_array()
        .expect("messages array")
        .iter()
        .filter(|m| m["role"].as_str() == Some("user"))
        .filter_map(|m| {
            m["content"].as_array().and_then(|parts| {
                parts.iter().find_map(|p| {
                    (p["type"].as_str() == Some("text"))
                        .then(|| p["text"].as_str().map(str::to_owned))
                        .flatten()
                })
            })
        })
        .collect()
}

/// Send a JSON-RPC request to an A2A channel and return the parsed envelope.
async fn a2a_rpc(
    server: &TestServer,
    app_id: &str,
    channel_id: &str,
    api_key: &str,
    method: &str,
    params: Value,
) -> Value {
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "rpc-1",
        "method": method,
        "params": params,
    }))
    .unwrap();
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json()
}

/// A runner that records resumes instead of executing a turn. The real
/// in-memory runner would try to run the model; what matters here is that the
/// parked turn was handed back to it.
struct ResumeRecordingRunner {
    resumes: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl everruns_worker::AgentRunner for ResumeRecordingRunner {
    async fn start_run(
        &self,
        _org_id: i64,
        _session_id: SessionId,
        _harness_id: everruns_provider::typed_id::HarnessId,
        _agent_id: Option<everruns_provider::typed_id::AgentId>,
        _input_message_id: everruns_provider::typed_id::MessageId,
        _request_id: Option<String>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn resume_after_tool_results(
        &self,
        _session_id: SessionId,
        _resolution_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        self.resumes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
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

/// Stand a task up and park it on `ask_user`, the way the act atom does:
/// the card is a `tool.call_requested` event and the session waits.
async fn parked_a2a_task(
    server: &TestServer,
    name: &str,
    card_arguments: Value,
) -> (String, String, String, String) {
    let (app, api_key) = create_app_with_a2a(server, name, "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap().to_string();
    let channel_id = app["channels"][0]["id"].as_str().unwrap().to_string();
    publish_app(server, &app_id).await;

    let send = a2a_rpc(
        server,
        &app_id,
        &channel_id,
        &api_key,
        "message/send",
        json!({ "message": { "role": "user", "parts": [{ "kind": "text", "text": "deploy it" }] } }),
    )
    .await;
    let task_id = send["result"]["id"].as_str().unwrap().to_string();

    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id: task_id.parse::<SessionId>().unwrap(),
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({
                "tool_calls": [{
                    "id": "call_ask_1",
                    "name": "ask_user",
                    "arguments": card_arguments,
                }]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit tool.call_requested");
    server
        .db
        .update_session(
            DEFAULT_ORG_ID,
            task_id.parse::<SessionId>().unwrap(),
            everruns_server::storage::models::UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("park the session")
        .expect("session exists");

    (app_id, channel_id, api_key, task_id)
}

fn choice_card_arguments() -> Value {
    json!({
        "questions": [{
            "kind": "choice",
            "id": "target",
            "header": "Target",
            "question": "Which environment should I deploy to?",
            "multi_select": false,
            "allow_other": true,
            "options": [
                {"label": "Staging", "description": "Safe, reversible.", "default": true},
                {"label": "Production", "description": "Live traffic."}
            ]
        }],
        "timeout_seconds": 300
    })
}

fn secret_card_arguments() -> Value {
    json!({
        "questions": [{
            "kind": "secret",
            "id": "stripe_key",
            "header": "Stripe key",
            "question": "Which Stripe restricted key should I use?",
            "multi_select": false,
            "allow_other": false,
            "options": [],
            "secret_name": "STRIPE_API_KEY",
            "purpose": "Read-only charge lookups."
        }],
        "timeout_seconds": 300
    })
}

async fn tool_completed_results(server: &TestServer, task_id: &str) -> Vec<Value> {
    server
        .db
        .list_events(
            task_id.parse::<SessionId>().unwrap(),
            None,
            None,
            &["tool.completed".to_string()],
            &[],
            None,
            Some(50),
        )
        .await
        .expect("list tool.completed")
        .into_iter()
        .map(|event| event.data)
        .collect()
}

/// A parked question reaches an A2A caller twice over: as prose any existing
/// text-only consumer already reads, and as a schema-declared DataPart.
#[tokio::test]
async fn a2a_tasks_get_projects_parked_ask_user_as_prose_and_data_part() {
    let server = TestServer::in_memory_with_runner(Arc::new(ResumeRecordingRunner {
        resumes: std::sync::atomic::AtomicUsize::new(0),
    }))
    .await;
    let (app_id, channel_id, api_key, task_id) =
        parked_a2a_task(&server, "a2a-ask-user-get", choice_card_arguments()).await;

    let got = a2a_rpc(
        &server,
        &app_id,
        &channel_id,
        &api_key,
        "tasks/get",
        json!({ "id": task_id }),
    )
    .await;

    assert_eq!(got["result"]["status"]["state"], "input_required");
    let parts = got["result"]["status"]["message"]["parts"]
        .as_array()
        .expect("status message carries parts");

    let text = parts
        .iter()
        .find(|part| part["kind"] == "text")
        .and_then(|part| part["text"].as_str())
        .expect("a text part renders the question in prose");
    assert!(
        text.contains("Which environment should I deploy to?"),
        "{text}"
    );
    assert!(text.contains("Staging"), "{text}");
    assert!(text.contains("Production"), "{text}");

    let data = parts
        .iter()
        .find(|part| part["kind"] == "data")
        .map(|part| &part["data"]["everruns/ask_user"])
        .expect("a data part carries the typed question set");
    assert_eq!(data["tool_call_id"], "call_ask_1");
    assert_eq!(data["questions"][0]["id"], "target");
    assert_eq!(data["questions"][0]["options"][0]["label"], "Staging");
    // The schema is what stands in for the elicitation primitive A2A lacks:
    // the caller is told the exact answer object, including which labels it
    // may select.
    assert_eq!(
        data["answer_schema"]["properties"]["answers"]["items"]["oneOf"][0]["properties"]["selected"]
            ["items"]["enum"],
        json!(["Staging", "Production"])
    );
}

/// The whole round trip: a DataPart answer resolves the question set and hands
/// the parked turn back to the runner.
#[tokio::test]
async fn a2a_message_send_data_part_answer_resumes_the_parked_turn() {
    let runner = Arc::new(ResumeRecordingRunner {
        resumes: std::sync::atomic::AtomicUsize::new(0),
    });
    let server = TestServer::in_memory_with_runner(runner.clone()).await;
    let (app_id, channel_id, api_key, task_id) =
        parked_a2a_task(&server, "a2a-ask-user-answer", choice_card_arguments()).await;

    let answered = a2a_rpc(
        &server,
        &app_id,
        &channel_id,
        &api_key,
        "message/send",
        json!({
            "message": {
                "role": "user",
                "taskId": task_id,
                "parts": [{
                    "kind": "data",
                    "data": {
                        "everruns/ask_user_answer": {
                            "tool_call_id": "call_ask_1",
                            "status": "answered",
                            "answers": [{ "id": "target", "selected": ["Production"] }]
                        }
                    }
                }]
            }
        }),
    )
    .await;

    assert!(
        answered.get("error").is_none(),
        "answer was rejected: {answered}"
    );
    assert_eq!(answered["result"]["id"], task_id);

    let completions = tool_completed_results(&server, &task_id).await;
    let answer = completions
        .iter()
        .find(|data| data["tool_call_id"] == "call_ask_1")
        .expect("the ask_user call was completed");
    let payload = answer["result"].to_string();
    assert!(payload.contains("Production"), "{payload}");
    assert!(payload.contains("answered"), "{payload}");
    assert_eq!(
        runner.resumes.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the parked turn was handed back to the runner"
    );
}

/// An answer that names an option nobody offered is refused by the shared
/// resolution operation, not accepted because it arrived over A2A.
#[tokio::test]
async fn a2a_data_part_answer_cannot_invent_an_option() {
    let runner = Arc::new(ResumeRecordingRunner {
        resumes: std::sync::atomic::AtomicUsize::new(0),
    });
    let server = TestServer::in_memory_with_runner(runner.clone()).await;
    let (app_id, channel_id, api_key, task_id) =
        parked_a2a_task(&server, "a2a-ask-user-invalid", choice_card_arguments()).await;

    let rejected = a2a_rpc(
        &server,
        &app_id,
        &channel_id,
        &api_key,
        "message/send",
        json!({
            "message": {
                "role": "user",
                "taskId": task_id,
                "parts": [{
                    "kind": "data",
                    "data": {
                        "everruns/ask_user_answer": {
                            "answers": [{ "id": "target", "selected": ["Everywhere"] }]
                        }
                    }
                }]
            }
        }),
    )
    .await;

    assert_eq!(rejected["error"]["code"], -32602);
    assert_eq!(
        runner.resumes.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "a refused answer never resumes the turn"
    );
}

/// An answer must name the task that asked. Without a task id there is nothing
/// binding it to a question set.
#[tokio::test]
async fn a2a_data_part_answer_without_a_task_id_is_refused() {
    let server = TestServer::in_memory_with_runner(Arc::new(ResumeRecordingRunner {
        resumes: std::sync::atomic::AtomicUsize::new(0),
    }))
    .await;
    let (app_id, channel_id, api_key, _task_id) =
        parked_a2a_task(&server, "a2a-ask-user-no-task", choice_card_arguments()).await;

    let rejected = a2a_rpc(
        &server,
        &app_id,
        &channel_id,
        &api_key,
        "message/send",
        json!({
            "message": {
                "role": "user",
                "parts": [{
                    "kind": "data",
                    "data": {
                        "everruns/ask_user_answer": {
                            "answers": [{ "id": "target", "selected": ["Staging"] }]
                        }
                    }
                }]
            }
        }),
    )
    .await;

    assert_eq!(rejected["error"]["code"], -32602);
}

/// THREAT[TM-AGENT-016]: a remote agent is never prompted for a human's
/// credential. The secret question becomes `auth_required` plus a URL, and
/// nothing in the projection is answerable.
#[tokio::test]
async fn a2a_secret_question_projects_as_auth_required_never_as_a_question_to_answer() {
    let server = TestServer::in_memory_with_runner(Arc::new(ResumeRecordingRunner {
        resumes: std::sync::atomic::AtomicUsize::new(0),
    }))
    .await;
    let (app_id, channel_id, api_key, task_id) =
        parked_a2a_task(&server, "a2a-ask-user-secret", secret_card_arguments()).await;

    let got = a2a_rpc(
        &server,
        &app_id,
        &channel_id,
        &api_key,
        "tasks/get",
        json!({ "id": task_id }),
    )
    .await;

    assert_eq!(got["result"]["status"]["state"], "auth_required");
    let message = got["result"]["status"]["message"].to_string();
    assert!(
        !message.contains("everruns/ask_user\""),
        "a secret question must never be projected as an answerable question set: {message}"
    );
    let parts = got["result"]["status"]["message"]["parts"]
        .as_array()
        .unwrap();
    let auth = parts
        .iter()
        .find(|part| part["kind"] == "data")
        .map(|part| &part["data"]["everruns/auth_required"])
        .expect("a data part names where a human completes it");
    assert_eq!(auth["reason"], "secret_question");
    assert!(
        auth["url"]
            .as_str()
            .unwrap_or_default()
            .contains(&format!("/sessions/{task_id}/chat")),
        "{auth}"
    );
    let text = parts
        .iter()
        .find(|part| part["kind"] == "text")
        .and_then(|part| part["text"].as_str())
        .expect("prose explains the credential is not carried over A2A");
    assert!(text.contains("STRIPE_API_KEY"), "{text}");

    // And an answer is refused at the channel boundary, not merely downstream.
    let refused = a2a_rpc(
        &server,
        &app_id,
        &channel_id,
        &api_key,
        "message/send",
        json!({
            "message": {
                "role": "user",
                "taskId": task_id,
                "parts": [{
                    "kind": "data",
                    "data": {
                        "everruns/ask_user_answer": {
                            "answers": [{ "id": "stripe_key", "secret_ref": "session:STRIPE_API_KEY" }]
                        }
                    }
                }]
            }
        }),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32602);
    assert!(
        tool_completed_results(&server, &task_id).await.is_empty(),
        "nothing completed the secret question"
    );
}

/// The pre-existing meaning of a text-only reply is untouched: it is delivered
/// as a message, and the question it superseded resolves as `cancelled`.
#[tokio::test]
async fn a2a_text_only_reply_to_a_parked_question_still_supersedes_it() {
    let runner = Arc::new(ResumeRecordingRunner {
        resumes: std::sync::atomic::AtomicUsize::new(0),
    });
    let server = TestServer::in_memory_with_runner(runner).await;
    let (app_id, channel_id, api_key, task_id) =
        parked_a2a_task(&server, "a2a-ask-user-text", choice_card_arguments()).await;

    let sent = a2a_rpc(
        &server,
        &app_id,
        &channel_id,
        &api_key,
        "message/send",
        json!({
            "message": {
                "role": "user",
                "taskId": task_id,
                "parts": [{ "kind": "text", "text": "never mind, do something else" }]
            }
        }),
    )
    .await;
    assert!(sent.get("error").is_none(), "{sent}");

    let completions = tool_completed_results(&server, &task_id).await;
    let cancelled = completions
        .iter()
        .find(|data| data["tool_call_id"] == "call_ask_1")
        .expect("the superseded question was resolved");
    assert!(
        cancelled["result"].to_string().contains("cancelled"),
        "{cancelled}"
    );
    assert!(
        list_user_message_texts(&server, &task_id)
            .await
            .iter()
            .any(|text| text.contains("never mind")),
        "the text reply is still delivered as a message"
    );
}
