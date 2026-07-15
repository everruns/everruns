//! Skills integration tests
//!
//! Tests the skills CRUD API and the integration of skills as virtual capabilities.

mod test_harness;

use axum::http::StatusCode;
use serde_json::json;
use test_harness::TestServer;

const VALID_SKILL_MD: &str = "---\nname: test-skill\ndescription: A test skill for integration testing.\n---\n\n# Test Skill\n\n## Instructions\n\n1. Read the input data\n2. Process it according to the rules\n3. Return the result\n";

/// Generate a unique, schema-valid skill name for this test run.
///
/// All tests share one PostgreSQL database (and the default org), and skill
/// names are unique per org, so fixed names collide across tests (409). Names
/// must match `^[a-z0-9]+(-[a-z0-9]+)*$`, so derive a lowercase-hex suffix from
/// a UUIDv7 and prefix it.
fn unique_skill_name(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        &uuid::Uuid::now_v7().simple().to_string()[..12]
    )
}

/// Build a minimal valid skill markdown document for `name`.
fn skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n")
}

// ============================================================
// Skill CRUD Tests
// ============================================================

#[tokio::test]
async fn test_create_skill() {
    let server = TestServer::new().await;

    let name = unique_skill_name("test-skill");
    let resp = server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name, "A test skill for integration testing.") }),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.json_value();
    assert_eq!(body["name"], name);
    assert_eq!(body["description"], "A test skill for integration testing.");
    assert_eq!(body["source_type"], "markdown");
    assert_eq!(body["status"], "active");
    assert!(body["id"].as_str().unwrap().starts_with("skill_"));
}

#[tokio::test]
async fn test_list_skills() {
    let server = TestServer::new().await;

    // Create two skills
    let name_a = unique_skill_name("test-skill");
    let name_b = unique_skill_name("data-analysis");
    server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name_a, "A test skill for integration testing.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name_b, "Analyze datasets and generate reports.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // List skills
    let resp = server.get("/v1/skills").await.assert_success();
    let body = resp.json_value();
    let skills = body["data"].as_array().unwrap();
    assert!(skills.len() >= 2);

    let names: Vec<&str> = skills.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(names.contains(&name_a.as_str()));
    assert!(names.contains(&name_b.as_str()));
}

#[tokio::test]
async fn test_get_skill() {
    let server = TestServer::new().await;

    let name = unique_skill_name("test-skill");
    let create_resp = server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name, "A test skill for integration testing.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let get_resp = server
        .get(&format!("/v1/skills/{skill_id}"))
        .await
        .assert_success();

    let body = get_resp.json_value();
    assert_eq!(body["name"], name);
    assert_eq!(body["id"], skill_id);
}

#[tokio::test]
async fn test_get_skill_content() {
    let server = TestServer::new().await;

    let name = unique_skill_name("test-skill");
    let create_resp = server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name, "A test skill for integration testing.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let content_resp = server
        .get(&format!("/v1/skills/{skill_id}/content"))
        .await
        .assert_success();

    let body = content_resp.json_value();
    assert!(
        body["skill_md"]
            .as_str()
            .unwrap()
            .contains(&format!("# {name}"))
    );
}

