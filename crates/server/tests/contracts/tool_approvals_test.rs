//! Hard tool approval for hosted sessions (EVE-1140,
//! knowledge/execution/tool-approval.md).
//!
//! The `tool_approval` gate defers a risky call it has no decision for, the
//! turn parks on a synthetic `approve_tool_call` call, and a person answers it
//! through `POST /v1/sessions/{id}/tool-approvals`. These tests drive the real
//! gate (`DurableToolApprover`) against the server's own session storage, so a
//! "fresh gate" here is exactly what a newly started worker builds: nothing in
//! memory, everything read from the database.

use crate::test_harness;

use async_trait::async_trait;
use axum::http::StatusCode;
use everruns_builtins::{DurableToolApprover, ToolApprovalCapability};
use everruns_core::capabilities::Capability;
use everruns_core::session_services::SessionStorageStore;
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_hooks::PreToolUseDecision;
use everruns_core::{Caller, Permission, PermissionResolver};
use everruns_platform::{Agent, Session};
use everruns_provider::tool_types::{
    BuiltinTool, ToolApprovalRequired, ToolCall, ToolDefinition, ToolHints,
};
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_server::storage::StorageBackend;
use everruns_worker::AgentRunner;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use test_harness::TestServer;
use uuid::Uuid;

const TEST_ORG_ID: i64 = 1;

/// Counts durable resumes, which is how a test sees the turn restart.
struct RecordingRunner {
    resume_calls: Arc<AtomicUsize>,
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

/// Withholds `org:sessions:manage` once armed (after the fixture is built,
/// since creating the session needs the same permission).
#[derive(Default)]
struct DenySessionManagement {
    armed: AtomicBool,
}

impl PermissionResolver for DenySessionManagement {
    fn has_permission(&self, _caller: &Caller, permission: &Permission) -> bool {
        !(self.armed.load(Ordering::SeqCst) && permission == &Permission::OrgSessionsManage)
    }

    fn caller_permissions(&self, caller: &Caller) -> Vec<Permission> {
        Permission::ALL
            .iter()
            .copied()
            .filter(|permission| self.has_permission(caller, permission))
            .collect()
    }
}

struct Fixture {
    server: TestServer,
    resumes: Arc<AtomicUsize>,
}

async fn fixture() -> Fixture {
    let resumes = Arc::new(AtomicUsize::new(0));
    let server = TestServer::in_memory_with_runner(Arc::new(RecordingRunner {
        resume_calls: resumes.clone(),
    }))
    .await;
    Fixture { server, resumes }
}

async fn parked_session(server: &TestServer) -> SessionId {
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("approval-test-agent-{}", Uuid::now_v7().simple()),
                "display_name": "Approval Test",
                "description": "Agent for the hosted tool approval test",
                "system_prompt": "You are a helpful assistant",
                "capabilities": [{ "ref": "tool_approval", "config": { "mode": "normal" } }],
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
    set_waiting(server, session.id).await;
    session.id
}

async fn set_waiting(server: &TestServer, session_id: SessionId) {
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
        .expect("update session status")
        .expect("session exists");
}

/// The server's session storage, as the worker reaches it.
fn storage(server: &TestServer) -> Arc<dyn SessionStorageStore> {
    match server.db.as_ref() {
        StorageBackend::InMemory(memory) => memory.clone(),
        StorageBackend::Postgres(db) => Arc::new(
            everruns_server::storage::DbSessionStorageStore::new_without_encryption(db.clone()),
        ),
    }
}

fn open_world_tool() -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: "send_email".to_string(),
        display_name: Some("Send email".to_string()),
        description: "Send an email".to_string(),
        parameters: json!({ "type": "object" }),
        policy: Default::default(),
        category: None,
        deferrable: Default::default(),
        hints: ToolHints {
            open_world: Some(true),
            ..Default::default()
        },
        full_parameters: None,
    })
}

fn send_email(id: &str, to: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "send_email".to_string(),
        arguments: json!({ "to": to, "subject": "Quarterly numbers" }),
    }
}

