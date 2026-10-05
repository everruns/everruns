//! Integration tests for generic command dispatch over HTTP
//! (`GET /v1/commands`, `POST /v1/commands/{name}`).
//!
//! The endpoint must behave as the scripted surfaces do: same contract, same
//! policy and feature gates, typed errors mapped to Problem Details.
//!
//! Run with: cargo test -p everruns-server --test domain command_dispatch_test::

use crate::test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        uuid::Uuid::now_v7().simple().to_string()[20..].to_owned()
    )
}

#[tokio::test]
async fn catalog_lists_the_contract_and_hides_internal_commands() {
    let server = TestServer::in_memory().await;
    let body: Value = server
        .get("/v1/commands")
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(body["api_version"], "v1");
    let commands = body["commands"].as_array().expect("commands array");
    let create = commands
        .iter()
        .find(|command| command["wire_name"] == "create_agent")
        .expect("create_agent is in the contract");
    assert_eq!(create["path"], json!(["agents"]));
    assert_eq!(create["verb"], "create");
    assert!(
        commands.iter().all(|command| !command["http_path"]
            .as_str()
            .unwrap_or_default()
            .starts_with("/v1/durable/")),
        "durable internals are not part of the contract"
    );
}

#[tokio::test]
async fn catalog_omits_commands_whose_feature_is_off() {
    let server = TestServer::in_memory().await;
    server
        .db
        .replace_org_feature_flags(
            everruns_core::DEFAULT_ORG_ID,
            &std::collections::HashMap::from([("skills".to_string(), false)]),
        )
        .await
        .expect("disable skills");

    let body: Value = server.get("/v1/commands").await.assert_success().json();
    let commands = body["commands"].as_array().expect("commands array");
    assert!(
        commands
            .iter()
            .all(|command| command["wire_name"] != "list_skills"),
        "list_skills must be hidden while skills is off"
    );

    let error: Value = server
        .post("/v1/commands/list_skills", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(error["code"], "feature_not_enabled");
}

#[tokio::test]
async fn dispatch_creates_reads_and_lists_an_agent() {
    let server = TestServer::in_memory().await;
    let name = unique("dispatch");

    let created: Value = server
        .post(
            "/v1/commands/create_agent",
            json!({ "params": { "name": name, "system_prompt": "You tell short jokes." } }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(created["command"], "create_agent");
    assert_eq!(created["api_version"], "v1");
    assert!(created["warnings"].as_array().is_some_and(Vec::is_empty));
    let created = &created["output"];
    let id = created["id"].as_str().expect("agent id").to_owned();
    assert_eq!(created["name"], name);

    let fetched: Value = server
        .post("/v1/commands/get_agent", json!({ "params": { "id": id } }))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let fetched = &fetched["output"];
    assert_eq!(fetched["id"], id);
    assert_eq!(fetched["system_prompt"], "You tell short jokes.");

    let listed: Value = server
        .post(
            "/v1/commands/list_agents",
            json!({ "params": { "search": name } }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(
        listed.to_string().contains(&id),
        "list_agents output should include the new agent: {listed}"
    );
}

#[tokio::test]
async fn dispatch_accepts_json_fields_as_text_like_the_scripted_surface() {
    let server = TestServer::in_memory().await;
    let name = unique("dispatch-tags");

    // A shell argument is text; a client that forwards it unparsed must get
    // the same result as one that sends structured JSON.
    let created: Value = server
        .post(
            "/v1/commands/create_agent",
            json!({ "params": {
                "name": name,
                "system_prompt": "p",
                "capabilities": "[]"
            } }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(created["output"]["name"], name);
}

#[tokio::test]
async fn unknown_command_is_not_found() {
    let server = TestServer::in_memory().await;
    let error: Value = server
        .post("/v1/commands/no_such_command", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert!(
        error["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("no_such_command"),
        "{error}"
    );
}

#[tokio::test]
async fn internal_commands_are_not_dispatchable() {
    let server = TestServer::in_memory().await;
    // Durable schedules are control-plane internals (`/v1/durable/...`).
    server
        .post("/v1/commands/list_schedules", json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn invalid_params_are_a_bad_request() {
    let server = TestServer::in_memory().await;

    // Missing the required system_prompt.
    server
        .post(
            "/v1/commands/create_agent",
            json!({ "params": { "name": unique("missing") } }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    // Params must be an object.
    server
        .post(
            "/v1/commands/list_agents",
            json!({ "params": ["not", "an", "object"] }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn catalog_entries_carry_version_and_schema_hash() {
    let server = TestServer::in_memory().await;
    let body: Value = server.get("/v1/commands").await.assert_success().json();
    let create = body["commands"]
        .as_array()
        .expect("commands array")
        .iter()
        .find(|command| command["wire_name"] == "create_agent")
        .expect("create_agent")
        .clone();
    assert_eq!(create["api_version"], "v1");
    assert_eq!(create["read_only"], false);
    let hash = create["schema_hash"]
        .as_str()
        .expect("schema_hash")
        .to_owned();
    assert_eq!(hash.len(), 16);

    // The current hash is echoed back without a warning.
    let fresh: Value = server
        .post(
            "/v1/commands/list_agents",
            json!({ "params": {}, "schema_hash": list_agents_hash(&body), "metadata": { "client": "test" } }),
        )
        .await
        .assert_success()
        .json();
    assert!(
        fresh["warnings"].as_array().is_some_and(Vec::is_empty),
        "{fresh}"
    );

    // A stale hash still runs, and says so.
    let stale: Value = server
        .post(
            "/v1/commands/list_agents",
            json!({ "params": {}, "schema_hash": "0000000000000000" }),
        )
        .await
        .assert_success()
        .json();
    assert_eq!(
        stale["warnings"].as_array().map(Vec::len),
        Some(1),
        "{stale}"
    );
}

fn list_agents_hash(catalog: &Value) -> String {
    catalog["commands"]
        .as_array()
        .and_then(|commands| commands.iter().find(|c| c["wire_name"] == "list_agents"))
        .and_then(|command| command["schema_hash"].as_str())
        .expect("list_agents hash")
        .to_owned()
}

#[tokio::test]
async fn envelope_rejects_unknown_versions_and_fields() {
    let server = TestServer::in_memory().await;
    server
        .post(
            "/v1/commands/list_agents",
            json!({ "params": {}, "api_version": "v2" }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    // A field the envelope does not define, such as bare params, is rejected
    // rather than silently ignored.
    let response = server
        .post("/v1/commands/list_agents", json!({ "search": "x" }))
        .await;
    assert!(response.status().is_client_error(), "{}", response.text());
}
