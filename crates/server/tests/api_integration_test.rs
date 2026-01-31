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
use everruns_core::{Agent, LlmModel, Session, SessionFile};

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
        .get(&format!("/v1/agents/{}", agent.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(fetched_agent.id, agent.id);
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
            &format!("/v1/agents/{}", agent.id),
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

    // Delete the agent
    server
        .delete(&format!("/v1/agents/{}", agent.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    // Verify agent is deleted (should return 404)
    server
        .get(&format!("/v1/agents/{}", agent.id))
        .await
        .assert_status(StatusCode::NOT_FOUND);
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
                "agent_id": agent.id,
                "title": "Test Session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(session.agent_id, agent.id);
    assert_eq!(session.title.as_deref(), Some("Test Session"));
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
                "agent_id": agent.id,
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
                    "agent_id": agent.id,
                    "title": format!("Session {}", i)
                }),
            )
            .await
            .assert_status(StatusCode::CREATED)
            .json();
    }

    // Test default pagination
    let body: Value = server
        .get(&format!("/v1/sessions?agent_id={}", agent.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["total"], 15);
    assert_eq!(body["offset"], 0);

    // Test custom limit
    let body: Value = server
        .get(&format!("/v1/sessions?agent_id={}&limit=5", agent.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["limit"], 5);
    assert_eq!(body["data"].as_array().unwrap().len(), 5);

    // Test offset pagination
    let body: Value = server
        .get(&format!(
            "/v1/sessions?agent_id={}&offset=10&limit=10",
            agent.id
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
                "agent_id": agent.id
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
                "agent_id": agent.id
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
                "agent_id": agent.id
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
                "is_default": true
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
                "is_default": true
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
                "is_default": false
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
                "agent_id": agent.id,
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
    server.delete(&format!("/v1/agents/{}", agent.id)).await;
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
                "agent_id": agent.id
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
    server
        .delete(&format!("{}/hello.txt", fs_url))
        .await
        .assert_status(StatusCode::NO_CONTENT);
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
