//! Form mode elicitation over Everruns' own MCP endpoint (EVE-1060,
//! knowledge/integrations/mcp.md, "Form mode elicitation").
//!
//! `/mcp` had exactly one MRTR use — a URL mode elicitation for a credential.
//! This covers the other one: an `ask_user` question set reaching a capable
//! client as a `requestedSchema` it renders itself, the answer resuming the
//! parked turn through the one shared resolution operation, and the two things
//! that must *not* happen — an incapable client being failed instead of served,
//! and a credential question ever becoming a form field.

use crate::test_harness;

use async_trait::async_trait;
use axum::http::{Method, StatusCode};
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::TurnBackend;
use everruns_core::{Caller, Permission, PermissionResolver};
use everruns_server::domains::agents::record::Agent;
use everruns_server::domains::sessions::record::Session;
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use test_harness::TestServer;
use uuid::Uuid;

const LATEST: &str = "2026-07-28";
const LEGACY: &str = "2025-06-18";
const TEST_ORG_ID: i64 = 1;
const CLIENT_CAPABILITIES_META_KEY: &str = "io.modelcontextprotocol/clientCapabilities";

/// Counts the durable resumes, which is how a test sees the turn restart.
struct RecordingRunner {
    resume_calls: Arc<AtomicUsize>,
}

/// Withholds `org:sessions:manage`, but only once armed.
///
/// `session.view` and `session.manage` both rest on that one permission, which
/// is the point of the test — a caller who cannot manage a session must not be
/// able to read its pending questions either. It also means the fixture cannot
/// build the parked session while the denial is live, since creating one needs
/// the same permission. So arm it after setup, not before.
#[derive(Default)]
struct DenySessionManagement {
    armed: AtomicBool,
}

