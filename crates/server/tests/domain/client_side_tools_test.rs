//! Unit-style tests for client-side tool calls feature.
//!
//! Tests serialization, deserialization, and type behavior for:
//! - Agent with client-side tools
//! - SubmitToolResultsRequest/Response
//! - ToolCallRequestedData event
//! - Session tools propagation
//!
//! No database required.
//!
//! Run with: cargo test -p everruns-server --test domain client_side_tools_test::
use crate::test_harness;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;

use axum::http::StatusCode;
use everruns_builtins::normalize_ask_user_arguments;
use everruns_platform::{Agent, Session};
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use everruns_server::storage::models::{ReserveActiveTurnSlotResult, WaitingTurnResolutionPlan};
use everruns_worker::AgentRunner;
use serde_json::json;
use test_harness::TestServer;

// ============================================
// Agent with Client-Side Tools
// ============================================

#[test]
fn test_agent_with_client_side_tools_serialization() {
    let agent_json = json!({
        "id": "agent_550e8400e29b41d4a716446655440000",
        "name": "browser-agent",
        "display_name": "Browser Agent",
        "system_prompt": "You control a browser.",
        "harness_id": "harness_00000000000000000000000000000000",
        "status": "active",
        "tags": [],
        "tools": [
            {
                "type": "client_side",
                "name": "browser_click",
                "description": "Click an element",
                "parameters": {"type": "object", "properties": {"selector": {"type": "string"}}}
            }
        ],
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    });

    let agent: everruns_platform::Agent = serde_json::from_value(agent_json).unwrap();
    assert_eq!(
        agent.harness_id.to_string(),
        "harness_00000000000000000000000000000000"
    );
    assert_eq!(agent.tools.len(), 1);
    assert_eq!(agent.tools[0].name(), "browser_click");
    assert!(matches!(
        &agent.tools[0],
        everruns_provider::tool_types::ToolDefinition::ClientSide(_)
    ));
}

#[test]
fn test_agent_with_mixed_tools_serialization() {
    let agent_json = json!({
        "id": "agent_550e8400e29b41d4a716446655440000",
        "name": "mixed-agent",
        "display_name": "Mixed Agent",
        "system_prompt": "You have both tool types.",
        "harness_id": "harness_00000000000000000000000000000000",
        "status": "active",
        "tags": [],
        "tools": [
            {
                "type": "builtin",
                "name": "fetch_data",
                "description": "Fetch from URL",
                "parameters": {"type": "object"}
            },
            {
                "type": "client_side",
                "name": "run_terminal",
                "description": "Run a terminal command",
                "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}}
            }
        ],
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    });

    let agent: everruns_platform::Agent = serde_json::from_value(agent_json).unwrap();
    assert_eq!(
        agent.harness_id.to_string(),
        "harness_00000000000000000000000000000000"
    );
    assert_eq!(agent.tools.len(), 2);

    assert!(matches!(
        &agent.tools[0],
        everruns_provider::tool_types::ToolDefinition::Builtin(_)
    ));
    assert!(matches!(
        &agent.tools[1],
        everruns_provider::tool_types::ToolDefinition::ClientSide(_)
    ));

    // Roundtrip
    let serialized = serde_json::to_value(&agent).unwrap();
    let tools = serialized["tools"].as_array().unwrap();
    assert_eq!(tools[0]["type"], "builtin");
    assert_eq!(tools[1]["type"], "client_side");
}

#[test]
fn test_agent_with_no_tools_omits_field() {
    let agent_json = json!({
        "id": "agent_550e8400e29b41d4a716446655440000",
        "name": "no-tools-agent",
        "display_name": "No Tools Agent",
        "system_prompt": "No tools.",
        "harness_id": "harness_00000000000000000000000000000000",
        "status": "active",
        "tags": [],
        "created_at": "2025-01-01T00:00:00Z",
        "updated_at": "2025-01-01T00:00:00Z"
    });

    let agent: everruns_platform::Agent = serde_json::from_value(agent_json).unwrap();
    assert_eq!(
        agent.harness_id.to_string(),
        "harness_00000000000000000000000000000000"
    );
    assert!(agent.tools.is_empty());

    // Serialized output should omit tools field when empty (skip_serializing_if)
    let serialized = serde_json::to_value(&agent).unwrap();
    assert!(serialized.get("tools").is_none());
}

