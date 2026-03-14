//! API Integration tests for Everruns using in-process server with PostgreSQL
//!
//! These tests run against a real PostgreSQL database but don't require
//! a running server process - the server routes are tested in-process
//! using tower's oneshot method.
//!
//! Run with: cargo test -p everruns-server --test api_integration_test -- --test-threads=1
//!
//! Requirements:
//! - PostgreSQL running with DATABASE_URL set
//! - Migrations applied (run migrations from crates/server/migrations/)

mod test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_core::llm_models::LlmProvider;
use everruns_core::{Agent, Harness, LlmModel, Session, SessionFile};

/// Seed harness ID from seed.rs (BASE_HARNESS = 0x01933b5a_0000_7000_8000_000000000601)
const SEED_BASE_HARNESS_ID: &str = "harness_01933b5a000070008000000000000601";
/// Seed harness ID from seed.rs (GENERIC_HARNESS = 0x01933b5a_0000_7000_8000_000000000602)
const SEED_GENERIC_HARNESS_ID: &str = "harness_01933b5a000070008000000000000602";
/// Seed harness ID from seed.rs (CHAT_HARNESS = 0x01933b5a_0000_7000_8000_000000000603)
const SEED_CHAT_HARNESS_ID: &str = "harness_01933b5a000070008000000000000603";

// ============================================
// Health Endpoint Tests
// ============================================

#[tokio::test]
async fn test_health_endpoint() {
    let server = TestServer::new().await;

    let body: Value = server
        .get("/health")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["status"], "ok");
}

// ============================================
// Feature Flags Endpoint Tests
// ============================================

#[tokio::test]
async fn test_feature_flags_endpoint() {
    let server = TestServer::new().await;

    let body: Value = server
        .get("/v1/feature-flags")
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Should return a JSON object with boolean flags
    assert!(body.is_object());
    assert!(body.get("global_chat").is_some());
    assert!(body.get("notifications").is_some());
    // In test env (DEV_MODE=true), experimental flags are enabled
    assert!(body["global_chat"].is_boolean());
    assert_eq!(body["notifications"], Value::Bool(false));
}

#[tokio::test]
async fn test_notifications_routes_disabled_by_default() {
    let server = TestServer::new().await;

    server
        .get("/v1/notifications")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ============================================
// Agent CRUD Tests
// ============================================

#[tokio::test]
async fn test_create_agent() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Test Agent",
                "description": "An agent for testing",
                "system_prompt": "You are a helpful assistant"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(agent.name, "Test Agent");
    assert_eq!(agent.description.as_deref(), Some("An agent for testing"));
}

#[tokio::test]
async fn test_list_agents() {
    let server = TestServer::new().await;

    // Create an agent first
    let _: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "List Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // List agents
    let data: Value = server
        .get("/v1/agents")
        .await
        .assert_status(StatusCode::OK)
        .json();

    let agents = data["data"].as_array().expect("Expected array");
    assert!(!agents.is_empty(), "Should have at least one agent");
}