impl DenySessionManagement {
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
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

#[async_trait]
impl TurnBackend for RecordingRunner {
    async fn start_turn(
        &self,
        request: everruns_core::host::TurnRequest,
    ) -> everruns_contracts::error::Result<everruns_core::host::TurnTicket> {
        if matches!(
            request.input,
            everruns_core::host::TurnInput::RecordedToolResults { .. }
        ) {
            self.resume_calls.fetch_add(1, Ordering::SeqCst);
        }
        // The server drops its tickets; this one never resolves.
        Ok(everruns_core::host::TurnTicket::new(
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

async fn test_server() -> (TestServer, Arc<AtomicUsize>) {
    let resume_calls = Arc::new(AtomicUsize::new(0));
    let server = TestServer::in_memory_with_runner(Arc::new(RecordingRunner {
        resume_calls: resume_calls.clone(),
    }))
    .await;
    (server, resume_calls)
}

/// `_meta` from a client that declared the base elicitation capability, which
/// is form mode.
fn elicitation_meta() -> Value {
    json!({ CLIENT_CAPABILITIES_META_KEY: { "elicitation": {} } })
}

fn tasks_meta() -> Value {
    json!({
        CLIENT_CAPABILITIES_META_KEY: {
            "elicitation": {},
            "extensions": { "io.modelcontextprotocol/tasks": {} }
        }
    })
}

fn tasks_only_meta() -> Value {
    json!({
        CLIENT_CAPABILITIES_META_KEY: {
            "extensions": { "io.modelcontextprotocol/tasks": {} }
        }
    })
}

async fn mcp_request(
    server: &TestServer,
    method: &str,
    params: Value,
    protocol_version: &str,
) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    server
        .request_raw(
            Method::POST,
            "/mcp",
            vec![
                ("content-type", "application/json"),
                ("MCP-Protocol-Version", protocol_version),
            ],
            serde_json::to_vec(&body).unwrap(),
        )
        .await
        .json()
}

async fn mcp_call(server: &TestServer, params: Value) -> Value {
    mcp_request(server, "tools/call", params, LATEST).await
}

async fn task_request(server: &TestServer, method: &str, task_id: &str, input: Value) -> Value {
    let mut params = json!({
        "taskId": task_id,
        "_meta": tasks_meta(),
    });
    if let (Some(params), Some(input)) = (params.as_object_mut(), input.as_object()) {
        for (key, value) in input {
            params.insert(key.clone(), value.clone());
        }
    }
    mcp_request(server, method, params, LATEST).await
}

/// One `session_get_status` call, merging any MRTR params into the request.
async fn poll_status(server: &TestServer, session_id: SessionId, extra: Value) -> Value {
    let mut params = json!({
        "name": "session_get_status",
        "arguments": { "session_id": session_id.to_string() },
    });
    if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            params.insert(key.clone(), value.clone());
        }
    }
    mcp_call(server, params).await
}

async fn create_agent(server: &TestServer) -> Agent {
    server
        .post(
            "/v1/agents",
            json!({
                "name": "form-elicitation-test-agent",
                "display_name": "Form Elicitation Test",
                "description": "Agent for the form elicitation test",
                "system_prompt": "You are a helpful assistant"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

/// A session parked exactly where the act pauses on a client-side tool call.
async fn parked_session(server: &TestServer) -> SessionId {
    parked_session_for_agent(server, false).await
}
async fn parked_session_for_agent(server: &TestServer, platform: bool) -> SessionId {
    let agent = create_agent(server).await;
    let session: Session = server
        .post(
            "/v1/sessions",
            if platform {
                json!({"agent_name":"platform-chat"})
            } else {
                json!({ "agent_id": agent.public_id })
            },
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    server
        .db
        .update_session(
            TEST_ORG_ID,
            session.id,
            everruns_server::storage::UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update session status")
        .expect("session exists");
    session.id
}

async fn another_users_platform_chat_session(server: &TestServer) -> SessionId {
    let session_id = parked_session_for_agent(server, true).await;
    let owner = server
        .db
        .create_user(everruns_server::storage::CreateUserRow {
            email: format!("platform-chat-owner-{}@example.com", Uuid::now_v7()),
            name: "Platform Chat Owner".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("create Platform Chat owner");
    server
        .db
        .add_organization_member(TEST_ORG_ID, owner.id, "member")
        .await
        .expect("add Platform Chat owner to the caller's organization");
    server
        .db
        .update_session(
            TEST_ORG_ID,
            session_id,
            everruns_server::storage::UpdateSession {
                harness_id: Some(server.seed_chat_harness_id.parse().unwrap()),
                resolved_owner_user_id: everruns_server::storage::UpdateField::Set(owner.id),
                ..Default::default()
            },
        )
        .await
        .expect("update Platform Chat ownership")
        .expect("session exists");
    session_id
}

/// Store the input message whose turn is the task, and return the task id.
/// Seeded before the `ask_user` call, as a real turn orders them.
async fn seed_task_input(server: &TestServer, session_id: SessionId) -> String {
    let message_id = everruns_contracts::typed_id::MessageId::new();
    server
        .db
        .create_event(everruns_server::storage::CreateEventRow {
            session_id,
            event_type: "input.message".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({
                "message": {
                    "id": message_id.to_string(),
                    "role": "user",
                    "content": [{ "type": "text", "text": "Deploy it" }]
                }
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit input.message");
    format!(
        "{}.{session_id}.{message_id}",
        everruns_core::DEFAULT_ORG_PUBLIC_ID
    )
}

/// Emit the `ask_user` call the engine emits, with whatever questions the test
/// needs. Ids are always present because normalization fills them in before the
/// call is emitted.
async fn emit_ask_user(
    server: &TestServer,
    session_id: SessionId,
    tool_call_id: &str,
    questions: Value,
) {
    server
        .db
        .create_event(everruns_server::storage::CreateEventRow {
            session_id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({
                "tool_calls": [{
                    "id": tool_call_id,
                    "name": "ask_user",
                    "arguments": { "questions": questions, "timeout_seconds": 300 }
                }]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit tool.call_requested");
}

fn choice_questions() -> Value {
    json!([
        {
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

/// Every `tool.completed` payload on the session.
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
        .expect("list events")
        .into_iter()
        .map(|event| event.data)
        .collect()
}

/// The `ask_user` outcome recorded against the call, read back from the tool
/// result the resolution wrote — which is exactly what the model reads.
async fn recorded_outcome(server: &TestServer, session_id: SessionId) -> Value {
    let results = completed_results(server, session_id).await;
    assert_eq!(results.len(), 1, "exactly one tool.completed: {results:?}");
    let text = results[0]["result"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("tool result text: {}", results[0]));
    serde_json::from_str(text).expect("the ask_user result is JSON")
}

#[tokio::test]
async fn a_parked_question_set_reaches_a_capable_client_as_a_form() {
    let (server, _) = test_server().await;
    let session_id = parked_session(&server).await;
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

    let response = poll_status(&server, session_id, json!({ "_meta": elicitation_meta() })).await;

    let result = &response["result"];
    assert_eq!(
        result["resultType"], "input_required",
        "expected an elicitation, got {response}"
    );
    assert!(
        result["requestState"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    let request = &result["inputRequests"]["ask_user"];
    assert_eq!(request["method"], "elicitation/create");
    assert_eq!(request["params"]["mode"], "form");

    let schema = &request["params"]["requestedSchema"];
    assert_eq!(schema["type"], "object");
    // Single-select: an enum of the offered labels, with the trade-offs the
    // model wrote carried in the description rather than dropped.
    let target = &schema["properties"]["target"];
    assert_eq!(target["type"], "string");
    assert_eq!(target["enum"], json!(["Staging", "Production"]));
    assert_eq!(target["enumNames"], json!(["Staging", "Production"]));
    assert_eq!(target["default"], "Staging");
    assert!(
        target["description"]
            .as_str()
            .unwrap()
            .contains("Safe, reversible.")
    );
    // Multi-select: one boolean per option, because the profile has no array.
    assert_eq!(schema["properties"]["areas__0"]["type"], "boolean");
    assert_eq!(schema["properties"]["areas__1"]["title"], "Areas: UI");
    assert_eq!(
        schema["required"],
        json!(["target", "areas__0", "areas__1"])
    );
    // Flat: nothing in the profile may nest.
    for (_, property) in schema["properties"].as_object().unwrap() {
        assert!(property["properties"].is_null());
        assert_ne!(property["type"], "array");
        assert_ne!(property["type"], "object");
    }
}

#[tokio::test]
async fn session_policy_denial_does_not_disclose_or_resolve_a_question_set() {
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
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;
    policy.arm();

    let response = poll_status(&server, session_id, json!({ "_meta": elicitation_meta() })).await;

    assert_eq!(response["result"]["isError"], true, "got {response}");
    assert!(
        response["result"]["requestState"].is_null(),
        "got {response}"
    );
    assert!(
        response["result"]["inputRequests"].is_null(),
        "got {response}"
    );
    assert!(completed_results(&server, session_id).await.is_empty());
    assert_eq!(resumes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn answering_the_form_resolves_the_question_and_resumes_the_turn() {
    let (server, resumes) = test_server().await;
    let session_id = parked_session(&server).await;
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

    let elicited = poll_status(&server, session_id, json!({ "_meta": elicitation_meta() })).await;
    let request_state = elicited["result"]["requestState"].as_str().unwrap();

    let answered = poll_status(
        &server,
        session_id,
        json!({
            "_meta": elicitation_meta(),
            "requestState": request_state,
            "inputResponses": { "ask_user": {
                "action": "accept",
                "content": { "target": "Production", "areas__0": true, "areas__1": false }
            }}
        }),
    )
    .await;

    // The same poll now reports the session rather than asking again.
    assert_eq!(answered["result"]["resultType"], "complete", "{answered}");
    assert!(answered["result"]["inputRequests"].is_null());

    let outcome = recorded_outcome(&server, session_id).await;
    assert_eq!(outcome["status"], "answered");
    assert_eq!(outcome["answered_by"], "user");
    assert_eq!(outcome["answers"][0]["id"], "target");
    assert_eq!(outcome["answers"][0]["selected"], json!(["Production"]));
    assert_eq!(outcome["answers"][1]["selected"], json!(["API"]));
    assert_eq!(resumes.load(Ordering::SeqCst), 1, "the turn should resume");
}

#[tokio::test]
async fn a_declined_form_is_a_finished_decision_and_a_cancelled_one_claims_nothing() {
    for (action, expected_status, expected_by) in [
        ("decline", "declined", "user"),
        ("cancel", "cancelled", "unattended"),
    ] {
        let (server, resumes) = test_server().await;
        let session_id = parked_session(&server).await;
        emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

        let elicited =
            poll_status(&server, session_id, json!({ "_meta": elicitation_meta() })).await;
        let request_state = elicited["result"]["requestState"].as_str().unwrap();
        poll_status(
            &server,
            session_id,
            json!({
                "_meta": elicitation_meta(),
                "requestState": request_state,
                "inputResponses": { "ask_user": { "action": action } }
            }),
        )
        .await;

        let outcome = recorded_outcome(&server, session_id).await;
        assert_eq!(outcome["status"], expected_status, "action {action}");
        assert_eq!(outcome["answered_by"], expected_by, "action {action}");
        // Neither outcome carries answers: nothing is claimed on the person's
        // behalf by a form they refused or dismissed.
        assert_eq!(outcome["answers"], json!([]), "action {action}");
        assert_eq!(resumes.load(Ordering::SeqCst), 1, "action {action}");
    }
}

/// The deliberate difference from URL mode: an unanswerable question is not a
/// failed call.
#[tokio::test]
async fn a_client_that_declares_nothing_gets_a_status_result_not_a_missing_capability_error() {
    let (server, _) = test_server().await;
    let session_id = parked_session(&server).await;
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

    let response = poll_status(&server, session_id, json!({})).await;

    assert!(
        response["error"].is_null(),
        "an unelicitable client must not be failed: {response}"
    );
    assert_eq!(response["result"]["resultType"], "complete");
    assert!(response["result"]["inputRequests"].is_null());
    // Nothing was answered on its behalf here either; the turn it polls never
    // parked in the first place, because the session declared no `ask_user`
    // hint (EVE-1057).
    assert!(completed_results(&server, session_id).await.is_empty());
}

/// THREAT[TM-AGENT-016]: an `ask_user` answer is a tool result, so a credential
/// must never be a form property.
#[tokio::test]
async fn a_secret_question_never_becomes_a_form_field() {
    let (server, _) = test_server().await;
    let session_id = parked_session(&server).await;
    emit_ask_user(
        &server,
        session_id,
        "call_1",
        json!([{
            "kind": "secret",
            "id": "stripe",
            "header": "Stripe key",
            "question": "Which Stripe restricted key should I use?",
            "multi_select": false,
            "allow_other": false,
            "options": [],
            "secret_name": "STRIPE_API_KEY",
            "purpose": "Read-only charge lookups."
        }]),
    )
    .await;

    let response = poll_status(&server, session_id, json!({ "_meta": elicitation_meta() })).await;

    assert_eq!(
        response["result"]["resultType"], "complete",
        "a secret question must not be elicited as a form: {response}"
    );
    assert!(response["result"]["inputRequests"].is_null());
    assert!(
        !serde_json::to_string(&response)
            .unwrap()
            .contains("STRIPE_API_KEY")
    );
}

/// MRTR treats `requestState` as attacker-controlled input.
#[tokio::test]
async fn a_forged_request_state_is_rejected() {
    let (server, _) = test_server().await;
    let session_id = parked_session(&server).await;
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

    let response = poll_status(
        &server,
        session_id,
        json!({
            "_meta": elicitation_meta(),
            "requestState": "forged.state",
            "inputResponses": { "ask_user": {
                "action": "accept",
                "content": { "target": "Production", "areas__0": true, "areas__1": false }
            }}
        }),
    )
    .await;

    assert_eq!(response["error"]["code"], -32602, "got {response}");
    assert!(
        completed_results(&server, session_id).await.is_empty(),
        "a forged state must answer nothing"
    );
}

/// An answer that does not match what was asked is refused without losing the
/// question: the turn stays parked and the client can answer again.
#[tokio::test]
async fn an_option_that_was_never_offered_is_refused_and_the_turn_stays_parked() {
    let (server, _) = test_server().await;
    let session_id = parked_session(&server).await;
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

    let elicited = poll_status(&server, session_id, json!({ "_meta": elicitation_meta() })).await;
    let request_state = elicited["result"]["requestState"]
        .as_str()
        .unwrap()
        .to_string();

    let refused = poll_status(
        &server,
        session_id,
        json!({
            "_meta": elicitation_meta(),
            "requestState": request_state,
            "inputResponses": { "ask_user": {
                "action": "accept",
                "content": { "target": "Wherever", "areas__0": true, "areas__1": false }
            }}
        }),
    )
    .await;
    assert_eq!(refused["result"]["isError"], true, "got {refused}");

    // Still parked, so the next poll asks again rather than dropping the turn.
    let again = poll_status(&server, session_id, json!({ "_meta": elicitation_meta() })).await;
    assert_eq!(again["result"]["resultType"], "input_required");
}

#[tokio::test]
async fn tasks_get_surfaces_the_pending_question_as_an_input_request() {
    let (server, _) = test_server().await;
    let session_id = parked_session(&server).await;
    let task_id = seed_task_input(&server, session_id).await;
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

    let response = task_request(&server, "tasks/get", &task_id, json!({})).await;

    let task = &response["result"];
    assert_eq!(task["status"], "input_required", "{response}");
    let request = &task["inputRequests"]["ask_user:call_1"];
    assert_eq!(request["method"], "elicitation/create");
    assert_eq!(request["params"]["mode"], "form");
    assert_eq!(
        request["params"]["requestedSchema"]["properties"]["target"]["enum"],
        json!(["Staging", "Production"])
    );
    assert_eq!(
        request["params"]["requestedSchema"]["properties"]["areas__0"]["type"],
        "boolean"
    );
    assert_eq!(task["resultType"], "complete");
    assert!(task.get("result").is_none(), "{response}");
}

#[tokio::test]
async fn tasks_update_resolves_a_structured_answer_through_the_shared_operation() {
    let (server, resumes) = test_server().await;
    let session_id = parked_session(&server).await;
    let task_id = seed_task_input(&server, session_id).await;
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

    let response = task_request(
        &server,
        "tasks/update",
        &task_id,
        json!({
            "inputResponses": {
                "ask_user:call_1": {
                    "action": "accept",
                    "content": {
                        "target": "Production",
                        "areas__0": true,
                        "areas__1": false
                    }
                }
            }
        }),
    )
    .await;

    assert_eq!(
        response["result"],
        json!({ "resultType": "complete" }),
        "{response}"
    );
    let outcome = recorded_outcome(&server, session_id).await;
    assert_eq!(outcome["status"], "answered");
    assert_eq!(outcome["answered_by"], "user");
    assert_eq!(outcome["answers"][0]["selected"], json!(["Production"]));
    assert_eq!(outcome["answers"][1]["selected"], json!(["API"]));
    assert_eq!(resumes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn tasks_update_rejects_an_unoffered_option_without_resuming() {
    let (server, resumes) = test_server().await;
    let session_id = parked_session(&server).await;
    let task_id = seed_task_input(&server, session_id).await;
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

    let response = task_request(
        &server,
        "tasks/update",
        &task_id,
        json!({
            "inputResponses": {
                "ask_user:call_1": {
                    "action": "accept",
                    "content": {
                        "target": "Wherever",
                        "areas__0": true,
                        "areas__1": false
                    }
                }
            }
        }),
    )
    .await;

    assert_eq!(response["error"]["code"], -32602, "{response}");
    assert_eq!(resumes.load(Ordering::SeqCst), 0);
    assert!(completed_results(&server, session_id).await.is_empty());
    let still_pending = task_request(&server, "tasks/get", &task_id, json!({})).await;
    assert_eq!(still_pending["result"]["status"], "input_required");
    assert!(
        still_pending["result"]["inputRequests"]["ask_user:call_1"].is_object(),
        "{still_pending}"
    );
}

#[tokio::test]
async fn tasks_update_rejects_non_owner_platform_chat_question_responses() {
    for response in [
        json!({
            "action": "accept",
            "content": { "target": "Production", "areas__0": true, "areas__1": false }
        }),
        json!({ "action": "decline" }),
        json!({ "action": "cancel" }),
    ] {
        let (server, resumes) = test_server().await;
        let session_id = another_users_platform_chat_session(&server).await;
        let task_id = seed_task_input(&server, session_id).await;
        emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

        let result = task_request(
            &server,
            "tasks/update",
            &task_id,
            json!({ "inputResponses": { "ask_user:call_1": response } }),
        )
        .await;

        assert_eq!(result["error"]["code"], -32602, "got {result}");
        assert_eq!(resumes.load(Ordering::SeqCst), 0);
        assert!(completed_results(&server, session_id).await.is_empty());
    }
}

#[tokio::test]
async fn a_legacy_client_cannot_enter_the_tasks_question_flow() {
    let (server, _) = test_server().await;
    let session_id = parked_session(&server).await;
    emit_ask_user(&server, session_id, "call_1", choice_questions()).await;

    let response = mcp_request(
        &server,
        "tasks/get",
        json!({
            "taskId": session_id.to_string(),
            "_meta": tasks_meta(),
        }),
        LEGACY,
    )
    .await;

    assert_eq!(response["error"]["code"], -32601, "{response}");
    assert!(response["result"].is_null(), "{response}");
}

/// The hint is a claim about the client, so only a client that can be elicited
/// makes it. Without it the turn never parks (EVE-1057).
#[tokio::test]
async fn agent_run_declares_the_ask_user_hint_only_for_a_capable_client() {
    let (server, _) = test_server().await;
    let agent = create_agent(&server).await;

    for (meta, expected) in [
        (json!({ "_meta": elicitation_meta() }), Some(json!(true))),
        (json!({ "_meta": tasks_only_meta() }), None),
        (json!({}), None),
    ] {
        let mut params = json!({
            "name": "agent_run",
            "arguments": { "agent_id": agent.public_id, "message": "hello" },
        });
        if let (Some(params), Some(meta)) = (params.as_object_mut(), meta.as_object()) {
            for (key, value) in meta {
                params.insert(key.clone(), value.clone());
            }
        }
        let response = mcp_call(&server, params).await;
        // A Tasks client gets the turn as a task, whose id names the session.
        let session_id = match response["result"]["taskId"].as_str() {
            Some(task_id) => task_id.split('.').nth(1).unwrap().to_string(),
            None => {
                let content: Value = serde_json::from_str(
                    response["result"]["content"][0]["text"]
                        .as_str()
                        .unwrap_or_else(|| panic!("agent_run content: {response}")),
                )
                .unwrap();
                content["session_id"].as_str().unwrap().to_string()
            }
        };
        let session: Value = server
            .get(&format!("/v1/sessions/{session_id}"))
            .await
            .assert_success()
            .json_value();
        assert_eq!(
            session["hints"].get("ask_user").cloned(),
            expected,
            "hints were {}",
            session["hints"]
        );
    }
}