// ============================================
// SubmitToolResultsRequest Serialization
// ============================================

#[test]
fn test_submit_tool_results_request_success() {
    use everruns_server::api::tool_results::SubmitToolResultsRequest;

    let json = json!({
        "tool_results": [
            {
                "tool_call_id": "call_abc123",
                "result": {"status": "deployed", "url": "https://staging.app"}
            }
        ]
    });

    let req: SubmitToolResultsRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.tool_results.len(), 1);
    assert_eq!(req.tool_results[0].tool_call_id, "call_abc123");
    assert!(req.tool_results[0].result.is_some());
    assert!(req.tool_results[0].error.is_none());
}

#[test]
fn test_submit_tool_results_request_error() {
    use everruns_server::api::tool_results::SubmitToolResultsRequest;

    let json = json!({
        "tool_results": [
            {
                "tool_call_id": "call_fail1",
                "error": "Connection timeout after 30s"
            }
        ]
    });

    let req: SubmitToolResultsRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.tool_results[0].tool_call_id, "call_fail1");
    assert!(req.tool_results[0].result.is_none());
    assert_eq!(
        req.tool_results[0].error.as_ref().unwrap(),
        "Connection timeout after 30s"
    );
}

#[test]
fn test_submit_tool_results_request_mixed() {
    use everruns_server::api::tool_results::SubmitToolResultsRequest;

    let json = json!({
        "tool_results": [
            {"tool_call_id": "call_1", "result": 42},
            {"tool_call_id": "call_2", "error": "not found"},
            {"tool_call_id": "call_3", "result": null},
            {"tool_call_id": "call_4", "result": {"nested": true}}
        ]
    });

    let req: SubmitToolResultsRequest = serde_json::from_value(json).unwrap();
    assert_eq!(req.tool_results.len(), 4);
    assert_eq!(req.tool_results[0].tool_call_id, "call_1");
    assert!(req.tool_results[0].error.is_none());
    assert_eq!(req.tool_results[1].error.as_deref(), Some("not found"));
    // null result still deserializes as None
    assert!(req.tool_results[2].result.is_none());
    assert!(req.tool_results[3].result.is_some());
}

#[test]
fn test_submit_tool_results_request_empty_results() {
    use everruns_server::api::tool_results::SubmitToolResultsRequest;

    let json = json!({
        "tool_results": []
    });

    let req: SubmitToolResultsRequest = serde_json::from_value(json).unwrap();
    assert!(req.tool_results.is_empty());
}

#[test]
fn test_submit_tool_results_response_serialization() {
    use everruns_server::api::tool_results::SubmitToolResultsResponse;

    let resp = SubmitToolResultsResponse {
        accepted: 3,
        status: "active".to_string(),
    };

    let json = serde_json::to_value(&resp).unwrap();
    assert_eq!(json["accepted"], 3);
    assert_eq!(json["status"], "active");
}

// ============================================
// ToolCallRequestedData Serialization
// ============================================

#[test]
fn test_tool_call_requested_data_serialization() {
    use everruns_core::events::ToolCallRequestedData;
    use everruns_provider::tool_types::ToolCall;

    let data = ToolCallRequestedData {
        tool_calls: vec![
            ToolCall {
                id: "call_abc".to_string(),
                name: "browser_click".to_string(),
                arguments: json!({"selector": "#submit-btn"}),
            },
            ToolCall {
                id: "call_def".to_string(),
                name: "browser_type".to_string(),
                arguments: json!({"selector": "#input", "text": "hello"}),
            },
        ],
        tool_summaries: vec![],
        headline: None,
        completed_headline: None,
    };

    let json = serde_json::to_value(&data).unwrap();
    let tool_calls = json["tool_calls"].as_array().unwrap();
    assert_eq!(tool_calls.len(), 2);
    assert_eq!(tool_calls[0]["id"], "call_abc");
    assert_eq!(tool_calls[0]["name"], "browser_click");
    assert_eq!(tool_calls[0]["arguments"]["selector"], "#submit-btn");
    assert_eq!(tool_calls[1]["id"], "call_def");
    assert_eq!(tool_calls[1]["name"], "browser_type");
}

