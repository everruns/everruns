//! Skills integration tests
//!
//! Tests the skills CRUD API and the integration of skills as virtual capabilities.

mod test_harness;

use axum::http::StatusCode;
use serde_json::json;
use test_harness::TestServer;

const VALID_SKILL_MD: &str = "---\nname: test-skill\ndescription: A test skill for integration testing.\n---\n\n# Test Skill\n\n## Instructions\n\n1. Read the input data\n2. Process it according to the rules\n3. Return the result\n";

const VALID_SKILL_MD_2: &str = "---\nname: data-analysis\ndescription: Analyze datasets and generate reports.\n---\n\n# Data Analysis\n\nUse pandas to analyze the provided dataset.\n";

// ------------------------------------------------------------
// Skill CRUD Tests
// ------------------------------------------------------------

#[tokio::test]
async fn test_create_skill() {
    let server = TestServer::new().await;

    let resp = server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
        .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.json_value();
    assert_eq!(body["name"], "test-skill");
    assert_eq!(body["description"], "A test skill for integration testing.");
    assert_eq!(body["source_type"], "markdown");
    assert_eq!(body["status"], "active");
    assert!(body["id"].as_str().unwrap().starts_with("skill_"));
}

#[tokio::test]
async fn test_list_skills() {
    let server = TestServer::new().await;

    // Create two skills
    server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
        .await
        .assert_status(StatusCode::CREATED);

    server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD_2 }))
        .await
        .assert_status(StatusCode::CREATED);

    // List skills
    let resp = server.get("/v1/skills").await.assert_success();
    let body = resp.json_value();
    let skills = body["data"].as_array().unwrap();
    assert!(skills.len() >= 2);

    let names: Vec<&str> = skills.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(names.contains(&"test-skill"));
    assert!(names.contains(&"data-analysis"));
}

#[tokio::test]
async fn test_get_skill() {
    let server = TestServer::new().await;

    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let get_resp = server
        .get(&format!("/v1/skills/{skill_id}"))
        .await
        .assert_success();

    let body = get_resp.json_value();
    assert_eq!(body["name"], "test-skill");
    assert_eq!(body["id"], skill_id);
}

#[tokio::test]
async fn test_get_skill_content() {
    let server = TestServer::new().await;

    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let content_resp = server
        .get(&format!("/v1/skills/{skill_id}/content"))
        .await
        .assert_success();

    let body = content_resp.json_value();
    assert!(body["skill_md"].as_str().unwrap().contains("# Test Skill"));
}

#[tokio::test]
async fn test_update_skill() {
    let server = TestServer::new().await;

    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    let updated_md = "---\nname: test-skill\ndescription: Updated description.\n---\n\n# Updated\n";
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
async fn test_update_skill_status() {
    let server = TestServer::new().await;

    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
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

    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
        .await
        .assert_status(StatusCode::CREATED);

    let skill_id = create_resp.json_value()["id"].as_str().unwrap().to_string();

    // Delete the skill
    server
        .delete(&format!("/v1/skills/{skill_id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    // Verify it's gone
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

// ------------------------------------------------------------
// Error Cases
// ------------------------------------------------------------

#[tokio::test]
async fn test_create_skill_duplicate_name() {
    let server = TestServer::new().await;

    server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
        .await
        .assert_status(StatusCode::CREATED);

    // Duplicate name should fail
    let resp = server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
        .await;

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

// ------------------------------------------------------------
// Skills as Capabilities Integration
// ------------------------------------------------------------

#[tokio::test]
async fn test_skill_appears_in_capabilities_listing() {
    let server = TestServer::new().await;

    // Create a skill
    server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
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
        .find(|c| c["name"].as_str() == Some("test-skill"))
        .expect("Should find test-skill in capabilities");

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
    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
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
        .filter(|c| c["name"].as_str() == Some("test-skill"))
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
    let create_resp = server
        .post("/v1/skills", json!({ "skill_md": VALID_SKILL_MD }))
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
        .find(|c| c["name"].as_str() == Some("test-skill"))
        .expect("Skill capability should exist");
    let cap_id = skill_cap["id"].as_str().unwrap();

    // Create agent with skill capability
    let agent_resp = server
        .post(
            "/v1/agents",
            json!({
                "name": "Skill Agent",
                "system_prompt": "You are a helpful assistant.",
                "capabilities": [
                    { "ref": cap_id, "config": {} }
                ]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let agent = agent_resp.json_value();
    assert_eq!(agent["name"], "Skill Agent");

    // Verify capabilities include the skill
    let agent_caps = agent["capabilities"].as_array().unwrap();
    assert!(
        agent_caps.iter().any(|c| c["ref"].as_str() == Some(cap_id)),
        "Agent should have skill capability assigned"
    );
}