#[tokio::test]
async fn test_get_agent_by_id() {
    let server = TestServer::new().await;

    // Create an agent
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Get Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Get the agent by ID
    let fetched_agent: Agent = server
        .get(&format!("/v1/agents/{}", agent.public_id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(fetched_agent.public_id, agent.public_id);
    assert_eq!(fetched_agent.name, "Get Test Agent");
}

#[tokio::test]
async fn test_update_agent() {
    let server = TestServer::new().await;

    // Create an agent
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Original Name",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Update the agent
    let updated_agent: Agent = server
        .patch(
            &format!("/v1/agents/{}", agent.public_id),
            json!({
                "name": "Updated Name",
                "description": "Updated description"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(updated_agent.name, "Updated Name");
    assert_eq!(
        updated_agent.description.as_deref(),
        Some("Updated description")
    );
}

#[tokio::test]
async fn test_create_agent_missing_default_model_returns_not_found() {
    let server = TestServer::in_memory().await;

    server
        .post(
            "/v1/agents",
            json!({
                "name": "Missing Model Agent",
                "system_prompt": "Test",
                "default_model_id": "model_019563a3000070008000000000000001"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_agent_missing_default_model_returns_not_found() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Update Missing Model Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .patch(
            &format!("/v1/agents/{}", agent.public_id),
            json!({
                "default_model_id": "model_019563a3000070008000000000000002"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_delete_agent() {
    let server = TestServer::new().await;

    // Create an agent
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Delete Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Delete the agent (soft-delete: archives the agent)
    server
        .delete(&format!("/v1/agents/{}", agent.public_id))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    // Verify agent is archived (not hard-deleted)
    let archived_agent: Agent = server
        .get(&format!("/v1/agents/{}", agent.public_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(archived_agent.status, everruns_core::AgentStatus::Archived);

    let default_list: Value = server
        .get("/v1/agents")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let listed = default_list["data"]
        .as_array()
        .expect("Expected agents array")
        .iter()
        .any(|candidate| candidate["id"] == agent.public_id.to_string());
    assert!(!listed, "Archived agent should be hidden from default list");

    let archived_list: Value = server
        .get("/v1/agents?include_archived=true")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let listed = archived_list["data"]
        .as_array()
        .expect("Expected agents array")
        .iter()
        .any(|candidate| candidate["id"] == agent.public_id.to_string());
    assert!(
        listed,
        "Archived agent should appear when include_archived=true"
    );
}

#[tokio::test]
async fn test_destroy_agent_requires_archive_and_hides_detail_api() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Destroy Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(&format!("/v1/agents/{}/delete", agent.public_id), json!({}))
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    server
        .delete(&format!("/v1/agents/{}", agent.public_id))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    server
        .post(&format!("/v1/agents/{}/delete", agent.public_id), json!({}))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    server
        .get(&format!("/v1/agents/{}", agent.public_id))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_session_rejects_archived_agent() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Archived Session Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .delete(&format!("/v1/agents/{}", agent.public_id))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id,
                "title": "Should Fail"
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

// ============================================
// Session CRUD Tests
// ============================================

#[tokio::test]
async fn test_create_session() {
    let server = TestServer::new().await;

    // Create an agent first
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Session Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a session
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id,
                "title": "Test Session",
                "locale": "uk-UA"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(session.agent_id, Some(agent.public_id));
    assert_eq!(session.title.as_deref(), Some("Test Session"));
    assert_eq!(session.locale.as_deref(), Some("uk-UA"));

    let fetched: Session = server
        .get(&format!("/v1/sessions/{}", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(fetched.locale.as_deref(), Some("uk-UA"));
}

#[tokio::test]
async fn test_create_session_nonexistent_harness_returns_404() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": "harness_00000000000000000000000000000000",
                "title": "Should fail"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_session_nonexistent_model_returns_404() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "model_id": "model_00000000000000000000000000000000",
                "title": "Should fail"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_session_nonexistent_agent_returns_404() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": "agent_00000000000000000000000000000000",
                "title": "Should fail"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_get_session() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Get Session Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id,
                "title": "Get Test Session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Get the session
    let fetched_session: Session = server
        .get(&format!("/v1/sessions/{}", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(fetched_session.id, session.id);
}

#[tokio::test]
async fn test_sessions_pagination() {
    let server = TestServer::new().await;

    // Create an agent
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Pagination Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create 15 sessions
    for i in 1..=15 {
        let _: Session = server
            .post(
                "/v1/sessions",
                json!({
                    "harness_id": SEED_BASE_HARNESS_ID,
                    "agent_id": agent.public_id,
                    "title": format!("Session {}", i)
                }),
            )
            .await
            .assert_status(StatusCode::CREATED)
            .json();
    }

    // Test default pagination
    let body: Value = server
        .get(&format!("/v1/sessions?agent_id={}", agent.public_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["total"], 15);
    assert_eq!(body["offset"], 0);

    // Test custom limit
    let body: Value = server
        .get(&format!(
            "/v1/sessions?agent_id={}&limit=5",
            agent.public_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["limit"], 5);
    assert_eq!(body["data"].as_array().unwrap().len(), 5);

    // Test offset pagination
    let body: Value = server
        .get(&format!(
            "/v1/sessions?agent_id={}&offset=10&limit=10",
            agent.public_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["data"].as_array().unwrap().len(), 5); // Only 5 remaining
}

// ============================================
// Message Tests
// ============================================

#[tokio::test]
async fn test_create_user_message() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Message Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a user message
    let message: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello!"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(message["role"], "user");
}

#[tokio::test]
async fn test_list_messages() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "List Messages Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a message
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello!"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // List messages
    let data: Value = server
        .get(&format!("/v1/sessions/{}/messages", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let messages = data["data"].as_array().expect("Expected array");
    assert_eq!(messages.len(), 1);
}

// ============================================
// Events Tests
// ============================================

#[tokio::test]
async fn test_list_events() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Events Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a message (which generates events)
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Hello!"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // List events
    let data: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Events should be present (at least the user message event)
    assert!(data["data"].is_array());
}

// ============================================
// LLM Provider Tests
// ============================================

#[tokio::test]
async fn test_llm_provider_crud() {
    let server = TestServer::new().await;

    // Create a provider
    let provider: LlmProvider = server
        .post(
            "/v1/llm-providers",
            json!({
                "name": "Test OpenAI Provider",
                "provider_type": "openai",
                "base_url": "https://api.openai.com/v1",
                "installed": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(provider.name, "Test OpenAI Provider");

    // List providers
    server
        .get("/v1/llm-providers")
        .await
        .assert_status(StatusCode::OK);

    // Delete provider
    server
        .delete(&format!("/v1/llm-providers/{}", provider.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_llm_model_crud() {
    let server = TestServer::new().await;

    // Create a provider first
    let provider: LlmProvider = server
        .post(
            "/v1/llm-providers",
            json!({
                "name": "Model Test Provider",
                "provider_type": "openai"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a model
    let model: LlmModel = server
        .post(
            &format!("/v1/llm-providers/{}/models", provider.id),
            json!({
                "model_id": "gpt-4-test",
                "display_name": "GPT-4 Test",
                "capabilities": ["chat"],
                "installed": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(model.model_id, "gpt-4-test");

    // List all models
    server
        .get("/v1/llm-models")
        .await
        .assert_status(StatusCode::OK);

    // Cleanup
    server
        .delete(&format!("/v1/llm-models/{}", model.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .delete(&format!("/v1/llm-providers/{}", provider.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

// ============================================
// Session Model Inheritance Tests
// ============================================

#[tokio::test]
async fn test_session_inherits_agent_default_model() {
    let server = TestServer::new().await;

    // Create provider and model
    let provider: LlmProvider = server
        .post(
            "/v1/llm-providers",
            json!({
                "name": "Inheritance Test Provider",
                "provider_type": "openai"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let model: LlmModel = server
        .post(
            &format!("/v1/llm-providers/{}/models", provider.id),
            json!({
                "model_id": "inherit-test-model",
                "display_name": "Inherit Test Model",
                "installed": false
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create agent with default_model_id
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Model Inheritance Agent",
                "system_prompt": "Test",
                "default_model_id": model.id.to_string()
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(agent.default_model_id, Some(model.id));

    // Create session without specifying model_id
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id,
                "title": "Inheritance Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Session should inherit agent's default_model_id
    assert_eq!(
        session.model_id,
        Some(model.id),
        "Session should inherit agent's default_model_id"
    );

    // Cleanup in correct order: session -> agent -> model -> provider
    server.delete(&format!("/v1/sessions/{}", session.id)).await;
    server
        .delete(&format!("/v1/agents/{}", agent.public_id))
        .await;
    server.delete(&format!("/v1/llm-models/{}", model.id)).await;
    server
        .delete(&format!("/v1/llm-providers/{}", provider.id))
        .await;
}

// ============================================
// Session Filesystem Tests
// ============================================

#[tokio::test]
async fn test_session_filesystem() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Filesystem Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // List root (should be empty)
    let data: Value = server
        .get(&fs_url)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(data["data"].as_array().unwrap().len(), 0);

    // Create a file
    let file: SessionFile = server
        .post(
            &format!("{}/hello.txt", fs_url),
            json!({
                "content": "Hello, World!",
                "encoding": "text"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(file.path, "/hello.txt");

    // Read the file
    let file: SessionFile = server
        .get(&format!("{}/hello.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(file.content.as_deref(), Some("Hello, World!"));

    // Update the file
    let file: SessionFile = server
        .put(
            &format!("{}/hello.txt", fs_url),
            json!({
                "content": "Updated content"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(file.content.as_deref(), Some("Updated content"));

    // Create directory
    let dir: SessionFile = server
        .post(
            &format!("{}/docs", fs_url),
            json!({
                "is_directory": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert!(dir.is_directory);

    // Delete file
    let result: Value = server
        .delete(&format!("{}/hello.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(result["deleted"], true);
}

#[tokio::test]
async fn test_session_filesystem_list_nonexistent_directory_returns_404() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "FS 404 Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // List a directory that doesn't exist — should return 404, not 500
    let resp = server.get(&format!("{}/.agents/skills", fs_url)).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ============================================
// Harness Tests
// ============================================

#[tokio::test]
async fn test_list_harnesses_includes_base_and_generic() {
    let server = TestServer::new().await;

    let data: Value = server
        .get("/v1/harnesses")
        .await
        .assert_status(StatusCode::OK)
        .json();

    let harnesses = data["data"].as_array().expect("Expected array");
    assert!(
        harnesses.len() >= 2,
        "Should have at least Base and Generic harnesses"
    );

    let names: Vec<&str> = harnesses
        .iter()
        .filter_map(|h| h["name"].as_str())
        .collect();
    assert!(names.contains(&"Base"), "Should have Base harness");
    assert!(names.contains(&"Generic"), "Should have Generic harness");
}

#[tokio::test]
async fn test_get_base_harness() {
    let server = TestServer::new().await;

    let harness: Harness = server
        .get(&format!("/v1/harnesses/{}", SEED_BASE_HARNESS_ID))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(harness.name, "Base");
    assert!(
        harness.capabilities.is_empty(),
        "Base harness should have no capabilities"
    );
    assert!(harness.tags.contains(&"base".to_string()));
    assert!(harness.tags.contains(&"built-in".to_string()));
}

#[tokio::test]
async fn test_get_generic_harness() {
    let server = TestServer::new().await;

    let harness: Harness = server
        .get(&format!("/v1/harnesses/{}", SEED_GENERIC_HARNESS_ID))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(harness.name, "Generic");
    assert!(harness.tags.contains(&"generic".to_string()));
    assert!(harness.tags.contains(&"default".to_string()));

    // Verify Generic harness has the expected built-in defaults
    let cap_ids: Vec<&str> = harness
        .capabilities
        .iter()
        .map(|c| c.capability_id())
        .collect();
    assert_eq!(
        cap_ids.len(),
        9,
        "Generic harness should have 9 capabilities"
    );
    assert!(
        cap_ids.contains(&"session_file_system"),
        "Should have file system"
    );
    assert!(
        cap_ids.contains(&"virtual_bash"),
        "Should have virtual bash"
    );
    assert!(cap_ids.contains(&"web_fetch"), "Should have web fetch");
    assert!(
        cap_ids.contains(&"session_storage"),
        "Should have session storage"
    );
    assert!(
        cap_ids.contains(&"session"),
        "Should have session capability"
    );
    assert!(
        cap_ids.contains(&"agent_instructions"),
        "Should have agent instructions"
    );
    assert!(cap_ids.contains(&"skills"), "Should have skills discovery");
    assert!(
        cap_ids.contains(&"infinity_context"),
        "Should have infinity context"
    );
    assert!(
        cap_ids.contains(&"openai_tool_search"),
        "Should have OpenAI tool search"
    );
}

#[tokio::test]
async fn test_create_session_with_generic_harness() {
    let server = TestServer::new().await;

    // Create an agent
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Generic Harness Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create session with Generic harness
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_GENERIC_HARNESS_ID,
                "agent_id": agent.public_id,
                "title": "Generic Harness Session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(session.title.as_deref(), Some("Generic Harness Session"));
}

// ============================================
// Copy Agent Tests
// ============================================

#[tokio::test]
async fn test_copy_agent() {
    let server = TestServer::new().await;

    // Create an agent with capabilities and tags
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Original Agent",
                "description": "Original description",
                "system_prompt": "You are helpful",
                "tags": ["tag1", "tag2"],
                "capabilities": [
                    {"ref": "current_time", "config": {}}
                ]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Copy the agent
    let copied: Agent = server
        .post(&format!("/v1/agents/{}/copy", agent.public_id), json!({}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Verify copy
    assert_eq!(copied.name, "Original Agent (copy)");
    assert_eq!(copied.description.as_deref(), Some("Original description"));
    assert_eq!(copied.system_prompt, "You are helpful");
    assert_eq!(copied.tags, vec!["tag1", "tag2"]);
    assert_eq!(copied.capabilities.len(), 1);
    assert_eq!(copied.capabilities[0].capability_id(), "current_time");
    // New ID
    assert_ne!(copied.public_id, agent.public_id);
}

#[tokio::test]
async fn test_copy_agent_not_found() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/agents/agent_00000000000000000000000000000099/copy",
            json!({}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ============================================
// Copy Harness Tests
// ============================================

#[tokio::test]
async fn test_copy_harness() {
    let server = TestServer::new().await;

    // Create a harness with capabilities and tags
    let harness: Harness = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "Original Harness",
                "description": "Original harness description",
                "system_prompt": "Harness prompt",
                "tags": ["harness-tag"],
                "capabilities": [
                    {"ref": "current_time", "config": {}}
                ]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Copy the harness
    let copied: Harness = server
        .post(&format!("/v1/harnesses/{}/copy", harness.id), json!({}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Verify copy
    assert_eq!(copied.name, "Original Harness (copy)");
    assert_eq!(
        copied.description.as_deref(),
        Some("Original harness description")
    );
    assert_eq!(copied.system_prompt, "Harness prompt");
    assert_eq!(copied.tags, vec!["harness-tag"]);
    assert_eq!(copied.capabilities.len(), 1);
    assert_eq!(copied.capabilities[0].capability_id(), "current_time");
    // New ID
    assert_ne!(copied.id, harness.id);
}

#[tokio::test]
async fn test_create_harness_missing_default_model_returns_not_found() {
    let server = TestServer::in_memory().await;

    server
        .post(
            "/v1/harnesses",
            json!({
                "name": "Missing Model Harness",
                "system_prompt": "Test",
                "default_model_id": "model_019563a3000070008000000000000003"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_harness_missing_default_model_returns_not_found() {
    let server = TestServer::in_memory().await;

    let harness: Harness = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "Update Missing Model Harness",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .patch(
            &format!("/v1/harnesses/{}", harness.id),
            json!({
                "default_model_id": "model_019563a3000070008000000000000004"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_copy_harness_not_found() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/harnesses/harness_00000000000000000000000000000099/copy",
            json!({}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_copy_seed_generic_harness() {
    let server = TestServer::new().await;

    // Copy the seed Generic harness
    let copied: Harness = server
        .post(
            &format!("/v1/harnesses/{}/copy", SEED_GENERIC_HARNESS_ID),
            json!({}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(copied.name, "Generic (copy)");
    // Generic harness capabilities should be preserved on copy
    assert_eq!(
        copied.capabilities.len(),
        9,
        "Copied harness should have same 9 capabilities"
    );
}

// ============================================
// Capabilities Tests
// ============================================

#[tokio::test]
async fn test_list_capabilities() {
    let server = TestServer::new().await;

    let data: Value = server
        .get("/v1/capabilities")
        .await
        .assert_status(StatusCode::OK)
        .json();

    let capabilities = data["data"].as_array().expect("Expected array");
    // Should have built-in capabilities
    assert!(
        !capabilities.is_empty(),
        "Should have built-in capabilities"
    );
    assert!(
        capabilities
            .iter()
            .any(|cap| cap["id"] == "infinity_context"),
        "Should expose infinity_context as a standard capability"
    );
}

#[tokio::test]
async fn test_agent_with_capabilities() {
    let server = TestServer::new().await;

    // Create agent with capabilities
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Capability Test Agent",
                "system_prompt": "Test",
                "capabilities": [
                    {"ref": "current_time", "config": {}},
                    {"ref": "session_file_system", "config": {}}
                ]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(
        agent.capabilities.len(),
        2,
        "Agent should have 2 capabilities"
    );
}

// ============================================
// Session SQL Database Tests
// ============================================

#[tokio::test]
async fn test_session_databases_crud() {
    let server = TestServer::new().await;

    // Create agent + session for the test
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "SQL Test Agent",
                "system_prompt": "Test",
                "capabilities": [{"ref": "session_sql_database", "config": {}}]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id.to_string()
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = session.id.to_string();

    // List databases (should be empty)
    let list: Value = server
        .get(&format!("/v1/sessions/{session_id}/databases"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(list["data"].as_array().unwrap().len(), 0);

    // Create a database
    let db_info: Value = server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "analytics"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(db_info["name"], "analytics");
    assert_eq!(db_info["size_bytes"], 0);

    // Get database
    let db_info: Value = server
        .get(&format!("/v1/sessions/{session_id}/databases/analytics"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(db_info["name"], "analytics");

    // List databases (should have 1)
    let list: Value = server
        .get(&format!("/v1/sessions/{session_id}/databases"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(list["data"].as_array().unwrap().len(), 1);

    // Create duplicate (should conflict)
    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "analytics"}),
        )
        .await
        .assert_status(StatusCode::CONFLICT);

    // Get nonexistent database (should 404)
    server
        .get(&format!("/v1/sessions/{session_id}/databases/nope"))
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // Delete database
    server
        .delete(&format!("/v1/sessions/{session_id}/databases/analytics"))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    // Delete again (should 404)
    server
        .delete(&format!("/v1/sessions/{session_id}/databases/analytics"))
        .await
        .assert_status(StatusCode::NOT_FOUND);

    // List should be empty again
    let list: Value = server
        .get(&format!("/v1/sessions/{session_id}/databases"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(list["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_session_databases_schema() {
    let server = TestServer::new().await;

    // Create agent + session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "SQL Schema Agent",
                "system_prompt": "Test",
                "capabilities": [{"ref": "session_sql_database", "config": {}}]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": SEED_BASE_HARNESS_ID, "agent_id": agent.public_id.to_string()}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = session.id.to_string();

    // Create a database
    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "test_db"}),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // Schema of empty database should have no tables
    let schema: Value = server
        .get(&format!(
            "/v1/sessions/{session_id}/databases/test_db/schema"
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(schema["database"], "test_db");
    assert_eq!(schema["tables"].as_array().unwrap().len(), 0);

    // Schema of nonexistent database should 404
    server
        .get(&format!("/v1/sessions/{session_id}/databases/nope/schema"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_session_databases_invalid_name() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "SQL Invalid Name Agent",
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": SEED_BASE_HARNESS_ID, "agent_id": agent.public_id.to_string()}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = session.id.to_string();

    // Invalid name (starts with number)
    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "1bad"}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // Invalid name (contains dash)
    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({"name": "my-db"}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

// ============================================================================
// Session Features Tests
// ============================================================================

#[tokio::test]
async fn test_session_features_base_harness_empty() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "Base Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Base harness has no capabilities → no features
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(
        session.features.is_empty(),
        "Base harness session should have no features, got: {:?}",
        session.features,
    );
}

#[tokio::test]
async fn test_session_features_generic_harness() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "Generic Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Generic harness has session_file_system, virtual_bash, session_storage, etc.
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_GENERIC_HARNESS_ID,
                "agent_id": agent.public_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(
        session.features.contains(&"file_system".to_string()),
        "Generic harness should include file_system feature, got: {:?}",
        session.features,
    );
    assert!(
        session.features.contains(&"secrets".to_string()),
        "Generic harness should include secrets feature, got: {:?}",
        session.features,
    );
    assert!(
        session.features.contains(&"key_value".to_string()),
        "Generic harness should include key_value feature, got: {:?}",
        session.features,
    );
    // file_system should only appear once despite session_file_system + virtual_bash
    let fs_count = session
        .features
        .iter()
        .filter(|f| *f == "file_system")
        .count();
    assert_eq!(fs_count, 1, "file_system should appear exactly once");
}

#[tokio::test]
async fn test_session_features_persisted_in_get() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "Get Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_GENERIC_HARNESS_ID,
                "agent_id": agent.public_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Features should also appear in GET response
    let fetched: Session = server
        .get(&format!("/v1/sessions/{}", session.id))
        .await
        .assert_success()
        .json();

    assert_eq!(
        fetched.features, session.features,
        "GET session should return same features as POST",
    );
}

#[tokio::test]
async fn test_session_features_in_list() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "List Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let _session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_GENERIC_HARNESS_ID,
                "agent_id": agent.public_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Features should appear in list response
    let list_body: Value = server
        .get(&format!("/v1/sessions?agent_id={}", agent.public_id))
        .await
        .assert_success()
        .json_value();

    let data = list_body["data"].as_array().expect("data array");
    assert!(!data.is_empty());

    let first = &data[0];
    let features = first["features"]
        .as_array()
        .expect("features should be an array");
    assert!(
        features.contains(&json!("file_system")),
        "List should include file_system feature",
    );
}

#[tokio::test]
async fn test_session_features_with_session_capabilities() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "Session Cap Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Base harness has no caps, but add session_schedule at session level
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id,
                "capabilities": [{"ref": "session_schedule"}],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(
        session.features.contains(&"schedules".to_string()),
        "Session-level capability should contribute features, got: {:?}",
        session.features,
    );
}

#[tokio::test]
async fn test_session_features_with_agent_capabilities() {
    let server = TestServer::new().await;

    // Create agent with session_schedule capability
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Agent Cap Features Agent",
                "system_prompt": "Test",
                "capabilities": [{"ref": "session_schedule"}],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Base harness (no caps) + agent has session_schedule
    let body: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();

    let features = body["features"]
        .as_array()
        .expect("features should be an array");
    assert!(
        features.contains(&json!("schedules")),
        "Agent capability should contribute features, got: {:?}",
        features,
    );
}

#[tokio::test]
async fn test_session_features_sql_database() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "SqlDb Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Add session_sql_database via session-level capability
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id,
                "capabilities": [{"ref": "session_sql_database"}],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(
        session.features.contains(&"sql_database".to_string()),
        "session_sql_database should contribute sql_database feature, got: {:?}",
        session.features,
    );
}

#[tokio::test]
async fn test_capability_info_includes_features() {
    let server = TestServer::new().await;

    // Check that the capabilities endpoint returns features
    let body: Value = server
        .get("/v1/capabilities")
        .await
        .assert_success()
        .json_value();

    let data = body["data"].as_array().expect("data array");

    // Find session_storage capability — should have secrets and key_value features
    let storage = data
        .iter()
        .find(|c| c["id"] == "session_storage")
        .expect("session_storage capability should exist");

    let features = storage["features"]
        .as_array()
        .expect("features should be an array");
    assert!(
        features.contains(&json!("secrets")),
        "session_storage should have secrets feature",
    );
    assert!(
        features.contains(&json!("key_value")),
        "session_storage should have key_value feature",
    );

    // Find session_schedule — should have schedules feature
    let schedule = data
        .iter()
        .find(|c| c["id"] == "session_schedule")
        .expect("session_schedule capability should exist");

    let schedule_features = schedule["features"]
        .as_array()
        .expect("features should be an array");
    assert!(
        schedule_features.contains(&json!("schedules")),
        "session_schedule should have schedules feature",
    );

    // Find noop — should have no features (empty or absent)
    let noop = data
        .iter()
        .find(|c| c["id"] == "noop")
        .expect("noop capability should exist");
    let noop_features = noop.get("features");
    assert!(
        noop_features
            .and_then(|v| v.as_array())
            .is_none_or(|a| a.is_empty()),
        "noop should have no features",
    );
}

// ============================================
// Global Chat Session Tests
// ============================================

#[tokio::test]
async fn test_global_chat_creates_session() {
    let server = TestServer::new().await;

    // First call should create a new chat session
    let session: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();

    assert_eq!(session.title.as_deref(), Some("Platform Chat"));
    assert!(session.tags.contains(&"global-chat".to_string()));
}

#[tokio::test]
async fn test_global_chat_returns_same_session() {
    let server = TestServer::new().await;

    // First call creates
    let first: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();

    // Second call returns the same session
    let second: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();

    assert_eq!(
        first.id, second.id,
        "Should return the same singleton session"
    );
}

#[tokio::test]
async fn test_global_chat_has_chat_harness() {
    let server = TestServer::new().await;

    let session: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();

    assert_eq!(
        session.harness_id.to_string(),
        SEED_CHAT_HARNESS_ID,
        "Chat session should use the Platform Chat harness"
    );
}

#[tokio::test]
async fn test_chat_harness_exists_in_seed() {
    let server = TestServer::new().await;

    // Verify the Platform Chat harness was seeded (response is {"data": [...]})
    let body = server
        .get("/v1/harnesses")
        .await
        .assert_success()
        .json_value();
    let harnesses: Vec<Harness> =
        serde_json::from_value(body["data"].clone()).expect("Failed to parse harnesses data");

    let chat_harness = harnesses
        .iter()
        .find(|h| h.name == "Platform Chat")
        .expect("Platform Chat harness should exist in seed data");

    assert_eq!(chat_harness.id.to_string(), SEED_CHAT_HARNESS_ID);
    assert!(chat_harness.tags.contains(&"chat".to_string()));
}

#[tokio::test]
async fn test_chat_harness_has_platform_management() {
    let server = TestServer::new().await;

    let harness: Harness = server
        .get(&format!("/v1/harnesses/{}", SEED_CHAT_HARNESS_ID))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(harness.name, "Platform Chat");

    let cap_ids: Vec<&str> = harness
        .capabilities
        .iter()
        .map(|c| c.capability_id())
        .collect();

    assert_eq!(
        cap_ids.len(),
        10,
        "Platform Chat harness should have 10 capabilities (Generic + platform_management)"
    );
    assert!(
        cap_ids.contains(&"infinity_context"),
        "Platform Chat harness should include infinity_context from Generic"
    );
    assert!(
        cap_ids.contains(&"platform_management"),
        "Platform Chat harness should include platform_management capability"
    );
}

// ============================================
// User Profile Tests
// ============================================

#[tokio::test]
async fn test_update_profile_name() {
    let server = TestServer::in_memory().await;

    // Update profile name
    let resp: Value = server
        .patch("/v1/users/me", json!({ "name": "New Display Name" }))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(resp["name"], "New Display Name");
    assert!(resp["id"].is_string());
    assert!(resp["email"].is_string());
}

#[tokio::test]
async fn test_update_profile_trims_whitespace() {
    let server = TestServer::in_memory().await;

    let resp: Value = server
        .patch("/v1/users/me", json!({ "name": "  Trimmed Name  " }))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(resp["name"], "Trimmed Name");
}

#[tokio::test]
async fn test_update_profile_rejects_empty_name() {
    let server = TestServer::in_memory().await;

    server
        .patch("/v1/users/me", json!({ "name": "" }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_profile_rejects_whitespace_only_name() {
    let server = TestServer::in_memory().await;

    server
        .patch("/v1/users/me", json!({ "name": "   " }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_update_profile_rejects_too_long_name() {
    let server = TestServer::in_memory().await;

    let long_name = "a".repeat(256);
    server
        .patch("/v1/users/me", json!({ "name": long_name }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

// ============================================
// Readonly File Deletion Tests
// ============================================

#[tokio::test]
async fn test_cannot_delete_readonly_file() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Readonly Delete Test",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // Create a readonly file
    let _file: SessionFile = server
        .post(
            &format!("{}/protected.txt", fs_url),
            json!({
                "content": "Do not delete me",
                "is_readonly": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Attempt to delete should fail with 403 Forbidden
    server
        .delete(&format!("{}/protected.txt", fs_url))
        .await
        .assert_status(StatusCode::FORBIDDEN);

    // File should still exist
    let file: SessionFile = server
        .get(&format!("{}/protected.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(file.content.as_deref(), Some("Do not delete me"));
}

#[tokio::test]
async fn test_cannot_recursively_delete_directory_with_readonly_file() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Readonly Recursive Delete Test",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // Create directory with a readonly file inside
    let _dir: SessionFile = server
        .post(&format!("{}/docs", fs_url), json!({ "is_directory": true }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let _file: SessionFile = server
        .post(
            &format!("{}/docs/readme.txt", fs_url),
            json!({
                "content": "Protected content",
                "is_readonly": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Recursive delete of parent directory should fail with 403 Forbidden
    server
        .delete(&format!("{}/docs?recursive=true", fs_url))
        .await
        .assert_status(StatusCode::FORBIDDEN);

    // Readonly file should still exist
    let file: SessionFile = server
        .get(&format!("{}/docs/readme.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(file.content.as_deref(), Some("Protected content"));
}

#[tokio::test]
async fn test_can_delete_non_readonly_file() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "Normal Delete Test",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // Create a normal (non-readonly) file
    let _file: SessionFile = server
        .post(
            &format!("{}/temp.txt", fs_url),
            json!({
                "content": "Delete me"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Should succeed
    let result: Value = server
        .delete(&format!("{}/temp.txt", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(result["deleted"], true);
}

// ============================================
// User Connection Verify Endpoint Tests
// ============================================

#[tokio::test]
async fn test_verify_connection_no_connection_returns_404() {
    let server = TestServer::in_memory().await;

    server
        .post("/v1/user/connections/nonexistent/verify", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_verify_connection_providers_listed() {
    let server = TestServer::in_memory().await;

    let resp: Value = server
        .get("/v1/user/connections/providers")
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Should be an array (may include plugin-registered providers)
    assert!(resp.is_array());
}

#[tokio::test]
async fn test_list_connections_initially_empty() {
    let server = TestServer::in_memory().await;

    let resp: Vec<Value> = server
        .get("/v1/user/connections")
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert!(resp.is_empty());
}