#[test]
fn test_tool_call_requested_data_roundtrip() {
    use everruns_core::events::ToolCallRequestedData;
    use everruns_provider::tool_types::ToolCall;

    let original = ToolCallRequestedData {
        tool_calls: vec![ToolCall {
            id: "call_xyz".to_string(),
            name: "deploy_staging".to_string(),
            arguments: json!({"env": "staging", "version": "1.2.3"}),
        }],
        tool_summaries: vec![],
        headline: None,
        completed_headline: None,
    };

    let json_str = serde_json::to_string(&original).unwrap();
    let parsed: ToolCallRequestedData = serde_json::from_str(&json_str).unwrap();

    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].id, "call_xyz");
    assert_eq!(parsed.tool_calls[0].name, "deploy_staging");
    assert_eq!(parsed.tool_calls[0].arguments["env"], "staging");
}

#[test]
fn test_tool_call_requested_data_empty_tool_calls() {
    use everruns_core::events::ToolCallRequestedData;

    let data = ToolCallRequestedData {
        tool_calls: vec![],
        tool_summaries: vec![],
        headline: None,
        completed_headline: None,
    };

    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["tool_calls"].as_array().unwrap().len(), 0);
}

// ============================================
// Client-Side Tool Definition in Session Context
// ============================================

#[test]
fn test_client_side_tool_in_session_tools_json() {
    // Verify client-side tools survive JSON roundtrip in session-like contexts
    let tools_json = json!([
        {
            "type": "client_side",
            "name": "screenshot",
            "description": "Take a screenshot of the current page",
            "parameters": {"type": "object"}
        },
        {
            "type": "client_side",
            "name": "navigate",
            "description": "Navigate to a URL",
            "parameters": {
                "type": "object",
                "properties": {"url": {"type": "string", "format": "uri"}},
                "required": ["url"]
            }
        }
    ]);

    let tools: Vec<everruns_provider::tool_types::ToolDefinition> =
        serde_json::from_value(tools_json.clone()).unwrap();
    assert_eq!(tools.len(), 2);

    for tool in &tools {
        assert!(matches!(
            tool,
            everruns_provider::tool_types::ToolDefinition::ClientSide(_)
        ));
        assert_eq!(
            tool.policy(),
            &everruns_provider::tool_types::ToolPolicy::ClientSide
        );
    }

    // Roundtrip
    let serialized = serde_json::to_value(&tools).unwrap();
    assert_eq!(serialized, tools_json);
}

// ============================================
// ToolCall + ToolResult Correlation
// ============================================

#[test]
fn test_tool_call_and_result_correlation() {
    use everruns_provider::tool_types::{ToolCall, ToolResult};

    let tool_call = ToolCall {
        id: "call_corr123".to_string(),
        name: "run_command".to_string(),
        arguments: json!({"cmd": "ls -la"}),
    };

    let tool_result = ToolResult {
        tool_call_id: tool_call.id.clone(),
        result: Some(json!({"output": "total 42\n..."})),
        images: None,
        error: None,
        connection_required: None,
        raw_output: None,
    };

    // IDs correlate
    assert_eq!(tool_call.id, tool_result.tool_call_id);

    // Both serialize/deserialize correctly
    let call_json = serde_json::to_string(&tool_call).unwrap();
    let result_json = serde_json::to_string(&tool_result).unwrap();

    let parsed_call: ToolCall = serde_json::from_str(&call_json).unwrap();
    let parsed_result: ToolResult = serde_json::from_str(&result_json).unwrap();

    assert_eq!(parsed_call.id, parsed_result.tool_call_id);
}

struct RecordingRunner {
    resumed_sessions: tokio::sync::mpsc::UnboundedSender<SessionId>,
}

struct BlockingRunner {
    entered: tokio::sync::mpsc::UnboundedSender<SessionId>,
    releases: Arc<tokio::sync::Semaphore>,
}

struct FastCompletingRunner {
    db: Mutex<Option<Arc<everruns_server::storage::StorageBackend>>>,
    completed_status: &'static str,
}

impl FastCompletingRunner {
    fn new(completed_status: &'static str) -> Self {
        Self {
            db: Mutex::new(None),
            completed_status,
        }
    }

