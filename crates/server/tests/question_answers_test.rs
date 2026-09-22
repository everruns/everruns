//! Answering an `ask_user` question set (EVE-1054).
//!
//! `ask_user` is answered from four surfaces, so what matters here is that the
//! one shared operation holds the line for all of them: an answer is checked
//! against what was actually asked, a second answer is refused rather than
//! corrupting the turn, and typing in chat instead of clicking resolves the
//! question rather than leaving it pending forever.

mod test_harness;
use async_trait::async_trait;

use axum::http::StatusCode;
use everruns_platform::{Agent, Session};
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_worker::AgentRunner;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use test_harness::TestServer;
use uuid::Uuid;

const TEST_ORG_ID: i64 = 1;

struct RecordingRunner {
    resume_calls: AtomicUsize,
}

#[async_trait]
impl AgentRunner for RecordingRunner {
    async fn start_run(
        &self,
        _org_id: i64,
        _session_id: SessionId,
        _harness_id: HarnessId,
        _agent_id: Option<AgentId>,
        _input_message_id: MessageId,
        _request_id: Option<String>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn resume_after_tool_results(
        &self,
        _session_id: SessionId,
        _resolution_id: Uuid,
    ) -> anyhow::Result<()> {
        self.resume_calls.fetch_add(1, Ordering::SeqCst);
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

async fn test_server() -> TestServer {
    TestServer::in_memory_with_runner(Arc::new(RecordingRunner {
        resume_calls: AtomicUsize::new(0),
    }))
    .await
}
async fn waiting_session(server: &TestServer) -> SessionId {
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "ask-user-test-agent",
                "display_name": "Ask User Test",
                "description": "Agent for the question answers test",
                "system_prompt": "You are a helpful assistant"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session: Session = server
        .post("/v1/sessions", json!({ "agent_id": agent.public_id }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .db
        .update_session(
            TEST_ORG_ID,
            session.id,
            everruns_server::storage::models::UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update session status")
        .expect("session exists");

    session.id
}

/// Emit the card the engine emits when the model calls `ask_user`.
///
/// Every question carries an id here because `normalize_ask_user_arguments`
/// fills one in before the call is emitted — answers correlate by that id.
async fn emit_question_card(server: &TestServer, session_id: SessionId, tool_call_id: &str) {
    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({
                "tool_calls": [{
                    "id": tool_call_id,
                    "name": "ask_user",
                    "arguments": {
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
                    }
                }]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit tool.call_requested");
}

async fn emit_other_tool_card(server: &TestServer, session_id: SessionId, tool_call_id: &str) {
    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({
                "tool_calls": [{
                    "id": tool_call_id,
                    "name": "setup_connection",
                    "arguments": {}
                }]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit later tool.call_requested");
}

async fn post_answer(
    server: &TestServer,
    session_id: SessionId,
    body: Value,
) -> test_harness::TestResponse {
    server
        .post(&format!("/v1/sessions/{session_id}/question-answers"), body)
        .await
}

async fn completed_results(server: &TestServer, session_id: SessionId) -> Vec<Value> {
    server
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
        .expect("read events")
        .into_iter()
        .map(|event| event.data)
        .collect()
}

#[tokio::test]
async fn an_answer_resumes_the_turn_with_the_validated_result() {
    let server = test_server().await;
    let session_id = waiting_session(&server).await;
    emit_question_card(&server, session_id, "toolu_ask_1").await;

    let response = post_answer(
        &server,
        session_id,
        json!({
            "tool_call_id": "toolu_ask_1",
            "status": "answered",
            "answers": [{"id": "target", "selected": ["Staging"]}]
        }),
    )
    .await;
    response.assert_status(StatusCode::OK);

    let results = completed_results(&server, session_id).await;
    assert_eq!(results.len(), 1, "exactly one tool.completed: {results:?}");
    assert_eq!(
        results[0].get("tool_call_id").and_then(Value::as_str),
        Some("toolu_ask_1")
    );

    let session = server
        .db
        .get_session(TEST_ORG_ID, session_id)
        .await
        .expect("read session")
        .expect("session exists");
    assert_eq!(session.status, "active", "the turn is resumed");
}

/// THREAT[TM-AGENT-015]: the caller does not get to say what it was asked.
#[tokio::test]
async fn a_label_that_was_never_offered_is_refused() {
    let server = test_server().await;
    let session_id = waiting_session(&server).await;
    emit_question_card(&server, session_id, "toolu_ask_1").await;

    post_answer(
        &server,
        session_id,
        json!({
            "tool_call_id": "toolu_ask_1",
            "status": "answered",
            "answers": [{"id": "target", "selected": ["Production; rm -rf /"]}]
        }),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);

    assert!(
        completed_results(&server, session_id).await.is_empty(),
        "a refused answer must not complete the call"
    );
}

#[tokio::test]
async fn a_second_answer_is_a_conflict_not_a_second_result() {
    let server = test_server().await;
    let session_id = waiting_session(&server).await;
    emit_question_card(&server, session_id, "toolu_ask_1").await;

    let body = json!({
        "tool_call_id": "toolu_ask_1",
        "status": "answered",
        "answers": [{"id": "target", "selected": ["Staging"]}]
    });
    post_answer(&server, session_id, body.clone())
        .await
        .assert_status(StatusCode::OK);

    // Two clicks, or a browser racing the deadline sweep. First writer wins.
    post_answer(&server, session_id, body)
        .await
        .assert_status(StatusCode::CONFLICT);

    let results = completed_results(&server, session_id).await;
    assert_eq!(
        results.len(),
        1,
        "a second tool.completed would be a malformed transcript: {results:?}"
    );
}

#[tokio::test]
async fn answering_a_session_that_is_not_parked_is_a_conflict() {
    let server = test_server().await;
    let session_id = waiting_session(&server).await;
    emit_question_card(&server, session_id, "toolu_ask_1").await;
    server
        .db
        .update_session(
            TEST_ORG_ID,
            session_id,
            everruns_server::storage::models::UpdateSession {
                status: Some("active".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update session status")
        .expect("session exists");

    post_answer(
        &server,
        session_id,
        json!({
            "tool_call_id": "toolu_ask_1",
            "status": "answered",
            "answers": [{"id": "target", "selected": ["Staging"]}]
        }),
    )
    .await
    .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn a_stale_question_card_cannot_resolve_a_later_tool_pause() {
    for explicit_id in [true, false] {
        let runner = Arc::new(RecordingRunner {
            resume_calls: AtomicUsize::new(0),
        });
        let server = TestServer::in_memory_with_runner(runner.clone()).await;
        let session_id = waiting_session(&server).await;
        let old_call_id = format!("toolu_old_ask_{explicit_id}");
        emit_question_card(&server, session_id, &old_call_id).await;

        post_answer(
            &server,
            session_id,
            json!({
                "tool_call_id": old_call_id,
                "status": "answered",
                "answers": [{"id": "target", "selected": ["Staging"]}]
            }),
        )
        .await
        .assert_status(StatusCode::OK);

        server
            .db
            .update_session(
                TEST_ORG_ID,
                session_id,
                everruns_server::storage::models::UpdateSession {
                    status: Some("waiting_for_tool_results".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("park session on later tool call")
            .expect("session exists");
        emit_other_tool_card(
            &server,
            session_id,
            &format!("toolu_setup_connection_{explicit_id}"),
        )
        .await;

        let mut stale_answer = json!({
            "status": "answered",
            "answers": [{"id": "target", "selected": ["Production"]}]
        });
        if explicit_id {
            stale_answer["tool_call_id"] = Value::String(old_call_id);
        }
        post_answer(&server, session_id, stale_answer)
            .await
            .assert_status(StatusCode::CONFLICT);

        let results = completed_results(&server, session_id).await;
        assert_eq!(
            results.len(),
            1,
            "the old answer must not be completed twice"
        );
        assert_eq!(
            runner.resume_calls.load(Ordering::SeqCst),
            1,
            "the stale card must not resume the later pause"
        );
        assert_eq!(
            server
                .db
                .get_session(TEST_ORG_ID, session_id)
                .await
                .expect("read session")
                .expect("session exists")
                .status,
            "waiting_for_tool_results"
        );
        assert!(
            server
                .db
                .list_events(
                    session_id,
                    None,
                    None,
                    &["input.message".to_string()],
                    &[],
                    None,
                    None,
                )
                .await
                .expect("read input events")
                .is_empty(),
            "rejecting a stale card must not inject user input"
        );
    }
}

/// Typing an answer rather than clicking one. Before EVE-1054 the message was
/// appended and the call stayed pending forever.
#[tokio::test]
async fn a_chat_message_cancels_the_question_and_is_still_delivered() {
    let server = test_server().await;
    let session_id = waiting_session(&server).await;
    emit_question_card(&server, session_id, "toolu_ask_1").await;

    server
        .post(
            &format!("/v1/sessions/{session_id}/messages"),
            json!({ "message": { "role": "user", "content": [{"type": "text", "text": "just use staging"}] } }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let results = completed_results(&server, session_id).await;
    assert_eq!(results.len(), 1, "the question is resolved: {results:?}");
    let result: Value = serde_json::from_str(
        results[0]
            .get("result")
            .and_then(Value::as_array)
            .and_then(|parts| parts.first())
            .and_then(|part| part.get("text"))
            .and_then(Value::as_str)
            .expect("the tool result carries the outcome"),
    )
    .expect("the outcome parses");
    // Cancelled, not answered: they said something, but not in terms of the
    // options, and nothing may guess which one they meant.
    assert_eq!(
        result.get("status").and_then(Value::as_str),
        Some("cancelled")
    );
    // Nobody chose, so nobody is credited with choosing.
    assert_eq!(
        result.get("answered_by").and_then(Value::as_str),
        Some("unattended")
    );
    assert_eq!(
        result
            .get("answers")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(0),
        "a cancellation must not invent a selection: {result}"
    );

    let messages: Value = server
        .get(&format!("/v1/sessions/{session_id}/messages"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let rendered = messages.to_string();
    assert!(
        rendered.contains("just use staging"),
        "the message is delivered normally: {rendered}"
    );
}