/// Run a call through a freshly built gate, as a newly started worker would.
async fn gate(server: &TestServer, session_id: SessionId, call: ToolCall) -> PreToolUseDecision {
    let capability = ToolApprovalCapability::new(Arc::new(DurableToolApprover));
    let hooks = capability.pre_tool_use_hooks_with_config(&json!({ "mode": "normal" }));
    let context = ToolContext::new(session_id).with_storage_store_arc(storage(server));
    hooks[0]
        .before_exec(call, &open_world_tool(), &context)
        .await
}

/// The gate defers the call; emit the request exactly as the engine does.
async fn park_on(server: &TestServer, session_id: SessionId, calls: &[ToolCall]) -> Vec<String> {
    let mut requests = Vec::new();
    for call in calls {
        let PreToolUseDecision::Defer { result, .. } = gate(server, session_id, call.clone()).await
        else {
            panic!("an open_world call with no decision must be deferred");
        };
        let request = ToolApprovalRequired::from_tool_result(&result).expect("payload");
        requests.push(request.request_call());
    }
    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({ "tool_calls": requests }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit tool.call_requested");
    requests.into_iter().map(|call| call.id).collect()
}

async fn post_decisions(
    server: &TestServer,
    session_id: SessionId,
    decisions: Value,
) -> test_harness::TestResponse {
    server
        .post(
            &format!("/v1/sessions/{session_id}/tool-approvals"),
            json!({ "decisions": decisions }),
        )
        .await
}

async fn events_of(server: &TestServer, session_id: SessionId, kind: &str) -> Vec<Value> {
    server
        .db
        .list_events(
            session_id,
            None,
            None,
            &[kind.to_string()],
            &[],
            None,
            Some(50),
        )
        .await
        .expect("list events")
        .into_iter()
        .map(|event| event.data)
        .collect()
}

/// The acceptance path: the call is blocked until a person answers through the
/// API, the answer survives into a fresh gate (a restarted worker), and it lets
/// exactly that call through, once.
#[tokio::test]
async fn an_open_world_call_waits_for_a_person_and_runs_once_approved() {
    let Fixture { server, resumes } = fixture().await;
    let session_id = parked_session(&server).await;
    let call = send_email("toolu_1", "cfo@example.com");
    let ids = park_on(&server, session_id, std::slice::from_ref(&call)).await;
    assert_eq!(ids, ["tool_approval_toolu_1"]);

    // Still blocked while nobody has answered, on any worker.
    assert!(matches!(
        gate(&server, session_id, call.clone()).await,
        PreToolUseDecision::Defer { .. }
    ));
    assert_eq!(resumes.load(Ordering::SeqCst), 0);

    let response = post_decisions(
        &server,
        session_id,
        json!([{ "tool_call_id": "tool_approval_toolu_1", "decision": "allow" }]),
    )
    .await
    .assert_status(StatusCode::OK)
    .json_value();
    assert_eq!(response["status"], "active");
    assert_eq!(response["resolved"][0]["outcome"], "allow");
    assert_eq!(resumes.load(Ordering::SeqCst), 1, "the turn resumes once");

    // The card closes, and the decision reaches the model as a user turn.
    let completed = events_of(&server, session_id, "tool.completed").await;
    assert_eq!(
        completed.last().unwrap()["tool_call_id"],
        "tool_approval_toolu_1"
    );
    let spoken = events_of(&server, session_id, "input.message").await;
    let text = serde_json::to_string(spoken.last().unwrap()).unwrap();
    assert!(text.contains("send_email"), "unexpected: {text}");

    // The retried call, in a fresh gate, runs — exactly once.
    assert!(matches!(
        gate(
            &server,
            session_id,
            send_email("toolu_2", "cfo@example.com")
        )
        .await,
        PreToolUseDecision::Continue(_)
    ));
    assert!(matches!(
        gate(
            &server,
            session_id,
            send_email("toolu_3", "cfo@example.com")
        )
        .await,
        PreToolUseDecision::Defer { .. }
    ));
}

#[tokio::test]
async fn a_one_off_approval_does_not_cover_different_arguments() {
    let Fixture { server, .. } = fixture().await;
    let session_id = parked_session(&server).await;
    park_on(
        &server,
        session_id,
        &[send_email("toolu_1", "cfo@example.com")],
    )
    .await;
    post_decisions(
        &server,
        session_id,
        json!([{ "tool_call_id": "tool_approval_toolu_1", "decision": "allow" }]),
    )
    .await
    .assert_status(StatusCode::OK);

    assert!(matches!(
        gate(
            &server,
            session_id,
            send_email("toolu_2", "everyone@example.com")
        )
        .await,
        PreToolUseDecision::Defer { .. }
    ));
}

#[tokio::test]
async fn always_answers_are_remembered_per_session_and_tool() {
    let Fixture { server, .. } = fixture().await;
    let session_id = parked_session(&server).await;
    park_on(
        &server,
        session_id,
        &[send_email("toolu_1", "a@example.com")],
    )
    .await;
    post_decisions(
        &server,
        session_id,
        json!([{ "tool_call_id": "tool_approval_toolu_1", "decision": "allow_always" }]),
    )
    .await
    .assert_status(StatusCode::OK);
    for (id, to) in [("toolu_2", "b@example.com"), ("toolu_3", "c@example.com")] {
        assert!(matches!(
            gate(&server, session_id, send_email(id, to)).await,
            PreToolUseDecision::Continue(_)
        ));
    }

    // Another session is not covered.
    let other = parked_session(&server).await;
    assert!(matches!(
        gate(&server, other, send_email("toolu_4", "b@example.com")).await,
        PreToolUseDecision::Defer { .. }
    ));

    park_on(&server, other, &[send_email("toolu_5", "d@example.com")]).await;
    post_decisions(
        &server,
        other,
        json!([{ "tool_call_id": "tool_approval_toolu_5", "decision": "reject_always" }]),
    )
    .await
    .assert_status(StatusCode::OK);
    assert!(matches!(
        gate(&server, other, send_email("toolu_6", "e@example.com")).await,
        PreToolUseDecision::Block { .. }
    ));
}

#[tokio::test]
async fn a_rejection_blocks_the_identical_retry() {
    let Fixture { server, .. } = fixture().await;
    let session_id = parked_session(&server).await;
    park_on(
        &server,
        session_id,
        &[send_email("toolu_1", "a@example.com")],
    )
    .await;
    post_decisions(
        &server,
        session_id,
        json!([{ "tool_call_id": "tool_approval_toolu_1", "decision": "reject" }]),
    )
    .await
    .assert_status(StatusCode::OK);
    assert!(matches!(
        gate(&server, session_id, send_email("toolu_2", "a@example.com")).await,
        PreToolUseDecision::Block { .. }
    ));
}

/// The turn resumes once, so every request in the batch is settled now; one
/// left out of the submission is not approved, and nothing is recorded for it.
#[tokio::test]
async fn a_request_left_out_of_a_batch_is_not_approved() {
    let Fixture { server, .. } = fixture().await;
    let session_id = parked_session(&server).await;
    park_on(
        &server,
        session_id,
        &[
            send_email("toolu_1", "a@example.com"),
            send_email("toolu_2", "b@example.com"),
        ],
    )
    .await;
    let response = post_decisions(
        &server,
        session_id,
        json!([{ "tool_call_id": "tool_approval_toolu_1", "decision": "allow" }]),
    )
    .await
    .assert_status(StatusCode::OK)
    .json_value();
    assert_eq!(response["resolved"][1]["outcome"], "not_approved");
    assert!(matches!(
        gate(&server, session_id, send_email("toolu_3", "b@example.com")).await,
        PreToolUseDecision::Defer { .. }
    ));
}

#[tokio::test]
async fn an_unknown_or_model_authored_call_is_not_an_approval() {
    let Fixture { server, resumes } = fixture().await;
    let session_id = parked_session(&server).await;
    park_on(
        &server,
        session_id,
        &[send_email("toolu_1", "a@example.com")],
    )
    .await;

    post_decisions(
        &server,
        session_id,
        json!([{ "tool_call_id": "toolu_1", "decision": "allow" }]),
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    post_decisions(&server, session_id, json!([]))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    assert_eq!(resumes.load(Ordering::SeqCst), 0);
    assert!(matches!(
        gate(&server, session_id, send_email("toolu_2", "a@example.com")).await,
        PreToolUseDecision::Defer { .. }
    ));
}

#[tokio::test]
async fn a_session_that_is_not_parked_rejects_a_decision() {
    let Fixture { server, .. } = fixture().await;
    let session_id = parked_session(&server).await;
    park_on(
        &server,
        session_id,
        &[send_email("toolu_1", "a@example.com")],
    )
    .await;
    server
        .db
        .update_session(
            TEST_ORG_ID,
            session_id,
            everruns_server::storage::models::UpdateSession {
                status: Some("idle".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update session status");

    post_decisions(
        &server,
        session_id,
        json!([{ "tool_call_id": "tool_approval_toolu_1", "decision": "allow" }]),
    )
    .await
    .assert_status(StatusCode::CONFLICT);
    assert!(matches!(
        gate(&server, session_id, send_email("toolu_2", "a@example.com")).await,
        PreToolUseDecision::Defer { .. }
    ));
}

#[tokio::test]
async fn a_caller_who_cannot_manage_the_session_cannot_approve() {
    let resumes = Arc::new(AtomicUsize::new(0));
    let policy = Arc::new(DenySessionManagement::default());
    let server = TestServer::in_memory_with_runner_and_permission_resolver(
        Arc::new(RecordingRunner {
            resume_calls: resumes.clone(),
        }),
        policy.clone(),
    )
    .await;
    let session_id = parked_session(&server).await;
    park_on(
        &server,
        session_id,
        &[send_email("toolu_1", "a@example.com")],
    )
    .await;
    policy.armed.store(true, Ordering::SeqCst);

    post_decisions(
        &server,
        session_id,
        json!([{ "tool_call_id": "tool_approval_toolu_1", "decision": "allow" }]),
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
    assert_eq!(resumes.load(Ordering::SeqCst), 0);
    assert!(matches!(
        gate(&server, session_id, send_email("toolu_2", "a@example.com")).await,
        PreToolUseDecision::Defer { .. }
    ));
}

/// Nobody answered: the sweep resolves the request as not approved, records
/// nothing, and resumes the turn. A late answer is refused.
#[tokio::test]
async fn an_unanswered_request_expires_as_not_approved() {
    let Fixture { server, resumes } = fixture().await;
    let session_id = parked_session(&server).await;
    let mut request = match gate(&server, session_id, send_email("toolu_1", "a@example.com")).await
    {
        PreToolUseDecision::Defer { result, .. } => {
            ToolApprovalRequired::from_tool_result(&result).unwrap()
        }
        other => panic!("expected a deferral, got {other:?}"),
    };
    request.expires_at = (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({ "tool_calls": [request.request_call()] }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit tool.call_requested");

    post_decisions(
        &server,
        session_id,
        json!([{ "tool_call_id": "tool_approval_toolu_1", "decision": "allow" }]),
    )
    .await
    .assert_status(StatusCode::CONFLICT);

    let event_service = everruns_server::EventService::new(
        server.db.clone(),
        everruns_server::EventDelivery::in_memory(),
    );
    // A generous generic timeout: only the request's own deadline applies.
    everruns_server::tool_result_timeout::sweep_timed_out_sessions(
        &server.db,
        &server.runner,
        &event_service,
        86_400,
    )
    .await
    .expect("sweep");
    assert_eq!(resumes.load(Ordering::SeqCst), 1);

    let completed = events_of(&server, session_id, "tool.completed").await;
    let text = serde_json::to_string(completed.last().unwrap()).unwrap();
    assert!(text.contains("expired"), "unexpected: {text}");
    assert!(matches!(
        gate(&server, session_id, send_email("toolu_2", "a@example.com")).await,
        PreToolUseDecision::Defer { .. }
    ));
}