    fn attach(&self, db: Arc<everruns_server::storage::StorageBackend>) {
        *self.db.lock().expect("runner database lock") = Some(db);
    }
}

#[async_trait]
impl AgentRunner for FastCompletingRunner {
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
        session_id: SessionId,
        _resolution_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        let db = self
            .db
            .lock()
            .expect("runner database lock")
            .clone()
            .expect("runner database attached");
        db.update_session(
            1,
            session_id,
            everruns_server::storage::models::UpdateSession {
                status: Some(self.completed_status.to_string()),
                ..Default::default()
            },
        )
        .await?;
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

#[async_trait]
impl AgentRunner for BlockingRunner {
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
        session_id: SessionId,
        _resolution_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        self.entered
            .send(session_id)
            .map_err(|_| anyhow::anyhow!("resume observer dropped"))?;
        self.releases.acquire().await?.forget();
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

#[tokio::test]
async fn user_message_claim_rejects_concurrent_tool_result() {
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let releases = Arc::new(tokio::sync::Semaphore::new(0));
    let server = Arc::new(
        TestServer::in_memory_with_runner(Arc::new(BlockingRunner {
            entered: entered_tx,
            releases: releases.clone(),
        }))
        .await,
    );
    let session = create_waiting_client_tool_session(&server, "message-result").await;
    let first_server = server.clone();
    let first = tokio::spawn(async move {
        first_server
            .post(
                &format!("/v1/sessions/{}/messages", session.id),
                json!({
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": "typed answer wins"}]
                    }
                }),
            )
            .await
    });
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), entered_rx.recv())
            .await
            .expect("resume entry timeout")
            .expect("resume entry"),
        session.id
    );

    server
        .post(
            &format!("/v1/sessions/{}/tool-results", session.id),
            json!({
                "tool_results": [{
                    "tool_call_id": "call_message-result",
                    "result": {"answer": "late"}
                }]
            }),
        )
        .await
        .assert_status(StatusCode::CONFLICT);
    releases.add_permits(1);
    first
        .await
        .expect("message task")
        .assert_status(StatusCode::CREATED);

    let completions = server
        .db
        .list_events(
            session.id,
            None,
            None,
            &["tool.completed".to_string()],
            &[],
            None,
            None,
        )
        .await
        .expect("list completions");
    let matching: Vec<_> = completions
        .iter()
        .filter(|event| event.data["tool_call_id"] == "call_message-result")
        .collect();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].data["status"], "cancelled");
}

#[tokio::test]
async fn user_message_claim_rejects_concurrent_user_message() {
    let (entered_tx, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let releases = Arc::new(tokio::sync::Semaphore::new(0));
    let server = Arc::new(
        TestServer::in_memory_with_runner(Arc::new(BlockingRunner {
            entered: entered_tx,
            releases: releases.clone(),
        }))
        .await,
    );
    let session = create_waiting_client_tool_session(&server, "message-message").await;
    let first_server = server.clone();
    let first = tokio::spawn(async move {
        first_server
            .post(
                &format!("/v1/sessions/{}/messages", session.id),
                json!({
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": "first typed answer"}]
                    }
                }),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), entered_rx.recv())
        .await
        .expect("resume entry timeout")
        .expect("resume entry");

    server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "second typed answer"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CONFLICT);
    releases.add_permits(1);
    first
        .await
        .expect("message task")
        .assert_status(StatusCode::CREATED);

    let messages = server
        .db
        .list_message_events_limited(session.id, None)
        .await
        .expect("list messages");
    assert!(
        messages
            .iter()
            .any(|event| { event.data["message"]["content"][0]["text"] == "first typed answer" })
    );
    assert!(
        !messages
            .iter()
            .any(|event| { event.data["message"]["content"][0]["text"] == "second typed answer" })
    );
}