#[tokio::test]
async fn test_deleted_skill_content_returns_not_found() {
    let server = TestServer::new().await;

    let skill_name = format!(
        "deleted-content-skill-{}",
        &uuid::Uuid::now_v7().simple().to_string()[..8]
    );
    let skill_md = format!(
        "---\nname: {skill_name}\ndescription: Deleted content visibility test.\n---\n\n# Deleted Content Skill\n"
    );
    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": skill_md }))
        .await
        .assert_status(StatusCode::CREATED);
    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    server
        .delete(&format!("/v1/skills/{skill_id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .post(&format!("/v1/skills/{skill_id}/delete"), json!({}))
        .await
        .assert_success();

    let content_resp = server.get(&format!("/v1/skills/{skill_id}/content")).await;
    assert_eq!(content_resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_skill() {
    let server = TestServer::new().await;

    let name = unique_skill_name("test-skill");
    let create_resp = server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name, "A test skill for integration testing.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let updated_md =
        format!("---\nname: {name}\ndescription: Updated description.\n---\n\n# Updated\n");
    let update_resp = server
        .patch(
            &format!("/v1/skills/{skill_id}"),
            json!({ "skill_md": updated_md }),
        )
        .await
        .assert_success();

    let body = update_resp.json_value();
    assert_eq!(body["description"], "Updated description.");
}

#[tokio::test]
async fn test_update_skill_preserves_user_invocable_false() {
    let server = TestServer::new().await;

    let name = unique_skill_name("hidden-skill");
    let initial_md = format!(
        "---\nname: {name}\ndescription: Hidden command.\nuser-invocable: false\n---\n\n# Hidden\n"
    );
    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": initial_md }))
        .await
        .assert_status(StatusCode::CREATED);
    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let updated_md = format!(
        "---\nname: {name}\ndescription: Updated hidden command.\nuser-invocable: false\n---\n\n# Hidden Updated\n"
    );
    let update_resp = server
        .patch(
            &format!("/v1/skills/{skill_id}"),
            json!({ "skill_md": updated_md }),
        )
        .await
        .assert_success();

    let body = update_resp.json_value();
    assert_eq!(body["description"], "Updated hidden command.");
    assert_eq!(body["user_invocable"], false);
}

#[tokio::test]
async fn test_update_skill_preserves_disable_model_invocation_flag() {
    let server = TestServer::new().await;

    let name = unique_skill_name("model-invocation-flag-test");
    let initial_md = format!(
        "---
name: {name}
description: Test disable-model-invocation persistence.
disable-model-invocation: true
---

# Test
"
    );
    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": initial_md }))
        .await
        .assert_status(StatusCode::CREATED);

    let created_body = create_resp.json_value();
    assert_eq!(created_body["disable_model_invocation"], true);

    let skill_id = created_body["id"].as_str().unwrap().to_string();

    let updated_md = format!(
        "---
name: {name}
description: Updated description.
disable-model-invocation: true
---

# Updated
"
    );
    let update_resp = server
        .patch(
            &format!("/v1/skills/{skill_id}"),
            json!({ "skill_md": updated_md }),
        )
        .await
        .assert_success();

    let updated_body = update_resp.json_value();
    assert_eq!(updated_body["description"], "Updated description.");
    assert_eq!(updated_body["disable_model_invocation"], true);
}

#[tokio::test]
async fn test_update_skill_status() {
    let server = TestServer::new().await;

    let name = unique_skill_name("test-skill");
    let create_resp = server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name, "A test skill for integration testing.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let update_resp = server
        .patch(
            &format!("/v1/skills/{skill_id}"),
            json!({ "status": "disabled" }),
        )
        .await
        .assert_success();

    let body = update_resp.json_value();
    assert_eq!(body["status"], "disabled");
}

#[tokio::test]
async fn test_delete_skill() {
    let server = TestServer::new().await;

    let name = unique_skill_name("test-skill");
    let create_resp = server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name, "A test skill for integration testing.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    // DELETE archives the skill (soft delete); it stays GET-able as `archived`.
    server
        .delete(&format!("/v1/skills/{skill_id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .get(&format!("/v1/skills/{skill_id}"))
        .await
        .assert_status(StatusCode::OK);

    // POST /delete (destroy) permanently deletes the archived skill; only then
    // is it gone (status `deleted`, excluded from GET).
    server
        .post(&format!("/v1/skills/{skill_id}/delete"), json!({}))
        .await
        .assert_success();

    let get_resp = server.get(&format!("/v1/skills/{skill_id}")).await;
    assert_eq!(get_resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_validate_skill() {
    let server = TestServer::new().await;

    // Valid skill
    let resp = server
        .post("/v1/skills/validate", json!({ "skill_md": VALID_SKILL_MD }))
        .await
        .assert_success();

    let body = resp.json_value();
    assert_eq!(body["valid"], true);
    assert_eq!(body["name"], "test-skill");

    // Invalid skill (no frontmatter)
    let resp = server
        .post(
            "/v1/skills/validate",
            json!({ "skill_md": "no frontmatter here" }),
        )
        .await
        .assert_success();

    let body = resp.json_value();
    assert_eq!(body["valid"], false);
}

// ============================================================
// Error Cases
// ============================================================

#[tokio::test]
async fn test_create_skill_duplicate_name() {
    let server = TestServer::new().await;

    let md = skill_md(
        &unique_skill_name("test-skill"),
        "A test skill for integration testing.",
    );
    server
        .post("/v1/skills", json!({ "skill_md": md }))
        .await
        .assert_status(StatusCode::CREATED);

    // Duplicate name should fail
    let resp = server.post("/v1/skills", json!({ "skill_md": md })).await;

    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn test_create_skill_invalid_md() {
    let server = TestServer::new().await;

    let resp = server
        .post("/v1/skills", json!({ "skill_md": "no valid frontmatter" }))
        .await;

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_create_skill_invalid_name() {
    let server = TestServer::new().await;

    let invalid_md = "---\nname: INVALID_NAME\ndescription: Bad name format.\n---\n\n# Bad\n";
    let resp = server
        .post("/v1/skills", json!({ "skill_md": invalid_md }))
        .await;

    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn test_get_nonexistent_skill() {
    let server = TestServer::new().await;

    let resp = server
        .get("/v1/skills/skill_00000000000000000000000000000000")
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ============================================================
// Skills as Capabilities Integration
// ============================================================

#[tokio::test]
async fn test_skill_appears_in_capabilities_listing() {
    let server = TestServer::new().await;

    // Create a skill
    let name = unique_skill_name("test-skill");
    server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name, "A test skill for integration testing.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    // List capabilities - skill should appear
    let resp = server.get("/v1/capabilities").await.assert_success();
    let body = resp.json_value();
    let capabilities = body["data"].as_array().unwrap();

    let skill_caps: Vec<&serde_json::Value> = capabilities
        .iter()
        .filter(|c| c["is_skill"].as_bool() == Some(true))
        .collect();

    assert!(
        !skill_caps.is_empty(),
        "Skill should appear in capabilities"
    );
    let skill_cap = skill_caps
        .iter()
        .find(|c| c["name"].as_str() == Some(name.as_str()))
        .expect("Should find the created skill in capabilities");

    assert_eq!(skill_cap["category"], "Skills");
    assert!(skill_cap["id"].as_str().unwrap().starts_with("skill:"));
    assert_eq!(
        skill_cap["description"],
        "A test skill for integration testing."
    );
}

#[tokio::test]
async fn test_disabled_skill_hidden_from_capabilities() {
    let server = TestServer::new().await;

    // Create and disable a skill
    let name = unique_skill_name("test-skill");
    let create_resp = server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name, "A test skill for integration testing.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    server
        .patch(
            &format!("/v1/skills/{skill_id}"),
            json!({ "status": "disabled" }),
        )
        .await
        .assert_success();

    // List capabilities - disabled skill should NOT appear
    let resp = server.get("/v1/capabilities").await.assert_success();
    let body = resp.json_value();
    let capabilities = body["data"].as_array().unwrap();

    let skill_caps: Vec<&serde_json::Value> = capabilities
        .iter()
        .filter(|c| c["name"].as_str() == Some(name.as_str()))
        .collect();

    assert!(
        skill_caps.is_empty(),
        "Disabled skill should not appear in capabilities"
    );
}

#[tokio::test]
async fn test_agent_with_skill_capability() {
    let server = TestServer::new().await;

    // Create a skill
    let name = unique_skill_name("test-skill");
    let create_resp = server
        .post(
            "/v1/skills",
            json!({ "skill_md": skill_md(&name, "A test skill for integration testing.") }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let _skill = create_resp.json_value();
    // The ID from the skills API is the public_id (skill_xxx format)
    // But capability IDs use skill:{uuid} format
    // We need to extract the UUID from the listing
    let caps_resp = server.get("/v1/capabilities").await.assert_success();
    let capabilities = caps_resp.json_value()["data"].as_array().unwrap().clone();
    let skill_cap = capabilities
        .iter()
        .find(|c| c["name"].as_str() == Some(name.as_str()))
        .expect("Skill capability should exist");
    let cap_id = skill_cap["id"].as_str().unwrap();

    // Create agent with skill capability (agent names are unique per org, so
    // derive a unique name to avoid cross-test collisions on the shared DB).
    let agent_name = unique_skill_name("skill-agent");
    let agent_resp = server
        .post(
            "/v1/agents",
            json!({
                "name": agent_name,
                "display_name": "Skill Agent",
                "system_prompt": "You are a helpful assistant.",
                "capabilities": [
                    { "ref": cap_id, "config": {} }
                ]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let agent = agent_resp.json_value();
    assert_eq!(agent["name"], agent_name);

    // Verify capabilities include the skill
    let agent_caps = agent["capabilities"].as_array().unwrap();
    assert!(
        agent_caps.iter().any(|c| c["ref"].as_str() == Some(cap_id)),
        "Agent should have skill capability assigned"
    );
}

#[tokio::test]
async fn test_deleted_skill_hidden_from_usage() {
    let server = TestServer::new().await;

    let skill_name = format!(
        "deleted-usage-skill-{}",
        &uuid::Uuid::now_v7().simple().to_string()[..8]
    );
    let skill_md = format!(
        "---\nname: {skill_name}\ndescription: Deleted usage visibility test.\n---\n\n# Deleted Usage Skill\n"
    );
    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": skill_md }))
        .await
        .assert_status(StatusCode::CREATED);
    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let caps_resp = server.get("/v1/capabilities").await.assert_success();
    let capabilities = caps_resp.json_value()["data"].as_array().unwrap().clone();
    let skill_cap = capabilities
        .iter()
        .find(|c| c["name"].as_str() == Some(skill_name.as_str()))
        .expect("Skill capability should exist");
    let cap_id = skill_cap["id"].as_str().unwrap().to_string();
    let agent_name = format!("deleted-skill-usage-agent-{}", &skill_id[6..14]);

    server
        .post(
            "/v1/agents",
            json!({
                "name": agent_name,
                "display_name": "Deleted Skill Usage Agent",
                "system_prompt": "You are a helpful assistant.",
                "capabilities": [
                    { "ref": cap_id, "config": {} }
                ]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let usage_before = server.get("/v1/skills/usage").await.assert_success();
    let usage_before_body = usage_before.json_value();
    assert_eq!(usage_before_body[&skill_id]["agents"], 1);

    server
        .delete(&format!("/v1/skills/{skill_id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    server
        .post(&format!("/v1/skills/{skill_id}/delete"), json!({}))
        .await
        .assert_success();

    let usage_after = server.get("/v1/skills/usage").await.assert_success();
    let usage_after_body = usage_after.json_value();
    assert!(usage_after_body.get(&skill_id).is_none());
}
