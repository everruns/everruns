//! API integration tests: agents.

use crate::support::seed_archival_app;
use crate::test_harness;
use axum::http::StatusCode;
use everruns_core::DEFAULT_ORG_ID;
use everruns_platform::Agent;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use everruns_provider::typed_id::{AgentId, AgentIdentityId, HarnessId};
use everruns_server::storage::models::{
    CreateAgentRow, CreateMcpServerRow, UpdateOrganizationSettings,
};
use serde_json::{Value, json};
use test_harness::TestServer;

#[tokio::test]
async fn test_create_agent() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "test-agent",
                "display_name": "Test Agent",
                "description": "An agent for testing",
                "system_prompt": "You are a helpful assistant"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(agent.name, "test-agent");
    assert_eq!(agent.description.as_deref(), Some("An agent for testing"));
}

#[tokio::test]
async fn test_agent_mcp_credential_is_write_only_and_agent_scoped() {
    let server = TestServer::in_memory().await;
    let sentinel = "security-test-secret-must-not-be-returned";
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "credential-owner",
                "display_name": "Credential Owner",
                "system_prompt": "Use the attached test tool",
                "mcpServers": {
                    "visti-test": { "url": "https://example.com/mcp" }
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let other: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "credential-non-owner",
                "display_name": "Credential Non-owner",
                "system_prompt": "No credentials"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let binding: Value = server
        .post(
            &format!("/v1/agents/{}/credentials", agent.public_id),
            json!({
                "mcp_server_name": "visti-test",
                "tool_name": "visti_send",
                "parameter_name": "channel_key",
                "label": "Visti channel key"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(binding["configured"], false);
    assert!(binding.get("value").is_none());
    let binding_id = binding["id"].as_str().unwrap();

    let rotated: Value = server
        .put(
            &format!("/v1/agents/{}/credentials/{binding_id}", agent.public_id),
            json!({ "value": sentinel }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(rotated["configured"], true);
    assert!(!rotated.to_string().contains(sentinel));
    assert!(rotated.get("value").is_none());

    let listed: Value = server
        .get(&format!("/v1/agents/{}/credentials", agent.public_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(listed["data"][0]["configured"], true);
    assert!(!listed.to_string().contains(sentinel));
    let stored_agent = server
        .db
        .get_agent_by_public_id(DEFAULT_ORG_ID, &agent.public_id.to_string())
        .await
        .unwrap()
        .unwrap();
    let runtime_bindings =
        everruns_server::domains::agents::credentials::resolve_runtime_secret_bindings(
            server.db.as_ref(),
            server.encryption.as_deref(),
            DEFAULT_ORG_ID,
            Some(stored_agent.id),
            "visti-test",
            "https://example.com/mcp",
        )
        .await
        .unwrap();
    assert_eq!(
        runtime_bindings["visti_send"][0].value.as_deref(),
        Some(sentinel)
    );

    server
        .patch(
            &format!("/v1/agents/{}", agent.public_id),
            json!({
                "mcpServers": {
                    "visti-test": { "url": "https://example.org/replaced-mcp" }
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);
    let endpoint_changed: Value = server
        .post(
            &format!("/v1/agents/{}/credentials", agent.public_id),
            json!({
                "agent_id": agent.public_id,
                "mcp_server_name": "visti-test",
                "tool_name": "visti_send",
                "parameter_name": "channel_key",
                "label": "Visti channel key"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(endpoint_changed["id"], binding_id);
    assert_eq!(endpoint_changed["configured"], false);

    server
        .put(
            &format!("/v1/agents/{}/credentials/{binding_id}", other.public_id),
            json!({ "value": "attempted-cross-agent-replacement" }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);

    server
        .delete(&format!(
            "/v1/agents/{}/credentials/{binding_id}",
            agent.public_id
        ))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_agent_versions_snapshot_diff_default_and_session_capture() {
    // Feature flags are process-level env in this pilot; enable explicitly for
    // the in-process server before it computes route state.
    unsafe {
        std::env::set_var("FEATURE_AGENT_VERSIONS", "true");
    }
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "versioned-agent",
                "display_name": "Versioned Agent",
                "description": "An agent with versions",
                "system_prompt": "You are version one"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let first: Value = server
        .post(
            &format!("/v1/agents/{}/versions", agent.public_id),
            json!({
                "summary": "Initial saved prompt",
                "change_kind": "manual"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(first["version"], "0.1.0");

    let _: Agent = server
        .patch(
            &format!("/v1/agents/{}", agent.public_id),
            json!({
                "system_prompt": "You are version two"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    let second: Value = server
        .post(
            &format!("/v1/agents/{}/versions", agent.public_id),
            json!({
                "summary": "Prompt update",
                "change_kind": "patch"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(second["version"], "0.1.1");

    let diff: Value = server
        .get(&format!(
            "/v1/agents/{}/versions/{}/diff/{}",
            agent.public_id,
            first["id"].as_str().unwrap(),
            second["id"].as_str().unwrap()
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        diff["authored_diff"]["system_prompt"]["from"],
        "You are version one"
    );
    assert_eq!(
        diff["authored_diff"]["system_prompt"]["to"],
        "You are version two"
    );

    let updated_agent: Agent = server
        .post(
            &format!("/v1/agents/{}/versions/default", agent.public_id),
            json!({ "version_id": second["id"] }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        updated_agent.default_version_id.unwrap().to_string(),
        second["id"]
    );

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "agent_id": agent.public_id,
                "title": "Version capture"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(session.agent_version_id.unwrap().to_string(), second["id"]);
}

#[tokio::test]
async fn test_list_agents() {
    let server = TestServer::new().await;

    // Create an agent first
    let _: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "list-test-agent",
                "display_name": "List Test Agent",
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
async fn test_list_agents_resolves_explicit_inherited_and_missing_harnesses() {
    let server = TestServer::in_memory().await;
    let generic_id: everruns_provider::typed_id::HarnessId =
        server.seed_generic_harness_id.parse().unwrap();
    let base_id: everruns_provider::typed_id::HarnessId =
        server.seed_base_harness_id.parse().unwrap();

    let explicit: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "explicit-harness-card",
                "system_prompt": "Test",
                "harness_id": generic_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let inherited: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "inherited-harness-card",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Change the default after creation so the assertion proves list/session
    // resolution is dynamic rather than a read of the materialized harness_id.
    server
        .db
        .patch_organization_settings(
            everruns_core::DEFAULT_ORG_ID,
            UpdateOrganizationSettings {
                default_harness_id: everruns_durable::UpdateField::Set(base_id),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let missing_harness_id = everruns_provider::typed_id::HarnessId::new();
    let missing_agent = server
        .db
        .create_agent(
            everruns_core::DEFAULT_ORG_ID,
            CreateAgentRow {
                public_id: everruns_provider::typed_id::AgentId::new().to_string(),
                name: "missing-harness-card".to_string(),
                display_name: None,
                description: None,
                intro_markdown: None,
                short_description: None,
                starters: serde_json::json!([]),
                system_prompt: "Test".to_string(),
                default_model_id: None,
                harness_id: missing_harness_id,
                tags: vec![],
                initial_files: json!([]),
                tools: json!([]),
                mcp_servers: json!({}),
                network_access: None,
                max_iterations: None,
                parallel_tool_calls: None,
                is_built_in: false,
            },
        )
        .await
        .unwrap();

    let listed: Value = server
        .get("/v1/agents?limit=200")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let find = |id: &str| {
        listed["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|agent| agent["id"] == id)
            .unwrap()
    };

    let explicit_item = find(explicit["id"].as_str().unwrap());
    assert_eq!(
        explicit_item["effective_harness"]["id"],
        generic_id.to_string()
    );
    assert_eq!(
        explicit_item["effective_harness"]["display_name"],
        "Generic"
    );
    assert_eq!(explicit_item["effective_harness"]["source"], "explicit");
    assert_eq!(explicit_item["effective_harness"]["status"], "active");

    let inherited_item = find(inherited["id"].as_str().unwrap());
    assert_eq!(
        inherited_item["effective_harness"]["id"],
        base_id.to_string()
    );
    assert_eq!(
        inherited_item["effective_harness"]["source"],
        "organization_default"
    );

    let missing_item = find(&missing_agent.public_id);
    assert_eq!(
        missing_item["effective_harness"]["id"],
        missing_harness_id.to_string()
    );
    assert!(missing_item["effective_harness"]["name"].is_null());
    assert_eq!(missing_item["effective_harness"]["status"], "unresolved");

    server
        .db
        .delete_harness(everruns_core::DEFAULT_ORG_ID, generic_id)
        .await
        .unwrap();
    server
        .db
        .destroy_harness(everruns_core::DEFAULT_ORG_ID, generic_id)
        .await
        .unwrap();
    let relisted: Value = server
        .get("/v1/agents?limit=200")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let deleted_item = relisted["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["id"] == explicit["id"])
        .unwrap();
    assert_eq!(deleted_item["effective_harness"]["status"], "deleted");

    let session: everruns_platform::Session = server
        .post(
            "/v1/sessions",
            json!({ "agent_id": inherited["id"], "title": "Inherited harness proof" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(session.harness_id, base_id);
}

#[tokio::test]
async fn test_get_agent_by_id() {
    let server = TestServer::new().await;

    // Create an agent
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "get-test-agent",
                "display_name": "Get Test Agent",
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
    assert_eq!(fetched_agent.name, "get-test-agent");
}

#[tokio::test]
async fn test_update_agent() {
    let server = TestServer::new().await;

    // Create an agent
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "original-name",
                "display_name": "Original Name",
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
                "name": "updated-name",
                "display_name": "Updated Name",
                "description": "Updated description"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(updated_agent.name, "updated-name");
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
                "name": "missing-model-agent",
                "display_name": "Missing Model Agent",
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
                "name": "update-missing-model-agent",
                "display_name": "Update Missing Model Agent",
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
                "name": "delete-test-agent",
                "display_name": "Delete Test Agent",
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
    assert_eq!(
        archived_agent.status,
        everruns_platform::AgentStatus::Archived
    );

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
                "name": "destroy-test-agent",
                "display_name": "Destroy Test Agent",
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
                "name": "archived-session-agent",
                "display_name": "Archived Session Agent",
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
                "harness_id": server.seed_base_harness_id,
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
async fn test_create_session_nonexistent_agent_returns_404() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": "agent_00000000000000000000000000000000",
                "title": "Should fail"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_list_sessions_unknown_agent_returns_empty() {
    let server = TestServer::new().await;

    // Create a session so we know unfiltered results would be non-empty
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "test-agent-unknown",
                "display_name": "Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let _session: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Filter by nonexistent agent_id should return empty results, not unfiltered
    let body: Value = server
        .get("/v1/sessions?agent_id=agent_ffffffffffffffffffffffffffffffff")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(body["total"], 0);
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
    assert_eq!(body["offset"], 0);
    assert_eq!(body["limit"], 20);
}

// ============================================
// Message Tests
// ============================================

#[tokio::test]
async fn test_session_inherits_agent_default_model() {
    let server = TestServer::new().await;

    // Create provider and model
    let provider: Provider = server
        .post(
            "/v1/providers",
            json!({
                "name": "Inheritance Test Provider",
                "provider_type": "openai"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let model: Model = server
        .post(
            &format!("/v1/providers/{}/models", provider.id),
            json!({
                "model_id": "inherit-test-model",
                "display_name": "Inherit Test Model",
                "enabled": true
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
                "name": "model-inheritance-agent",
                "display_name": "Model Inheritance Agent",
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
                "harness_id": server.seed_base_harness_id,
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
    server.delete(&format!("/v1/models/{}", model.id)).await;
    server
        .delete(&format!("/v1/providers/{}", provider.id))
        .await;
}

// ============================================
// Session Filesystem Tests
// ============================================

#[tokio::test]
async fn test_copy_agent() {
    let server = TestServer::new().await;

    // Create an agent with capabilities and tags
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "original-agent",
                "display_name": "Original Agent",
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
    assert_eq!(copied.name, "original-agent-copy");
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
// Check Harness Name Tests
// ============================================

#[tokio::test]
async fn test_agent_with_capabilities() {
    let server = TestServer::new().await;

    // Create agent with capabilities
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "capability-test-agent",
                "display_name": "Capability Test Agent",
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
async fn test_session_features_with_agent_capabilities() {
    let server = TestServer::new().await;

    // Create agent with session_schedule capability
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "agent-cap-features-agent",
                "display_name": "Agent Cap Features Agent",
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
                "harness_id": server.seed_base_harness_id,
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
async fn test_delete_agent_referenced_by_app_returns_conflict() {
    let server = TestServer::in_memory().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "delete-app-agent",
                "display_name": "Delete App Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    seed_archival_app(
        &server,
        "Agent Delete Blocker",
        server.seed_generic_harness_id.parse::<HarnessId>().unwrap(),
        Some(agent["id"].as_str().unwrap().parse::<AgentId>().unwrap()),
        None,
    )
    .await;

    server
        .delete(&format!("/v1/agents/{}", agent["id"].as_str().unwrap()))
        .await
        .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_delete_agent_identity_referenced_by_app_returns_conflict() {
    let server = TestServer::in_memory().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "delete-identity-app-agent",
                "display_name": "Delete Identity App Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let identity: Value = server
        .post(
            "/v1/agent-identities",
            json!({"name": "Delete App Identity"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    seed_archival_app(
        &server,
        "Identity Delete Blocker",
        server.seed_generic_harness_id.parse::<HarnessId>().unwrap(),
        Some(agent["id"].as_str().unwrap().parse::<AgentId>().unwrap()),
        Some(
            identity["id"]
                .as_str()
                .unwrap()
                .parse::<AgentIdentityId>()
                .unwrap(),
        ),
    )
    .await;

    server
        .delete(&format!(
            "/v1/agent-identities/{}",
            identity["id"].as_str().unwrap()
        ))
        .await
        .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_agent_catalog_mcp_attachment_round_trip() {
    let server = TestServer::in_memory().await;
    server
        .db
        .create_mcp_server(
            DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: "linear-catalog".to_string(),
                description: None,
                url: "https://mcp.linear.app/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: None,
                settings: Some(json!({
                    "auth_mode": "oauth",
                    "oauth": {}
                })),
            },
        )
        .await
        .unwrap();

    let created: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "catalog-mcp-agent",
                "system_prompt": "Use the attached Linear tools",
                "mcpServers": {
                    "project-tracker": {
                        "use": "catalog:linear-catalog",
                        "actsAs": "service"
                    }
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(
        created["mcpServers"]["project-tracker"],
        json!({
            "use": "catalog:linear-catalog",
            "actsAs": "service"
        })
    );

    let fetched: Value = server
        .get(&format!("/v1/agents/{}", created["id"].as_str().unwrap()))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        fetched["mcpServers"]["project-tracker"],
        created["mcpServers"]["project-tracker"]
    );
}
