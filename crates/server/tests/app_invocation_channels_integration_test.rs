//! Integration tests for app schedule/webhook invocation channels.

mod test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_core::DEFAULT_ORG_ID;
use everruns_server::api::common::Pagination;
use everruns_server::storage::SessionListFilters;

async fn create_app(
    server: &TestServer,
    name: &str,
    channel_type: &str,
    channel_config: Value,
) -> Value {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("{name}-agent"),
                "display_name": format!("{name} agent"),
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .seed_app_endpoint(
            name,
            agent["id"].as_str().unwrap(),
            channel_type,
            channel_config,
        )
        .await
}

async fn publish_app(server: &TestServer, app_id: &str) {
    server.set_app_endpoints_live(app_id, true).await;
}
#[tokio::test]
async fn webhook_legacy_app_channel_mismatch_is_not_found() {
    let server = TestServer::in_memory().await;
    let app_a = create_app(
        &server,
        "webhook-mismatch-a",
        "webhook",
        json!({
            "token": "secret-a",
            "session_mode": "shared_session",
            "message": "{{webhook.body}}",
        }),
    )
    .await;
    let app_b = create_app(
        &server,
        "webhook-mismatch-b",
        "webhook",
        json!({
            "token": "secret-b",
            "session_mode": "shared_session",
            "message": "{{webhook.body}}",
        }),
    )
    .await;
    let app_a_id = app_a["id"].as_str().unwrap();
    let app_b_id = app_b["id"].as_str().unwrap();
    let channel_b_id = app_b["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_a_id).await;
    publish_app(&server, app_b_id).await;

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_a_id}/webhooks/{channel_b_id}"),
            vec![("x-everruns-webhook-token", "secret-b")],
            b"hello".to_vec(),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

async fn list_app_sessions(server: &TestServer, app_id: &str, channel_id: &str) -> Vec<String> {
    let (rows, _) = server
        .db
        .list_sessions(
            DEFAULT_ORG_ID,
            &SessionListFilters::default(),
            Pagination {
                offset: 0,
                limit: 200,
            },
        )
        .await
        .expect("list sessions");

    let app_tag = format!("app:{app_id}");
    let channel_tag = format!("app_channel:{channel_id}");

    rows.into_iter()
        .filter(|row| row.tags.contains(&app_tag) && row.tags.contains(&channel_tag))
        .map(|row| row.id.to_string())
        .collect()
}

async fn list_user_message_texts(server: &TestServer, session_id: &str) -> Vec<String> {
    let body: Value = server
        .get(&format!("/v1/sessions/{session_id}/messages"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    body["data"]
        .as_array()
        .expect("messages array")
        .iter()
        .filter(|message| message["role"].as_str() == Some("user"))
        .filter_map(|message| {
            message["content"].as_array().and_then(|parts| {
                parts.iter().find_map(|part| {
                    (part["type"].as_str() == Some("text"))
                        .then(|| part["text"].as_str().map(str::to_owned))
                        .flatten()
                })
            })
        })
        .collect()
}

#[tokio::test]
async fn webhook_channel_shared_session_reuses_session_and_renders_template() {
    let server = TestServer::in_memory().await;
    let app = create_app(
        &server,
        "webhook-checker",
        "webhook",
        json!({
            "token": "secret-1",
            "session_mode": "shared_session",
            "message": "repo={{payload.repo.name}} action={{payload.action}} event={{webhook.headers.x-event}}",
        }),
    )
    .await;

    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let first: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("x-everruns-webhook-token", "secret-1"),
                ("x-event", "push"),
            ],
            serde_json::to_vec(&json!({
                "repo": { "name": "everruns" },
                "action": "opened"
            }))
            .unwrap(),
        )
        .await
        .assert_status(StatusCode::ACCEPTED)
        .json();

    let second: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("x-everruns-webhook-token", "secret-1"),
                ("x-event", "push"),
            ],
            serde_json::to_vec(&json!({
                "repo": { "name": "everruns" },
                "action": "synchronize"
            }))
            .unwrap(),
        )
        .await
        .assert_status(StatusCode::ACCEPTED)
        .json();

    assert!(first["created_session"].as_bool().unwrap());
    assert!(!second["created_session"].as_bool().unwrap());
    assert_eq!(first["session_id"], second["session_id"]);

    let texts = list_user_message_texts(&server, first["session_id"].as_str().unwrap()).await;
    assert!(
        texts
            .iter()
            .any(|text| text == "repo=everruns action=opened event=push")
    );
    assert!(
        texts
            .iter()
            .any(|text| text == "repo=everruns action=synchronize event=push")
    );
}

