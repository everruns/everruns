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
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_core::{DEFAULT_ORG_ID, SessionContextReport, SessionFile};
use everruns_durable::UpdateField;
use everruns_platform::Agent;
use everruns_platform::Harness;
use everruns_platform::Session;
use everruns_provider::model::Model;
use everruns_provider::provider::Provider;
use everruns_provider::typed_id::PrincipalId;
use everruns_provider::typed_id::ScheduleId;
use everruns_server::storage::models::{
    CreateAgentRow, CreatePrincipalRow, CreateSessionScheduleRow, UpdateOrganizationSettings,
    UpdateSession,
};
use uuid::Uuid;

#[tokio::test]
async fn test_knowledge_index_create_enqueues_sync_and_rejects_chat_model() {
    let server = TestServer::in_memory().await;
    let models: Value = server
        .get("/v1/models")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let models = models["data"].as_array().expect("models list");
    let embedding_model = models
        .iter()
        .find(|model| model["model_id"] == "text-embedding-3-small")
        .expect("seeded embedding model");
    assert_eq!(embedding_model["capabilities"], json!(["embeddings"]));

    let created: Value = server
        .post(
            "/v1/knowledge-indexes",
            json!({
                "name": "Bashkit knowledge test",
                "source_type": "github",
                "source_config": {
                    "repository": "everruns/bashkit",
                    "branch": "main",
                    "root_folder": "knowledge"
                },
                "embedding_model_id": embedding_model["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(created["sync_status"], "pending");

    let chat_model = models
        .iter()
        .find(|model| model["model_id"] == "claude-opus-4-7[1m]")
        .or_else(|| {
            models
                .iter()
                .find(|model| model["model_id"] == "claude-opus-4-7")
        })
        .expect("seeded Claude chat model");
    let rejected: Value = server
        .post(
            "/v1/knowledge-indexes",
            json!({
                "name": "Invalid embedding model",
                "source_type": "github",
                "source_config": {"repository": "everruns/bashkit"},
                "embedding_model_id": chat_model["id"]
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert_eq!(
        rejected["detail"],
        "Embedding model is unavailable or does not support embeddings"
    );
}

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
    assert!(body.get("notifications").is_some());
    assert!(body.get("mcp_endpoint").is_none());
    assert_eq!(body["machine_payments"], Value::Bool(false));
    // In test env (DEV_MODE=true), experimental flags are enabled
    assert_eq!(body["notifications"], Value::Bool(false));
}

#[tokio::test]
async fn test_org_feature_flags_opt_in() {
    let server = TestServer::new().await;
    let org_id = "org_00000000000000000000000000000001";

    let settings: serde_json::Value = server
        .get(&format!("/v1/orgs/{org_id}/feature-flags/settings"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let flags = settings["flags"].as_array().expect("flags array");
    assert!(flags.iter().all(|flag| flag["name"] != "mcp_endpoint"));
    assert!(flags.iter().all(|flag| flag["label"] != "Platform Chat"));
    let notifications = flags
        .iter()
        .find(|f| f["name"] == "notifications")
        .expect("notifications flag");
    assert_eq!(
        notifications["system_enabled"],
        serde_json::Value::Bool(false)
    );
    assert_eq!(notifications["org_enabled"], serde_json::Value::Bool(false));

    let patched: serde_json::Value = server
        .patch(
            &format!("/v1/orgs/{org_id}/feature-flags"),
            serde_json::json!({ "flags": { "notifications": true } }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();
    assert!(patched.get("detail").is_some());

    let effective: serde_json::Value = server
        .get(&format!("/v1/orgs/{org_id}/feature-flags"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(effective["notifications"], serde_json::Value::Bool(false));
    assert!(effective.get("mcp_endpoint").is_none());
    assert_eq!(
        effective["machine_payments"],
        serde_json::Value::Bool(false)
    );

    server
        .patch(
            &format!("/v1/orgs/{org_id}/feature-flags"),
            serde_json::json!({ "flags": { "mcp_endpoint": true } }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
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
async fn test_create_session() {
    let server = TestServer::new().await;

    // Create an agent first
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "session-test-agent",
                "display_name": "Session Test Agent",
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
                "harness_id": server.seed_base_harness_id,
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
async fn test_session_secret_lifecycle_is_write_only_and_reserves_internal_names() {
    let server = TestServer::in_memory().await;
    let sentinel = "session-secret-must-never-be-returned";
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "session-secret-agent",
                "display_name": "Session Secret Agent",
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
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id,
                "title": "Session secret lifecycle"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let secrets_url = format!("/v1/sessions/{}/storage/secrets", session.id);

    let stored: Value = server
        .put(
            &secrets_url,
            json!({ "secrets": { "DISPOSABLE_KEY": sentinel } }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(stored["count"], 1);
    assert!(!stored.to_string().contains(sentinel));

    let listed: Value = server
        .get(&secrets_url)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(listed["data"][0]["name"], "DISPOSABLE_KEY");
    assert!(listed["data"][0].get("value").is_none());
    assert!(!listed.to_string().contains(sentinel));

    server
        .put(
            &secrets_url,
            json!({ "secrets": { "mcp_oauth:server:access_token": "blocked" } }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    server
        .delete(&format!("{secrets_url}/DISPOSABLE_KEY"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    let empty: Value = server
        .get(&secrets_url)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(empty["data"], json!([]));
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
                "harness_id": server.seed_base_harness_id,
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
                "harness_id": server.seed_base_harness_id,
                "agent_id": "agent_00000000000000000000000000000000",
                "title": "Should fail"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

async fn create_schedule_test_session(server: &TestServer, title: &str) -> Session {
    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "title": title
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

async fn seed_session_schedule(server: &TestServer, session: &Session) -> ScheduleId {
    let scheduled_at = Utc::now() + Duration::hours(1);
    let row = server
        .db
        .create_session_schedule(CreateSessionScheduleRow {
            org_id: 1,
            session_id: session.id,
            owner_principal_id: session.owner_principal_id,
            resolved_owner_user_id: session.resolved_owner_user_id,
            description: "Mismatch test schedule".to_string(),
            cron_expression: None,
            scheduled_at: Some(scheduled_at),
            timezone: "UTC".to_string(),
            next_trigger_at: Some(scheduled_at),
        })
        .await
        .unwrap();
    row.id
}

// ============================================
// Session Schedule API Tests
// ============================================

#[tokio::test]
async fn test_get_session_schedule_wrong_parent_returns_not_found() {
    let server = TestServer::in_memory().await;
    let session = create_schedule_test_session(&server, "Schedule Owner").await;
    let other_session = create_schedule_test_session(&server, "Wrong Parent").await;
    let schedule_id = seed_session_schedule(&server, &session).await;

    let body: Value = server
        .get(&format!(
            "/v1/sessions/{}/schedules/{}",
            other_session.id, schedule_id
        ))
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();

    assert_eq!(body["detail"], "Schedule not found");
}

#[tokio::test]
async fn test_update_session_schedule_wrong_parent_returns_not_found() {
    let server = TestServer::in_memory().await;
    let session = create_schedule_test_session(&server, "Schedule Owner").await;
    let other_session = create_schedule_test_session(&server, "Wrong Parent").await;
    let schedule_id = seed_session_schedule(&server, &session).await;

    let body: Value = server
        .patch(
            &format!(
                "/v1/sessions/{}/schedules/{}",
                other_session.id, schedule_id
            ),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(body["detail"], "Schedule not found");

    let persisted = server
        .db
        .get_session_schedule(1, schedule_id)
        .await
        .unwrap();
    assert!(persisted.unwrap().enabled);
}

#[tokio::test]
async fn test_delete_session_schedule_wrong_parent_returns_not_found() {
    let server = TestServer::in_memory().await;
    let session = create_schedule_test_session(&server, "Schedule Owner").await;
    let other_session = create_schedule_test_session(&server, "Wrong Parent").await;
    let schedule_id = seed_session_schedule(&server, &session).await;

    let body: Value = server
        .delete(&format!(
            "/v1/sessions/{}/schedules/{}",
            other_session.id, schedule_id
        ))
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(body["detail"], "Schedule not found");

    let persisted = server
        .db
        .get_session_schedule(1, schedule_id)
        .await
        .unwrap();
    assert!(persisted.is_some());
}

#[tokio::test]
async fn test_trigger_session_schedule_wrong_parent_returns_not_found() {
    let server = TestServer::in_memory().await;
    let session = create_schedule_test_session(&server, "Schedule Owner").await;
    let other_session = create_schedule_test_session(&server, "Wrong Parent").await;
    let schedule_id = seed_session_schedule(&server, &session).await;

    let body: Value = server
        .post(
            &format!(
                "/v1/sessions/{}/schedules/{}/trigger",
                other_session.id, schedule_id
            ),
            json!({}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(body["detail"], "Schedule not found");

    let persisted = server
        .db
        .get_session_schedule(1, schedule_id)
        .await
        .unwrap();
    let persisted = persisted.unwrap();
    assert_eq!(persisted.trigger_count, 0);
    assert!(persisted.last_triggered_at.is_none());
}

#[tokio::test]
async fn test_get_session() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "get-session-test-agent",
                "display_name": "Get Session Test Agent",
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
                "harness_id": server.seed_base_harness_id,
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
async fn test_get_session_context_report_without_generation() {
    let server = TestServer::in_memory().await;

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "title": "Context Report Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let report: SessionContextReport = server
        .get(&format!("/v1/sessions/{}/context-report", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(report.session_id, session.id.to_string());
    assert_eq!(report.estimated_input_tokens, 0);
    assert!(report.sections.is_empty());
}

#[tokio::test]
async fn test_sessions_pagination() {
    let server = TestServer::new().await;

    // Create an agent
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "pagination-test-agent",
                "display_name": "Pagination Test Agent",
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
                    "harness_id": server.seed_base_harness_id,
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
async fn test_create_user_message() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "message-test-agent",
                "display_name": "Message Test Agent",
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
                "harness_id": server.seed_base_harness_id,
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
                "name": "list-messages-test-agent",
                "display_name": "List Messages Test Agent",
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
                "harness_id": server.seed_base_harness_id,
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
                "name": "events-test-agent",
                "display_name": "Events Test Agent",
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
                "harness_id": server.seed_base_harness_id,
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

/// Tests the events endpoint with limit=1 returns only the last event
/// (used by CLI chat to efficiently snapshot before sending a message).
#[tokio::test]
async fn test_list_events_limit_one_returns_last_event() {
    let server = TestServer::in_memory().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "events-limit-test-agent",
                "display_name": "Events Limit Test Agent",
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
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a message to generate events
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "First message"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Get all events
    let all_events: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let all_data = all_events["data"].as_array().unwrap();
    assert!(
        !all_data.is_empty(),
        "Should have at least one event after message creation"
    );
    let last_event_id = all_data.last().unwrap()["id"].as_str().unwrap();

    // Get events with limit=1 — should return only the last event
    let limited: Value = server
        .get(&format!("/v1/sessions/{}/events?limit=1", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let limited_data = limited["data"].as_array().unwrap();
    assert_eq!(
        limited_data.len(),
        1,
        "limit=1 should return exactly one event"
    );
    assert_eq!(
        limited_data[0]["id"].as_str().unwrap(),
        last_event_id,
        "limit=1 should return the last event"
    );
}

/// Tests the events endpoint with since_id returns only events after the given ID
/// (used by CLI chat SSE streaming to avoid replaying old events).
#[tokio::test]
async fn test_list_events_since_id_filters_old_events() {
    let server = TestServer::in_memory().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "events-sinceid-test-agent",
                "display_name": "Events SinceId Test Agent",
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
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create first message
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "First message"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Snapshot the last event ID
    let events_after_first: Value = server
        .get(&format!("/v1/sessions/{}/events?limit=1", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let snapshot_id = events_after_first["data"][0]["id"].as_str().unwrap();

    // Get count of events before second message
    let all_before: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let count_before = all_before["data"].as_array().unwrap().len();

    // Create second message
    let _: Value = server
        .post(
            &format!("/v1/sessions/{}/messages", session.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "Second message"}]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Get events since snapshot — should NOT include events from the first message
    let events_since: Value = server
        .get(&format!(
            "/v1/sessions/{}/events?since_id={}",
            session.id, snapshot_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let since_data = events_since["data"].as_array().unwrap();

    // Should have new events from the second message, but none from before the snapshot
    assert!(
        !since_data.is_empty(),
        "Should have events after second message"
    );

    // Verify no event ID matches the snapshot or any earlier event
    let all_before_ids: Vec<&str> = all_before["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    for event in since_data {
        let event_id = event["id"].as_str().unwrap();
        assert!(
            !all_before_ids.contains(&event_id),
            "since_id should exclude event {event_id} which existed before snapshot"
        );
    }

    // Total events should be: before + new since events
    let all_after: Value = server
        .get(&format!("/v1/sessions/{}/events", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let count_after = all_after["data"].as_array().unwrap().len();
    assert_eq!(
        count_after,
        count_before + since_data.len(),
        "All events = events before snapshot + events since snapshot"
    );
}

// ============================================
// LLM Provider Tests
// ============================================

#[tokio::test]
async fn test_provider_crud() {
    let server = TestServer::new().await;

    // Create a provider
    let provider: Provider = server
        .post(
            "/v1/providers",
            json!({
                "name": "Test OpenAI Provider",
                "provider_type": "openai",
                "base_url": "https://api.openai.com/v1",
                "enabled": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(provider.name, "Test OpenAI Provider");

    // List providers
    server
        .get("/v1/providers")
        .await
        .assert_status(StatusCode::OK);

    // Delete provider
    server
        .delete(&format!("/v1/providers/{}", provider.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

/// Connection-level request options must survive the API round trip, and the
/// API must reject a header the transport owns rather than storing something
/// the drivers would silently drop later.
#[tokio::test]
async fn test_provider_request_options_round_trip_and_validation() {
    let server = TestServer::in_memory().await;

    let provider: Provider = server
        .post(
            "/v1/providers",
            json!({
                "name": "Gateway Anthropic",
                "provider_type": "anthropic",
                "request_options": {
                    "headers": [{"name": "x-gateway-tenant", "value": "acme"}],
                    "cache_diagnostics": true
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let options = provider
        .request_options
        .as_ref()
        .expect("request options returned on create");
    assert!(options.cache_diagnostics);
    assert_eq!(
        options.header_pairs(),
        vec![("x-gateway-tenant".to_string(), "acme".to_string())]
    );

    // Read back through GET: the options are stored, not just echoed.
    let fetched: Provider = server
        .get(&format!("/v1/providers/{}", provider.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(fetched.request_options, provider.request_options);

    // A transport-owned header is refused with a message naming it.
    let rejected = server
        .patch(
            &format!("/v1/providers/{}", provider.id),
            json!({
                "request_options": {
                    "headers": [{"name": "Host", "value": "evil.example"}]
                }
            }),
        )
        .await;
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    // Update replaces the options wholesale, including clearing them.
    let cleared: Provider = server
        .patch(
            &format!("/v1/providers/{}", provider.id),
            json!({ "request_options": { "headers": [], "cache_diagnostics": false } }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(cleared.request_options.is_none());
}

#[tokio::test]
async fn test_model_crud() {
    let server = TestServer::new().await;

    // Create a provider first
    let provider: Provider = server
        .post(
            "/v1/providers",
            json!({
                "name": "Model Test Provider",
                "provider_type": "openai"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Create a model
    let model: Model = server
        .post(
            &format!("/v1/providers/{}/models", provider.id),
            json!({
                "model_id": "gpt-4-test",
                "display_name": "GPT-4 Test",
                "capabilities": ["chat"],
                "enabled": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(model.model_id, "gpt-4-test");

    // List all models
    server.get("/v1/models").await.assert_status(StatusCode::OK);

    // Cleanup
    server
        .delete(&format!("/v1/models/{}", model.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .delete(&format!("/v1/providers/{}", provider.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_create_model_missing_provider_returns_not_found() {
    let server = TestServer::in_memory().await;

    server
        .post(
            "/v1/providers/provider_019563a3000070008000000000000001/models",
            json!({
                "model_id": "missing-provider-model",
                "display_name": "Missing Provider Model",
                "capabilities": ["chat"],
                "enabled": true
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ============================================
// Session Model Inheritance Tests
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
async fn test_session_filesystem() {
    let server = TestServer::new().await;

    // Create agent and session
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "filesystem-test-agent",
                "display_name": "Filesystem Test Agent",
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
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    // List root (new sessions mount scoped memory under the reserved memory root)
    let data: Value = server
        .get(&fs_url)
        .await
        .assert_status(StatusCode::OK)
        .json();
    let root_entries = data["data"].as_array().unwrap();
    assert_eq!(root_entries.len(), 1);
    assert_eq!(root_entries[0]["path"], "/memory");
    assert_eq!(root_entries[0]["is_directory"], true);

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
                "name": "fs-404-test-agent",
                "display_name": "FS 404 Test Agent",
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
                "harness_id": server.seed_base_harness_id,
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

#[tokio::test]
async fn test_session_file_download_path_returns_raw_bytes() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "fs-download-agent",
                "display_name": "FS Download Agent",
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
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);
    let pdf_bytes = b"%PDF-1.7\n%\x80\x81\x82\x83\n".to_vec();

    server
        .post(
            &format!("{}/report.pdf", fs_url),
            json!({
                "content": BASE64.encode(&pdf_bytes),
                "encoding": "base64"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let json_read: SessionFile = server
        .get(&format!("{}/report.pdf", fs_url))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(json_read.encoding, "base64");
    assert_eq!(
        json_read.content.as_deref(),
        Some(BASE64.encode(&pdf_bytes).as_str())
    );

    let raw = server
        .get(&format!("{}/_/download/report.pdf", fs_url))
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        raw.headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/pdf")
    );
    assert_eq!(
        raw.headers()
            .get("content-disposition")
            .and_then(|value| value.to_str().ok()),
        Some("attachment; filename=\"report.pdf\"; filename*=UTF-8''report.pdf")
    );
    assert_eq!(raw.bytes(), pdf_bytes.as_slice());
}

#[tokio::test]
async fn test_session_file_read_accept_octet_stream_returns_raw_bytes() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "fs-negotiate-agent",
                "display_name": "FS Negotiate Agent",
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
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);
    let raw_bytes = vec![0, 159, 146, 150];

    server
        .post(
            &format!("{}/payload.bin", fs_url),
            json!({
                "content": BASE64.encode(&raw_bytes),
                "encoding": "base64"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let response = server
        .request_raw(
            axum::http::Method::GET,
            &format!("{}/payload.bin", fs_url),
            vec![("accept", "application/octet-stream")],
            vec![],
        )
        .await
        .assert_status(StatusCode::OK);

    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/octet-stream")
    );
    assert_eq!(
        response
            .headers()
            .get("content-disposition")
            .and_then(|value| value.to_str().ok()),
        Some("inline; filename=\"payload.bin\"; filename*=UTF-8''payload.bin")
    );
    assert_eq!(response.bytes(), raw_bytes.as_slice());
}

#[tokio::test]
async fn test_session_file_read_accept_octet_stream_q_zero_keeps_json_response() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "fs-negotiate-q-zero-agent",
                "display_name": "FS Negotiate Q Zero Agent",
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
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);
    let raw_bytes = vec![0, 159, 146, 150];

    server
        .post(
            &format!("{}/payload.bin", fs_url),
            json!({
                "content": BASE64.encode(&raw_bytes),
                "encoding": "base64"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let response = server
        .request_raw(
            axum::http::Method::GET,
            &format!("{}/payload.bin", fs_url),
            vec![("accept", "application/octet-stream;q=0, application/json")],
            vec![],
        )
        .await
        .assert_status(StatusCode::OK);

    let file: SessionFile = response.json();
    assert_eq!(file.encoding, "base64");
    assert_eq!(
        file.content.as_deref(),
        Some(BASE64.encode(&raw_bytes).as_str())
    );
}

#[tokio::test]
async fn test_session_file_download_path_rejects_directories() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "fs-download-dir-agent",
                "display_name": "FS Download Dir Agent",
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
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent.public_id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let fs_url = format!("/v1/sessions/{}/fs", session.id);

    server
        .post(
            &format!("{}/docs", fs_url),
            json!({
                "is_directory": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let response = server.get(&format!("{}/_/download/docs", fs_url)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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
    assert!(names.contains(&"base"), "Should have Base harness");
    assert!(names.contains(&"generic"), "Should have Generic harness");
}

#[tokio::test]
async fn test_get_base_harness() {
    let server = TestServer::new().await;

    let harness: Harness = server
        .get(&format!("/v1/harnesses/{}", server.seed_base_harness_id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(harness.name, "base");
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
        .get(&format!("/v1/harnesses/{}", server.seed_generic_harness_id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(harness.name, "generic");
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
        23,
        "Generic harness should have 23 capabilities"
    );
    assert!(
        cap_ids.contains(&"human_intent"),
        "Should have human intent narration"
    );
    assert!(
        cap_ids.contains(&"session_file_system"),
        "Should have file system"
    );
    assert!(
        cap_ids.contains(&"bashkit_shell"),
        "Should have bashkit shell"
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
        cap_ids.contains(&"session_schedule"),
        "Should have session schedules"
    );
    assert!(cap_ids.contains(&"btw"), "Should have btw capability");
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
        cap_ids.contains(&"auto_tool_search"),
        "Should have auto tool search"
    );
    assert!(cap_ids.contains(&"budgeting"), "Should have budgeting");
    assert!(
        cap_ids.contains(&"self_budget"),
        "Should have self_budget guidance"
    );
    assert!(cap_ids.contains(&"compaction"), "Should have compaction");
    assert!(
        cap_ids.contains(&"loop_detection"),
        "Should have loop detection"
    );
    assert!(
        cap_ids.contains(&"message_metadata"),
        "Should have message metadata annotations"
    );
    assert!(
        cap_ids.contains(&"error_disclosure"),
        "Should have detailed error disclosure"
    );
    assert!(
        cap_ids.contains(&"citation_retrieval"),
        "Should have retrieval citations"
    );
    assert!(
        cap_ids.contains(&"citation_verification"),
        "Should have citation verification"
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
                "name": "generic-harness-test-agent",
                "display_name": "Generic Harness Test Agent",
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
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent.public_id,
                "title": "Generic Harness Session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(session.title.as_deref(), Some("Generic Harness Session"));
}

async fn create_llmsim_agent(server: &TestServer, name: &str) -> Agent {
    let provider: Value = server
        .post(
            "/v1/providers",
            json!({
                "name": format!("{name}-provider"),
                "provider_type": "llmsim"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let model: Value = server
        .post(
            &format!(
                "/v1/providers/{}/models",
                provider["id"].as_str().expect("provider id")
            ),
            json!({
                "model_id": format!("{name}-model-{}", uuid::Uuid::new_v4()),
                "display_name": format!("{name} model"),
                "enabled": true,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(
            "/v1/agents",
            json!({
                "name": format!("{name}-agent"),
                "display_name": format!("{name} Agent"),
                "system_prompt": "You are a concise test agent.",
                "default_model_id": model["id"],
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

#[tokio::test]
async fn test_session_commands_are_scoped_to_active_capabilities() {
    let server = TestServer::in_memory().await;

    let generic_session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "title": "Generic command session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let base_session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "title": "Base command session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let generic_commands: Value = server
        .get(&format!("/v1/sessions/{}/commands", generic_session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let base_commands: Value = server
        .get(&format!("/v1/sessions/{}/commands", base_session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let generic_names: Vec<&str> = generic_commands["commands"]
        .as_array()
        .expect("generic commands")
        .iter()
        .filter_map(|command| command["name"].as_str())
        .collect();
    let base_names: Vec<&str> = base_commands["commands"]
        .as_array()
        .expect("base commands")
        .iter()
        .filter_map(|command| command["name"].as_str())
        .collect();

    assert!(generic_names.contains(&"btw"));
    assert!(!base_names.contains(&"btw"));
}

#[tokio::test]
async fn test_execute_btw_returns_ephemeral_response() {
    let server = TestServer::in_memory().await;
    let agent = create_llmsim_agent(&server, "btw-execute").await;

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent.public_id,
                "title": "BTW execute session"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let messages_before: Value = server
        .get(&format!("/v1/sessions/{}/messages", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    let result: Value = server
        .post(
            &format!("/v1/sessions/{}/commands/execute", session.id),
            json!({
                "name": "btw",
                "arguments": "What are you doing?"
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    let messages_after: Value = server
        .get(&format!("/v1/sessions/{}/messages", session.id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(result["success"], Value::Bool(true));
    assert_eq!(
        result["message"],
        Value::String("Hello! I'm a simulated LLM response.".to_string())
    );
    assert_eq!(
        messages_before["data"].as_array().map(Vec::len),
        messages_after["data"].as_array().map(Vec::len)
    );
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
async fn test_check_harness_name_available() {
    let server = TestServer::new().await;

    let data: Value = server
        .get("/v1/harnesses/check-name?name=fresh-new-name")
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(data["available"], true);
}

#[tokio::test]
async fn test_check_harness_name_taken() {
    let server = TestServer::new().await;

    // "generic" is a built-in harness name
    let data: Value = server
        .get("/v1/harnesses/check-name?name=generic")
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(data["available"], false);
}

#[tokio::test]
async fn test_check_harness_name_taken_with_exclude_id() {
    let server = TestServer::new().await;

    // Create a harness
    let harness: Harness = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "check-name-test",
                "display_name": "Check Name Test",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Without exclude_id — should be taken
    let data: Value = server
        .get("/v1/harnesses/check-name?name=check-name-test")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(data["available"], false);

    // With exclude_id (self) — should be available
    let data: Value = server
        .get(&format!(
            "/v1/harnesses/check-name?name=check-name-test&exclude_id={}",
            harness.id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(data["available"], true);
}

#[tokio::test]
async fn test_check_harness_name_invalid_format() {
    let server = TestServer::new().await;

    // Uppercase is invalid for harness names
    let data: Value = server
        .get("/v1/harnesses/check-name?name=INVALID")
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(data["available"], false);
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
                "name": "original-harness",
                "display_name": "Original Harness",
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
    assert_eq!(copied.name, "original-harness-copy");
    assert_eq!(
        copied.display_name.as_deref(),
        Some("Original Harness (copy)")
    );
    assert_eq!(
        copied.description.as_deref(),
        Some("Original harness description")
    );
    assert_eq!(copied.system_prompt.as_deref(), Some("Harness prompt"));
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
                "name": "missing-model-harness",
                "display_name": "Missing Model Harness",
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
                "name": "update-missing-model-harness",
                "display_name": "Update Missing Model Harness",
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
async fn test_update_nonexistent_harness_returns_not_found() {
    let server = TestServer::in_memory().await;

    server
        .patch(
            "/v1/harnesses/harness_ffffffffffffffffffffffffffffffff",
            json!({ "name": "updated" }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_destroy_nonexistent_harness_returns_not_found() {
    let server = TestServer::in_memory().await;

    server
        .post(
            "/v1/harnesses/harness_ffffffffffffffffffffffffffffffff/delete",
            json!({}),
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
            &format!("/v1/harnesses/{}/copy", server.seed_generic_harness_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(copied.name, "generic-copy");
    assert_eq!(copied.display_name.as_deref(), Some("Generic (copy)"));
    // Generic harness capabilities should be preserved on copy
    assert_eq!(
        copied.capabilities.len(),
        23,
        "Copied harness should have same 23 capabilities"
    );
    assert!(
        copied
            .capabilities
            .iter()
            .any(|cap| cap.capability_id() == "human_intent")
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
async fn test_session_databases_crud() {
    let server = TestServer::new().await;

    // Create agent + session for the test
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "sql-test-agent",
                "display_name": "SQL Test Agent",
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
                "harness_id": server.seed_base_harness_id,
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
                "name": "sql-schema-agent",
                "display_name": "SQL Schema Agent",
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
            json!({"harness_id": server.seed_base_harness_id, "agent_id": agent.public_id.to_string()}),
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
                "name": "sql-invalid-name-agent",
                "display_name": "SQL Invalid Name Agent",
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": server.seed_base_harness_id, "agent_id": agent.public_id.to_string()}),
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

#[tokio::test]
async fn test_session_databases_limit_exceeded_returns_422() {
    let server = TestServer::in_memory().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "sql-limit-agent",
                "display_name": "SQL Limit Agent",
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({"harness_id": server.seed_base_harness_id, "agent_id": agent.public_id.to_string()}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session_id = session.id.to_string();

    for i in 0..10 {
        server
            .post(
                &format!("/v1/sessions/{session_id}/databases"),
                json!({ "name": format!("db_{i}") }),
            )
            .await
            .assert_status(StatusCode::CREATED);
    }

    server
        .post(
            &format!("/v1/sessions/{session_id}/databases"),
            json!({ "name": "db_overflow" }),
        )
        .await
        .assert_status(StatusCode::UNPROCESSABLE_ENTITY);
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
            json!({"name": "base-features-agent", "display_name": "Base Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Base harness has no capabilities → no features
    let session: Session = server
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
            json!({"name": "generic-features-agent", "display_name": "Generic Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Generic harness has session_file_system, bashkit_shell, session_storage, etc.
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
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
    // file_system should only appear once despite session_file_system + bashkit_shell
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
            json!({"name": "get-features-agent", "display_name": "Get Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
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
            json!({"name": "list-features-agent", "display_name": "List Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let _session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
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
            json!({"name": "session-cap-features-agent", "display_name": "Session Cap Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Base harness has no caps, but add session_schedule at session level
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
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
async fn test_session_features_sql_database() {
    let server = TestServer::new().await;

    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({"name": "sqldb-features-agent", "display_name": "SqlDb Features Agent", "system_prompt": "Test"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    // Add session_sql_database via session-level capability
    let session: Session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
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

    // noop is a test fixture (everruns-test-support) and must not appear in
    // the product capability registry.
    assert!(
        !data.iter().any(|c| c["id"] == "noop"),
        "noop fixture capability must not be registered in the product",
    );
}

// ============================================
// Global Chat Session Tests
// ============================================

#[tokio::test]
async fn test_platform_chat_creates_session() {
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
async fn test_platform_chat_returns_same_session() {
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
async fn test_platform_chat_none_mode_reuse_accepts_messages_and_commands() {
    let server = TestServer::in_memory().await;

    let first: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();
    let reused: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();
    assert_eq!(reused.id, first.id);

    server
        .post(
            &format!("/v1/sessions/{}/messages", reused.id),
            json!({
                "message": {
                    "role": "user",
                    "content": [{ "type": "text", "text": "hello" }]
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let commands = server
        .get(&format!("/v1/sessions/{}/commands", reused.id))
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert!(
        commands["commands"]
            .as_array()
            .expect("commands array")
            .iter()
            .any(|command| command["name"] == "btw"),
        "the recovered Platform Chat session should resolve its harness commands"
    );
}

#[tokio::test]
async fn test_platform_chat_has_chat_harness() {
    let server = TestServer::new().await;

    let session: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();

    assert_eq!(
        session.harness_id.to_string(),
        server.seed_chat_harness_id,
        "Chat session should use the Platform Chat harness"
    );
}

#[tokio::test]
async fn test_platform_chat_is_unconditional_while_voice_remains_gated() {
    let server = TestServer::in_memory().await;
    let org_id = "org_00000000000000000000000000000001";

    // Turning off voice leaves Chats available because it is core functionality,
    // while voice remains independently hidden when disabled.
    server
        .patch(
            &format!("/v1/orgs/{org_id}/feature-flags"),
            json!({ "flags": { "voice": false } }),
        )
        .await
        .assert_status(StatusCode::OK);

    let effective_flags: Value = server
        .get(&format!("/v1/orgs/{org_id}/feature-flags"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(effective_flags["voice"], Value::Bool(false));

    server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success();

    // The voice gate rejects before any external provider call.
    server
        .post("/v1/sessions/chat/voice", json!({ "sdp": "v=0" }))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_platform_chat_ignores_tag_match_owned_by_other_principal() {
    let server = TestServer::in_memory().await;

    let initial: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();

    let attacker_user_id = Uuid::new_v4();
    let attacker_principal = server
        .db
        .create_principal(CreatePrincipalRow {
            id: PrincipalId::from_uuid(Uuid::new_v4()),
            org_id: DEFAULT_ORG_ID,
            kind: "user".to_string(),
            subject_id: Some(attacker_user_id),
            parent_principal_id: None,
            resolved_user_id: Some(attacker_user_id),
            metadata: json!({}),
        })
        .await
        .expect("failed to create attacker principal");

    server
        .db
        .update_session(
            DEFAULT_ORG_ID,
            initial.id,
            UpdateSession {
                owner_principal_id: Some(attacker_principal.id),
                resolved_owner_user_id: everruns_durable::UpdateField::Set(attacker_user_id),
                ..Default::default()
            },
        )
        .await
        .expect("failed to mutate chat session owner")
        .expect("chat session should exist");

    let recreated: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();

    assert_ne!(
        recreated.id, initial.id,
        "chat lookup must not attach to tag-matching sessions owned by a different principal"
    );
    assert_ne!(
        recreated.owner_principal_id, attacker_principal.id,
        "new Platform Chat should be owned by the authenticated user's principal"
    );

    server
        .get(&format!("/v1/sessions/{}/commands", initial.id))
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_platform_chat_repairs_stale_harness_binding() {
    let server = TestServer::in_memory().await;

    let original: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();

    let other_harness: Harness = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "chat-repair-test",
                "display_name": "Chat Repair Test",
                "system_prompt": "Test harness"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .db
        .update_session(
            DEFAULT_ORG_ID,
            original.id,
            UpdateSession {
                harness_id: Some(other_harness.id),
                ..Default::default()
            },
        )
        .await
        .expect("failed to mutate chat session")
        .expect("chat session should exist");

    let repaired: Session = server
        .post("/v1/sessions/chat", json!({}))
        .await
        .assert_success()
        .json();

    assert_eq!(repaired.id, original.id);
    assert_eq!(
        repaired.harness_id.to_string(),
        server.seed_chat_harness_id,
        "chat endpoint should repair stale session harness bindings"
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
        .find(|h| h.name == "platform-chat")
        .expect("Platform Chat harness should exist in seed data");

    assert_eq!(chat_harness.id.to_string(), server.seed_chat_harness_id);
    assert!(chat_harness.tags.contains(&"chat".to_string()));
}

#[tokio::test]
async fn test_chat_harness_includes_platform_capability() {
    let server = TestServer::new().await;

    let harness: Harness = server
        .get(&format!("/v1/harnesses/{}", server.seed_chat_harness_id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(harness.name, "platform-chat");
    assert_eq!(
        harness.parent_harness_id.as_ref().map(ToString::to_string),
        Some(server.seed_base_harness_id.to_string()),
        "Platform Chat should inherit from Base to keep its tool surface focused"
    );

    let cap_ids: Vec<&str> = harness
        .capabilities
        .iter()
        .map(|c| c.capability_id())
        .collect();

    assert_eq!(
        cap_ids,
        vec![
            "platform",
            "btw",
            "loop_detection",
            "error_disclosure",
            "compaction"
        ],
        "Platform Chat should keep platform operations, commands, and runtime safeguards locally"
    );

    let preview: Value = server
        .post(
            "/v1/harnesses/preview",
            json!({
                "system_prompt": harness.system_prompt,
                "parent_harness_id": harness.parent_harness_id,
                "capabilities": harness.capabilities,
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    let tool_names: Vec<&str> = preview["tools"]
        .as_array()
        .expect("preview tools should be an array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();

    for expected in ["discover", "query", "execute"] {
        assert!(
            tool_names.contains(&expected),
            "Platform Chat preview should include {expected}"
        );
    }
    assert!(
        !tool_names.contains(&"manage_harnesses"),
        "Platform Chat should use the catalog surface, not legacy management tools"
    );
    for excluded in ["bash", "web_fetch", "secret_store", "schedule_create"] {
        assert!(
            !tool_names.contains(&excluded),
            "Platform Chat should not expose unrelated {excluded}"
        );
    }
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
                "name": "readonly-delete-test",
                "display_name": "Readonly Delete Test",
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
                "harness_id": server.seed_base_harness_id,
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
                "name": "readonly-recursive-delete-test",
                "display_name": "Readonly Recursive Delete Test",
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
                "harness_id": server.seed_base_harness_id,
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
                "name": "normal-delete-test",
                "display_name": "Normal Delete Test",
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
                "harness_id": server.seed_base_harness_id,
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
async fn test_verify_connectors_listed() {
    let server = TestServer::in_memory().await;

    let resp: Value = server
        .get("/v1/user/connections/providers")
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Should be an array (may include plugin-registered providers)
    assert!(resp.is_array());
}

// ============================================
// App Reference Validation Tests
// ============================================

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

    server
        .post(
            "/v1/apps",
            json!({
                "name": "Agent Delete Blocker",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .delete(&format!("/v1/agents/{}", agent["id"].as_str().unwrap()))
        .await
        .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_delete_harness_referenced_by_app_returns_conflict() {
    let server = TestServer::in_memory().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "delete-harness-app-agent",
                "display_name": "Delete Harness App Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let harness: Harness = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "delete-app-harness",
                "display_name": "Delete App Harness",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(
            "/v1/apps",
            json!({
                "name": "Harness Delete Blocker",
                "harness_id": harness.id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .delete(&format!("/v1/harnesses/{}", harness.id))
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

    server
        .post(
            "/v1/apps",
            json!({
                "name": "Identity Delete Blocker",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "agent_identity_id": identity["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .delete(&format!(
            "/v1/agent-identities/{}",
            identity["id"].as_str().unwrap()
        ))
        .await
        .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_delete_harness_with_child_returns_conflict() {
    let server = TestServer::in_memory().await;

    let parent: Harness = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "parent-delete-blocker",
                "display_name": "Parent Delete Blocker",
                "system_prompt": "Parent"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(
            "/v1/harnesses",
            json!({
                "name": "child-delete-blocker",
                "display_name": "Child Delete Blocker",
                "system_prompt": "Child",
                "parent_harness_id": parent.id
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .delete(&format!("/v1/harnesses/{}", parent.id))
        .await
        .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_delete_org_default_harness_returns_conflict() {
    let server = TestServer::in_memory().await;

    let harness: Harness = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "org-default-delete-blocker",
                "display_name": "Org Default Delete Blocker",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .db
        .patch_organization_settings(
            DEFAULT_ORG_ID,
            UpdateOrganizationSettings {
                default_model_id: UpdateField::Unchanged,
                default_harness_id: UpdateField::Set(harness.id),
                base_harness_id: UpdateField::Unchanged,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    server
        .delete(&format!("/v1/harnesses/{}", harness.id))
        .await
        .assert_status(StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_delete_entities_referenced_only_by_session_succeeds() {
    let server = TestServer::in_memory().await;

    let harness: Harness = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "session-only-delete-harness",
                "display_name": "Session Only Delete Harness",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "session-only-delete-agent",
                "display_name": "Session Only Delete Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let identity: Value = server
        .post(
            "/v1/agent-identities",
            json!({"name": "Session Only Delete Identity"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": harness.id,
                "agent_id": agent["id"],
                "agent_identity_id": identity["id"],
                "title": "Session-only references"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .delete(&format!("/v1/agents/{}", agent["id"].as_str().unwrap()))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .delete(&format!("/v1/harnesses/{}", harness.id))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .delete(&format!(
            "/v1/agent-identities/{}",
            identity["id"].as_str().unwrap()
        ))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn test_create_app_missing_harness_returns_not_found() {
    let server = TestServer::new().await;

    // Create an agent to use
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "app-missing-harness-agent",
                "display_name": "Test Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(
            "/v1/apps",
            json!({
                "name": "Test App",
                "harness_id": "harness_ffffffffffffffffffffffffffffffff",
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_create_app_missing_agent_returns_not_found() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/apps",
            json!({
                "name": "Test App",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": "agent_ffffffffffffffffffffffffffffffff"
            }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_app_schedule_rejected_and_webhook_persists_in_postgres() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "app-invocation-postgres-agent",
                "display_name": "Invocation Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let schedule_response: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Scheduled Repo Check",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "channel_type": "schedule",
                "channel_config": {
                    "cron_expression": "0 15 * * * * *",
                    "timezone": "UTC",
                    "session_mode": "shared_session",
                    "message": "check repo"
                }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();

    assert!(
        schedule_response["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("schedule trigger on the app's agent"),
        "unexpected response: {schedule_response:?}"
    );

    let webhook_app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Webhook Repo Check",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "channel_type": "webhook",
                "channel_config": {
                    "token": "secret-token",
                    "session_mode": "session_per_invocation",
                    "message": "check webhook"
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let stored_webhook_app: Value = server
        .get(&format!("/v1/apps/{}", webhook_app["id"].as_str().unwrap()))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(stored_webhook_app["channels"].as_array().unwrap().len(), 1);
    assert_eq!(stored_webhook_app["channels"][0]["channel_type"], "webhook");
    assert!(
        stored_webhook_app["channels"][0]["channel_config"]["token"].is_null(),
        "webhook token must not be returned"
    );
    assert_eq!(
        stored_webhook_app["channels"][0]["channel_config"]["token_configured"],
        true
    );
    assert_eq!(
        stored_webhook_app["channels"][0]["channel_config"]["session_mode"],
        "session_per_invocation"
    );
    assert_eq!(
        stored_webhook_app["channels"][0]["channel_config"]["message"],
        "check webhook"
    );
}

#[tokio::test]
async fn test_publish_app_without_channels_returns_bad_request() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "channel-less-app-agent",
                "display_name": "Channel-less App Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Channel-less App",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let response: Value = server
        .post(
            &format!("/v1/apps/{}/publish", app["id"].as_str().unwrap()),
            json!({}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();

    assert!(
        response["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("at least one channel"),
        "unexpected response: {response:?}"
    );
}

#[tokio::test]
async fn test_update_app_to_published_returns_bad_request() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "patch-published-app-agent",
                "display_name": "Patch Published App Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Patch Published App",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let response: Value = server
        .patch(
            &format!("/v1/apps/{}", app["id"].as_str().unwrap()),
            json!({ "status": "published" }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();

    assert!(
        response["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("publish/unpublish endpoints"),
        "unexpected response: {response:?}"
    );
}

#[tokio::test]
async fn test_publish_archived_app_returns_bad_request() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "archived-publish-app-agent",
                "display_name": "Archived Publish App Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Archived Publish App",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "channel_type": "webhook",
                "channel_config": {
                    "token": "secret-token",
                    "session_mode": "session_per_invocation",
                    "message": "check webhook"
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .patch(
            &format!("/v1/apps/{}", app["id"].as_str().unwrap()),
            json!({ "status": "archived" }),
        )
        .await
        .assert_status(StatusCode::OK);

    let response: Value = server
        .post(
            &format!("/v1/apps/{}/publish", app["id"].as_str().unwrap()),
            json!({}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST)
        .json();

    assert!(
        response["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("draft before publishing"),
        "unexpected response: {response:?}"
    );
}

#[tokio::test]
async fn test_update_app_missing_harness_returns_not_found() {
    let server = TestServer::new().await;

    // Create agent and app
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({ "name": "update-app-harness-agent", "display_name": "Test Agent", "system_prompt": "Test" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Test App",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .patch(
            &format!("/v1/apps/{}", app["id"].as_str().unwrap()),
            json!({ "harness_id": "harness_ffffffffffffffffffffffffffffffff" }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_app_missing_agent_returns_not_found() {
    let server = TestServer::new().await;

    // Create agent and app
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({ "name": "update-app-agent-agent", "display_name": "Test Agent", "system_prompt": "Test" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Test App",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .patch(
            &format!("/v1/apps/{}", app["id"].as_str().unwrap()),
            json!({ "agent_id": "agent_ffffffffffffffffffffffffffffffff" }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_app_reencrypts_legacy_plaintext_channel_configs() {
    let server = TestServer::new().await;

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({ "name": "update-app-reencrypt-agent", "display_name": "Test Agent", "system_prompt": "Test" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": "Legacy plaintext app",
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "channel_type": "slack",
                "channel_config": {
                    "bot_token": "xoxb-reencrypt",
                    "signing_secret": "signing-reencrypt"
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let channel_id = app["channels"][0]["id"].as_str().unwrap().to_string();
    let channel_row = server
        .db
        .get_app_channel_by_public_id(&channel_id)
        .await
        .unwrap()
        .unwrap();

    // Simulate legacy migrated row: plaintext secrets + NULL ciphertext.
    // UpdateAppChannel uses COALESCE and cannot clear channel_config_encrypted,
    // so write the legacy state directly.
    let pool = server
        .db
        .pool()
        .expect("Postgres pool required for this test");
    sqlx::query(
        "UPDATE app_channels \
         SET channel_config = $1, channel_config_encrypted = NULL \
         WHERE id = $2",
    )
    .bind(json!({
        "bot_token": "xoxb-plaintext",
        "signing_secret": "signing-plaintext"
    }))
    .bind(channel_row.id)
    .execute(pool)
    .await
    .unwrap();

    server
        .patch(
            &format!("/v1/apps/{}", app["id"].as_str().unwrap()),
            json!({ "description": "touch app to trigger opportunistic encryption" }),
        )
        .await
        .assert_status(StatusCode::OK);

    let updated_channel_row = server
        .db
        .get_app_channel_by_public_id(&channel_id)
        .await
        .unwrap()
        .unwrap();

    assert!(
        updated_channel_row.channel_config_encrypted.is_some(),
        "channel config should be encrypted after app update"
    );
    assert_eq!(updated_channel_row.channel_config, json!({}));
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

// ============================================================================
// Agent Identity Connection tests
// ============================================================================

#[tokio::test]
async fn test_identity_connections_list_empty() {
    let server = TestServer::in_memory().await;

    // Create an identity first
    let identity: Value = server
        .post("/v1/agent-identities", json!({"name": "ConnTest"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = identity["id"].as_str().unwrap();

    // List connections — should be empty
    let connections: Vec<Value> = server
        .get(&format!("/v1/agent-identities/{id}/connections"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(connections.is_empty());
}

#[tokio::test]
async fn test_identity_connections_not_found_for_missing_identity() {
    let server = TestServer::in_memory().await;

    server
        .get("/v1/agent-identities/identity_019d166cd0147e638c72892ecb30ffff/connections")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_identity_connections_delete_not_found() {
    let server = TestServer::in_memory().await;

    let identity: Value = server
        .post("/v1/agent-identities", json!({"name": "ConnDel"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = identity["id"].as_str().unwrap();

    server
        .delete(&format!(
            "/v1/agent-identities/{id}/connections/nonexistent"
        ))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_identity_connections_create_unknown_provider() {
    let server = TestServer::in_memory().await;

    let identity: Value = server
        .post("/v1/agent-identities", json!({"name": "ConnProv"}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = identity["id"].as_str().unwrap();

    server
        .post(
            &format!("/v1/agent-identities/{id}/connections/nonexistent"),
            json!({"api_key": "test"}),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// ============================================
// Account Deletion & Data Export Tests
// ============================================

#[tokio::test]
async fn test_export_user_data() {
    let server = TestServer::in_memory().await;

    let resp: Value = server
        .get("/v1/users/me/export")
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Verify export structure
    assert!(resp["user"]["id"].is_string());
    assert!(resp["user"]["email"].is_string());
    assert!(resp["user"]["name"].is_string());
    assert!(resp["user"]["created_at"].is_string());
    assert!(resp["organizations"].is_array());
    assert!(resp["personal_access_tokens"].is_array());
    assert!(resp["exported_at"].is_string());
    // Verify no sensitive fields
    assert!(resp["user"].get("password_hash").is_none());
    assert!(resp["user"].get("roles").is_none());
}

#[tokio::test]
async fn test_delete_user_account() {
    let server = TestServer::in_memory().await;

    // Delete account
    let resp: Value = server
        .delete("/v1/users/me")
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(resp["deleted"], true);

    // After deletion, export should fail with 404
    server
        .get("/v1/users/me/export")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_image_ids_round_trip_across_upload_list_and_get() {
    let server = TestServer::in_memory().await;
    let boundary = "----everruns-image-upload";
    let content_type = format!("multipart/form-data; boundary={boundary}");
    let image_bytes = vec![0x89, 0x50, 0x4E, 0x47];
    let mut body = Vec::new();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\n\
Content-Disposition: form-data; name=\"file\"; filename=\"upload.png\"\r\n\
Content-Type: image/png\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(&image_bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    let upload: Value = server
        .request_raw(
            axum::http::Method::POST,
            "/v1/images",
            vec![("content-type", content_type.as_str())],
            body,
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let image_id = upload["id"].as_str().expect("upload image id");
    assert!(
        image_id.starts_with("img_"),
        "upload should return public image id, got {image_id}"
    );

    let listed: Vec<Value> = server
        .get("/v1/images")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(listed.len(), 1, "expected uploaded image to be listed");
    assert_eq!(listed[0]["id"], upload["id"]);

    server
        .get(&format!("/v1/images/{image_id}"))
        .await
        .assert_status(StatusCode::OK);
}
