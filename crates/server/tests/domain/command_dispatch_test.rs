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
async fn connection_command_matches_console_virtual_user_connections() {
    use everruns_core::DEFAULT_ORG_ID;
    use everruns_server::domains::organizations::record::ANONYMOUS_USER_ID;
    use everruns_server::storage::CreateVirtualUserConnectionRow;

    let server = TestServer::in_memory().await;
    // Resolve through the console before seeding the same runtime account.
    server
        .get("/v1/virtual-users/me/connections")
        .await
        .assert_status(StatusCode::OK);
    let runtime_user = server
        .db
        .default_virtual_user(DEFAULT_ORG_ID, ANONYMOUS_USER_ID)
        .await
        .expect("default runtime account");
    assert_ne!(runtime_user.id.uuid(), ANONYMOUS_USER_ID);
    server
        .db
        .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
            virtual_user_id: runtime_user.id,
            provider: "github".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: None,
            provider_username: Some("connected-account".to_string()),
            scopes: Some("contents:read".to_string()),
            access_token_encrypted: Some(vec![1, 2, 3]),
            refresh_token_encrypted: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .expect("seed runtime connection");

    let console: Value = server
        .get("/v1/virtual-users/me/connections")
        .await
        .assert_status(StatusCode::OK)
        .json();
    let command: Value = server
        .post("/v1/commands/list_user_connections", json!({"params": {}}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(console.as_array().unwrap().len(), 1);
    let connections = command["output"].as_array().expect("command connections");
    assert_eq!(
        connections.len(),
        1,
        "console connected, command: {command}"
    );
    for field in [
        "provider",
        "connection_type",
        "provider_username",
        "connected_at",
    ] {
        assert_eq!(connections[0][field], console[0][field]);
    }
    assert_eq!(connections[0]["ui_link"], "/settings/connections");
    assert!(connections[0].get("access_token_encrypted").is_none());
    assert!(connections[0].get("scopes").is_none());
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

async fn post_with_key(
    server: &TestServer,
    uri: &str,
    key: &str,
    body: Value,
) -> test_harness::TestResponse {
    server
        .request_raw(
            axum::http::Method::POST,
            uri,
            vec![
                ("content-type", "application/json"),
                ("idempotency-key", key),
            ],
            serde_json::to_vec(&body).expect("body"),
        )
        .await
}

async fn agents_named(server: &TestServer, name: &str) -> usize {
    let body: Value = server
        .post(
            "/v1/commands/list_agents",
            json!({ "params": { "search": name } }),
        )
        .await
        .assert_success()
        .json();
    body["output"]["data"]
        .as_array()
        .map(|agents| agents.iter().filter(|agent| agent["name"] == name).count())
        .unwrap_or_default()
}

async fn idempotency_key_replays_the_first_response(server: TestServer) {
    let name = unique("idem");
    let key = unique("key");
    let params = json!({ "params": { "name": name, "system_prompt": "Be brief" } });

    let first = post_with_key(&server, "/v1/commands/create_agent", &key, params).await;
    assert_eq!(first.status(), StatusCode::OK, "{}", first.text());
    assert!(first.headers().get("idempotent-replayed").is_none());
    let first: Value = first.json();

    // Field order is not part of the request's identity.
    let reordered = json!({ "params": { "system_prompt": "Be brief", "name": name } });
    let second = post_with_key(&server, "/v1/commands/create_agent", &key, reordered).await;
    assert_eq!(second.status(), StatusCode::OK, "{}", second.text());
    assert_eq!(second.headers()["idempotent-replayed"], "true");
    let second: Value = second.json();

    assert_eq!(second, first, "a retry returns the stored response");
    assert_eq!(
        agents_named(&server, &name).await,
        1,
        "the command ran once"
    );

    // At rest the response is sealed with the server's encryption key.
    use everruns_server::storage::command_idempotency::{
        ClaimIdempotencyKey, IdempotencyClaim, IdempotencyKeyScope,
    };
    let now = chrono::Utc::now();
    let IdempotencyClaim::Existing(stored) = server
        .db
        .claim_command_idempotency_key(&ClaimIdempotencyKey {
            scope: IdempotencyKeyScope {
                org_id: everruns_core::DEFAULT_ORG_ID,
                principal_id: everruns_server::domains::organizations::record::ANONYMOUS_USER_ID,
                key,
            },
            command: "create_agent".into(),
            fingerprint: String::new(),
            locked_until: now,
            expires_at: now + chrono::Duration::hours(1),
        })
        .await
        .expect("claim")
    else {
        panic!("the key is taken");
    };
    let sealed = stored.response.expect("stored response");
    assert!(
        !String::from_utf8_lossy(&sealed).contains(name.as_str()),
        "the stored response is encrypted"
    );
}

#[tokio::test]
async fn idempotency_key_replays_the_first_response_in_memory() {
    idempotency_key_replays_the_first_response(TestServer::in_memory().await).await;
}

#[tokio::test]
async fn idempotency_key_replays_the_first_response_on_postgres() {
    idempotency_key_replays_the_first_response(TestServer::new().await).await;
}

#[tokio::test]
async fn idempotency_key_reused_for_another_request_is_unprocessable() {
    let server = TestServer::in_memory().await;
    let key = unique("key");
    post_with_key(
        &server,
        "/v1/commands/create_agent",
        &key,
        json!({ "params": { "name": unique("idem"), "system_prompt": "a" } }),
    )
    .await
    .assert_success();

    let other_name = unique("idem");
    let error: Value = post_with_key(
        &server,
        "/v1/commands/create_agent",
        &key,
        json!({ "params": { "name": other_name, "system_prompt": "a" } }),
    )
    .await
    .assert_status(StatusCode::UNPROCESSABLE_ENTITY)
    .json();
    assert_eq!(error["code"], "idempotency_key_reused");
    assert_eq!(agents_named(&server, &other_name).await, 0);
}

#[tokio::test]
async fn a_failed_command_releases_its_key() {
    let server = TestServer::in_memory().await;
    let key = unique("key");
    post_with_key(
        &server,
        "/v1/commands/create_agent",
        &key,
        json!({ "params": { "name": 7 } }),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);

    let name = unique("idem");
    let response = post_with_key(
        &server,
        "/v1/commands/create_agent",
        &key,
        json!({ "params": { "name": name, "system_prompt": "a" } }),
    )
    .await
    .assert_success();
    assert!(response.headers().get("idempotent-replayed").is_none());
    assert_eq!(agents_named(&server, &name).await, 1);
}

#[tokio::test]
async fn a_key_still_in_flight_is_a_conflict() {
    use everruns_server::api::command_dispatch::request_fingerprint;
    use everruns_server::storage::command_idempotency::{
        ClaimIdempotencyKey, IdempotencyClaim, IdempotencyKeyScope,
    };

    let server = TestServer::in_memory().await;
    let key = unique("key");
    let name = unique("idem");
    let params = json!({ "name": name, "system_prompt": "a" });
    // An earlier request with this key is still running: claim it as the
    // handler does, without completing it.
    let now = chrono::Utc::now();
    let claim = server
        .db
        .claim_command_idempotency_key(&ClaimIdempotencyKey {
            scope: IdempotencyKeyScope {
                org_id: everruns_core::DEFAULT_ORG_ID,
                principal_id: everruns_server::domains::organizations::record::ANONYMOUS_USER_ID,
                key: key.clone(),
            },
            command: "create_agent".into(),
            fingerprint: request_fingerprint("create_agent", &params),
            locked_until: now + chrono::Duration::minutes(10),
            expires_at: now + chrono::Duration::hours(1),
        })
        .await
        .expect("claim");
    assert_eq!(claim, IdempotencyClaim::Claimed);

    let error: Value = post_with_key(
        &server,
        "/v1/commands/create_agent",
        &key,
        json!({ "params": params }),
    )
    .await
    .assert_status(StatusCode::CONFLICT)
    .json();
    assert_eq!(error["code"], "idempotency_key_in_progress");
    assert_eq!(
        agents_named(&server, &name).await,
        0,
        "the retry did not run"
    );
}

#[tokio::test]
async fn read_only_commands_ignore_the_key_and_bad_keys_are_rejected() {
    let server = TestServer::in_memory().await;
    let key = unique("key");
    for _ in 0..2 {
        let response = post_with_key(&server, "/v1/commands/list_agents", &key, json!({})).await;
        assert_eq!(response.status(), StatusCode::OK, "{}", response.text());
        assert!(response.headers().get("idempotent-replayed").is_none());
    }

    let too_long = "k".repeat(256);
    for bad in ["", "has space", too_long.as_str()] {
        post_with_key(&server, "/v1/commands/create_agent", bad, json!({}))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

/// The claim rules both stores must agree on: a live lease is not taken over,
/// an abandoned one only by the same request, an expired key by anyone, and a
/// completed key survives release.
async fn claim_rules_hold(server: TestServer) {
    use chrono::{Duration, Utc};
    use everruns_server::storage::command_idempotency::{
        ClaimIdempotencyKey, IdempotencyClaim, IdempotencyKeyScope, StoredIdempotencyKey,
    };

    let scope = |key: &str| IdempotencyKeyScope {
        org_id: everruns_core::DEFAULT_ORG_ID,
        principal_id: uuid::Uuid::now_v7(),
        key: key.to_string(),
    };
    let claim = |scope: &IdempotencyKeyScope, fingerprint: &str, lock: Duration, ttl: Duration| {
        ClaimIdempotencyKey {
            scope: scope.clone(),
            command: "create_agent".into(),
            fingerprint: fingerprint.into(),
            locked_until: Utc::now() + lock,
            expires_at: Utc::now() + ttl,
        }
    };
    let db = &server.db;
    let live = Duration::minutes(5);
    let day = Duration::hours(24);

    let abandoned = scope("abandoned");
    let first = claim(&abandoned, "a", Duration::seconds(-1), day);
    assert_eq!(
        db.claim_command_idempotency_key(&first).await.unwrap(),
        IdempotencyClaim::Claimed
    );
    assert!(matches!(
        db.claim_command_idempotency_key(&claim(&abandoned, "b", live, day))
            .await
            .unwrap(),
        IdempotencyClaim::Existing(_)
    ));
    assert_eq!(
        db.claim_command_idempotency_key(&claim(&abandoned, "a", live, day))
            .await
            .unwrap(),
        IdempotencyClaim::Claimed
    );
    assert_eq!(
        db.claim_command_idempotency_key(&claim(&abandoned, "a", live, day))
            .await
            .unwrap(),
        IdempotencyClaim::Existing(StoredIdempotencyKey {
            command: "create_agent".into(),
            fingerprint: "a".into(),
            response: None,
        })
    );

    let expired = scope("expired");
    db.claim_command_idempotency_key(&claim(&expired, "a", live, Duration::seconds(-1)))
        .await
        .unwrap();
    assert_eq!(
        db.claim_command_idempotency_key(&claim(&expired, "b", live, day))
            .await
            .unwrap(),
        IdempotencyClaim::Claimed
    );

    let completed = scope("completed");
    let request = claim(&completed, "a", live, day);
    db.claim_command_idempotency_key(&request).await.unwrap();
    db.complete_command_idempotency_key(&completed, b"sealed")
        .await
        .unwrap();
    db.release_command_idempotency_key(&completed)
        .await
        .unwrap();
    assert_eq!(
        db.claim_command_idempotency_key(&request).await.unwrap(),
        IdempotencyClaim::Existing(StoredIdempotencyKey {
            command: "create_agent".into(),
            fingerprint: "a".into(),
            response: Some(b"sealed".to_vec()),
        })
    );

    let released = scope("released");
    let request = claim(&released, "a", live, day);
    db.claim_command_idempotency_key(&request).await.unwrap();
    db.release_command_idempotency_key(&released).await.unwrap();
    assert_eq!(
        db.claim_command_idempotency_key(&claim(&released, "b", live, day))
            .await
            .unwrap(),
        IdempotencyClaim::Claimed
    );
}

#[tokio::test]
async fn idempotency_claim_rules_hold_in_memory() {
    claim_rules_hold(TestServer::in_memory().await).await;
}

#[tokio::test]
async fn idempotency_claim_rules_hold_on_postgres() {
    claim_rules_hold(TestServer::new().await).await;
}