#[tokio::test]
async fn tool_result_succeeds_for_fast_worker_advanced_statuses() {
    for completed_status in ["active", "idle", "paused"] {
        let runner = Arc::new(FastCompletingRunner::new(completed_status));
        let server = TestServer::in_memory_with_runner(runner.clone()).await;
        runner.attach(server.db.clone());
        let session =
            create_waiting_client_tool_session(&server, &format!("fast-worker-{completed_status}"))
                .await;

        server
            .post(
                &format!("/v1/sessions/{}/tool-results", session.id),
                json!({
                    "tool_results": [{
                        "tool_call_id": format!("call_fast-worker-{completed_status}"),
                        "result": {"answer": "done"}
                    }]
                }),
            )
            .await
            .assert_status(StatusCode::OK);

        assert_eq!(
            server
                .db
                .get_session(1, session.id)
                .await
                .unwrap()
                .unwrap()
                .status,
            completed_status
        );
    }
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
        session_id: SessionId,
        _resolution_id: uuid::Uuid,
    ) -> anyhow::Result<()> {
        self.resumed_sessions
            .send(session_id)
            .map_err(|_| anyhow::anyhow!("resume observer dropped"))
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

async fn create_waiting_client_tool_session(server: &TestServer, suffix: &str) -> Session {
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("parked-race-agent-{suffix}"),
                "display_name": "Parked Race Agent",
                "description": "Agent for parked-turn race coverage",
                "system_prompt": "Use client-side tools."
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
            1,
            session.id,
            everruns_server::storage::models::UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update session status")
        .expect("session exists");
    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id: session.id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({
                "tool_calls": [{
                    "id": format!("call_{suffix}"),
                    "name": "ask_user",
                    "arguments": {}
                }]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit pending client tool call");
    session
}

async fn abandon_message_resolution(server: &TestServer, session_id: SessionId) {
    let result = server
        .db
        .reserve_active_turn_slot_for_org(
            1,
            session_id,
            1,
            WaitingTurnResolutionPlan {
                kind: "user_message".to_string(),
                events: Vec::new(),
                session_values: Vec::new(),
                response: json!({}),
            },
        )
        .await
        .expect("reserve message resolution");
    let claim = match result {
        ReserveActiveTurnSlotResult::Accepted {
            resolution_claim: Some(claim),
            ..
        } => claim,
        other => panic!("expected parked-turn claim, got {other:?}"),
    };
    server
        .db
        .abandon_waiting_turn_claim(1, session_id, claim.resolution_id, claim.claim_token)
        .await
        .expect("expire message resolution claim");
}

#[tokio::test]
async fn expired_message_resolution_rejects_tool_results_without_acknowledging_them() {
    let server = TestServer::in_memory().await;
    let session = create_waiting_client_tool_session(&server, "stale-message-result").await;
    abandon_message_resolution(&server, session.id).await;

    server
        .post(
            &format!("/v1/sessions/{}/tool-results", session.id),
            json!({
                "tool_results": [{
                    "tool_call_id": "call_stale-message-result",
                    "result": { "answer": "discarded" }
                }]
            }),
        )
        .await
        .assert_status(StatusCode::CONFLICT);

    let completions = server
        .db
        .list_events(
            session.id,
            None,
            None,
            &["tool.completed".to_string()],
            &[],
            None,
            None,
        )
        .await
        .expect("list completions");
    assert!(completions.is_empty());
}

#[tokio::test]
async fn omitted_ask_user_question_ids_resume_through_tool_results() {
    let (resume_tx, mut resume_rx) = tokio::sync::mpsc::unbounded_channel();
    let server = TestServer::in_memory_with_runner(Arc::new(RecordingRunner {
        resumed_sessions: resume_tx,
    }))
    .await;
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "ask-user-test-agent",
                "display_name": "Ask User Test",
                "description": "Agent for ask_user integration coverage",
                "system_prompt": "Ask structured questions."
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
            1,
            session.id,
            everruns_server::storage::models::UpdateSession {
                status: Some("waiting_for_tool_results".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update session status")
        .expect("session exists");

    let normalized = normalize_ask_user_arguments(&json!({
        "questions": [{
            "header": "Target",
            "question": "Where should I deploy?",
            "options": [
                {"label": "Staging", "description": "Safe and reversible."},
                {"label": "Production", "description": "Serves live traffic."}
            ]
        }]
    }))
    .expect("valid ask_user arguments");
    assert_eq!(normalized["questions"][0]["id"], "question_1");
    server
        .db
        .create_event(everruns_server::storage::models::CreateEventRow {
            session_id: session.id,
            event_type: "tool.call_requested".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: json!({
                "tool_calls": [{
                    "id": "call_ask_user",
                    "name": "ask_user",
                    "arguments": normalized
                }]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit normalized ask_user call");

    let response = server
        .post(
            &format!("/v1/sessions/{}/tool-results", session.id),
            json!({
                "tool_results": [{
                    "tool_call_id": "call_ask_user",
                    "result": {
                        "status": "answered",
                        "answered_by": "user",
                        "answers": [{
                            "id": "question_1",
                            "selected": ["Staging"],
                            "other_text": null
                        }]
                    }
                }]
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(response["status"], "active");

    let events = server
        .db
        .list_events(
            session.id,
            None,
            None,
            &["tool.completed".to_string()],
            &[],
            None,
            Some(10),
        )
        .await
        .expect("list tool completion events");
    let completed = events.last().expect("tool result persisted");
    assert_eq!(completed.data["tool_call_id"], "call_ask_user");
    let resumed_session = tokio::time::timeout(Duration::from_secs(1), resume_rx.recv())
        .await
        .expect("resume signal timeout")
        .expect("resume signal");
    assert_eq!(resumed_session, session.id);
}

#[tokio::test]
async fn user_message_cancels_pending_client_tool_and_resumes_existing_turn() {
    let (resume_tx, mut resume_rx) = tokio::sync::mpsc::unbounded_channel();
    let server = TestServer::in_memory_with_runner(Arc::new(RecordingRunner {
        resumed_sessions: resume_tx,
    }))
    .await;
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "parked-turn-message-agent",
                "display_name": "Parked Turn Message",
                "description": "Agent for parked client-side tool coverage",
                "system_prompt": "Use client-side tools."
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    for tool_name in ["ask_user", "confirm_url_elicitation", "setup_connection"] {
        let session: Session = server
            .post("/v1/sessions", json!({ "agent_id": agent.public_id }))
            .await
            .assert_status(StatusCode::CREATED)
            .json();
        server
            .db
            .update_session(
                1,
                session.id,
                everruns_server::storage::models::UpdateSession {
                    status: Some("waiting_for_tool_results".to_string()),
                    ..Default::default()
                },
            )
            .await
            .expect("update session status")
            .expect("session exists");
        let tool_call_id = format!("call_{tool_name}");
        server
            .db
            .create_event(everruns_server::storage::models::CreateEventRow {
                session_id: session.id,
                event_type: "tool.call_requested".to_string(),
                ts: chrono::Utc::now(),
                context: json!({}),
                data: json!({
                    "tool_calls": [{
                        "id": tool_call_id,
                        "name": tool_name,
                        "arguments": {}
                    }]
                }),
                metadata: None,
                tags: None,
            })
            .await
            .expect("emit pending client tool call");

        server
            .post(
                &format!("/v1/sessions/{}/messages", session.id),
                json!({
                    "message": {
                        "role": "user",
                        "content": [{
                            "type": "text",
                            "text": format!("typed answer for {tool_name}")
                        }]
                    }
                }),
            )
            .await
            .assert_status(StatusCode::CREATED);

        let events = server
            .db
            .list_events(session.id, None, None, &[], &[], None, Some(20))
            .await
            .expect("list session events");
        let cancellation = events
            .iter()
            .find(|event| {
                event.event_type == "tool.completed" && event.data["tool_call_id"] == tool_call_id
            })
            .expect("pending client tool call is cancelled");
        assert_eq!(cancellation.data["status"], "cancelled");
        assert_eq!(cancellation.data["tool_name"], tool_name);
        assert!(events.iter().any(|event| {
            event.event_type == "input.message"
                && event.data["message"]["content"][0]["text"]
                    == format!("typed answer for {tool_name}")
        }));

        let resumed_session = tokio::time::timeout(Duration::from_secs(1), resume_rx.recv())
            .await
            .expect("resume signal timeout")
            .expect("resume signal");
        assert_eq!(resumed_session, session.id);

        server
            .post(
                &format!("/v1/sessions/{}/tool-results", session.id),
                json!({
                    "tool_results": [{
                        "tool_call_id": tool_call_id,
                        "result": {"status": "answered"}
                    }]
                }),
            )
            .await
            .assert_status(StatusCode::CONFLICT);
    }
}
