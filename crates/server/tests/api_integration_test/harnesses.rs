//! API integration tests: harnesses.

use crate::test_harness;
use axum::http::StatusCode;
use everruns_core::DEFAULT_ORG_ID;
use everruns_durable::UpdateField;
use everruns_platform::Agent;
use everruns_platform::Harness;
use everruns_platform::Session;
use everruns_server::storage::models::UpdateOrganizationSettings;
use serde_json::{Value, json};
use test_harness::TestServer;

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
        24,
        "Generic harness should have 24 capabilities"
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
        cap_ids.contains(&"soft_approval"),
        "Should have soft approval"
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
        24,
        "Copied harness should have same 24 capabilities"
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
            "human_intent",
            "current_time",
            "message_metadata",
            "parallel_tool_calls",
            "stateless_todo_list",
            "prompt_caching",
            "tool_call_repair",
            "loop_detection",
            "error_disclosure",
            "compaction",
            "soft_approval"
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