#[tokio::test]
async fn webhook_shared_session_does_not_reuse_user_seeded_tag_session() {
    // Defense in depth against an org member trying to hijack an app's shared
    // invocation session by pre-seeding one with the matching surface tags:
    //
    //   1. TM-AUTHZ-009 — the session create path rejects external callers
    //      that stamp `app:` / `app_channel:` / `slack:app:` tags. That gate
    //      makes the attack non-starter for normal API users.
    //
    //   2. `__internal:app_invocation` tag — even if such a session somehow
    //      existed, the lookup in `find_app_session_by_tags_and_owner` only
    //      matches sessions carrying this internal-only tag, which external
    //      callers cannot stamp (`__internal:` prefix is also reserved).
    //
    // This test pins both layers: it asserts the seed attempt is rejected
    // (layer 1) and that a subsequent legitimate webhook invocation creates a
    // fresh, app-owned session — never an attacker's session.
    let server = TestServer::in_memory().await;
    let app = create_app(
        &server,
        "webhook-protected-shared",
        "webhook",
        json!({
            "token": "secret-attack",
            "session_mode": "shared_session",
            "message": "repo={{payload.repo.name}}",
        }),
    )
    .await;

    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();

    let seed_response = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "title": "seeded attacker session",
                "tags": [
                    format!("app:{app_id}"),
                    format!("app_channel:{channel_id}"),
                    "app_channel_type:webhook",
                ],
            }),
        )
        .await;
    assert_eq!(
        seed_response.status(),
        StatusCode::BAD_REQUEST,
        "TM-AUTHZ-009 must reject external sessions stamping app:/app_channel: tags with 400"
    );
    let seed_body = seed_response.text();
    assert!(
        seed_body.contains("reserved for internal subsystems"),
        "TM-AUTHZ-009 rejection must explain the reservation; got: {seed_body}"
    );

    publish_app(&server, app_id).await;

    let invoked: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("x-everruns-webhook-token", "secret-attack"),
            ],
            serde_json::to_vec(&json!({
                "repo": { "name": "everruns" },
            }))
            .unwrap(),
        )
        .await
        .assert_status(StatusCode::ACCEPTED)
        .json();

    // Webhook always creates the canonical app-owned session; the attempted
    // pre-seed did not exist, so reuse is impossible.
    assert!(invoked["created_session"].as_bool().unwrap());
}

#[tokio::test]
async fn webhook_channel_per_invocation_rejects_bad_token_and_creates_new_sessions() {
    let server = TestServer::in_memory().await;
    let app = create_app(
        &server,
        "webhook-fanout",
        "webhook",
        json!({
            "token": "secret-2",
            "session_mode": "session_per_invocation",
            "message": "fanout {{payload.repo.name}}",
        }),
    )
    .await;

    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    server
        .request_raw(
            Method::POST,
            &format!("/v1/e/{channel_id}/webhook"),
            vec![
                ("content-type", "application/json"),
                ("x-everruns-webhook-token", "wrong"),
            ],
            serde_json::to_vec(&json!({ "repo": { "name": "ignored" } })).unwrap(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    let first: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("x-everruns-webhook-token", "secret-2"),
            ],
            serde_json::to_vec(&json!({ "repo": { "name": "alpha" } })).unwrap(),
        )
        .await
        .assert_status(StatusCode::ACCEPTED)
        .json();

    let second: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/e/{channel_id}/webhook"),
            vec![
                ("content-type", "application/json"),
                ("authorization", "Bearer secret-2"),
            ],
            serde_json::to_vec(&json!({ "repo": { "name": "beta" } })).unwrap(),
        )
        .await
        .assert_status(StatusCode::ACCEPTED)
        .json();

    assert!(first["created_session"].as_bool().unwrap());
    assert!(second["created_session"].as_bool().unwrap());
    assert_ne!(first["session_id"], second["session_id"]);

    let sessions = list_app_sessions(&server, app_id, channel_id).await;
    assert_eq!(sessions.len(), 2);
}
