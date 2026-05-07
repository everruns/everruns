//! Integration tests for app schedule/webhook invocation channels.

mod test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_core::DEFAULT_ORG_ID;
use everruns_server::api::common::Pagination;
use everruns_server::domains::apps::invoke_scheduled_app_channel;
use everruns_server::domains::messages::MessageService;
use everruns_server::domains::sessions::SessionService;
use everruns_server::event_delivery::EventDelivery;

async fn create_app(
    server: &TestServer,
    name: &str,
    channel_type: &str,
    channel_config: Value,
) -> Value {
    server
        .post(
            "/v1/apps",
            json!({
                "name": name,
                "description": "test app",
                "harness_id": server.seed_generic_harness_id.clone(),
                "channel_type": channel_type,
                "channel_config": channel_config,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

async fn publish_app(server: &TestServer, app_id: &str) {
    server
        .post(&format!("/v1/apps/{app_id}/publish"), json!({}))
        .await
        .assert_status(StatusCode::OK);
}

async fn unpublish_app(server: &TestServer, app_id: &str) {
    server
        .post(&format!("/v1/apps/{app_id}/unpublish"), json!({}))
        .await
        .assert_status(StatusCode::OK);
}

async fn find_schedule(server: &TestServer, app_id: &str, channel_id: &str) -> Option<Value> {
    let schedules: Value = server
        .get("/v1/durable/schedules")
        .await
        .assert_status(StatusCode::OK)
        .json();

    schedules["data"]
        .as_array()
        .and_then(|items| {
            items.iter().find(|item| {
                item["target"]["input"]["app_id"].as_str() == Some(app_id)
                    && item["target"]["input"]["channel_id"].as_str() == Some(channel_id)
            })
        })
        .cloned()
}

async fn list_app_sessions(server: &TestServer, app_id: &str, channel_id: &str) -> Vec<String> {
    let (rows, _) = server
        .db
        .list_sessions(
            DEFAULT_ORG_ID,
            None,
            None,
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
async fn schedule_channel_binds_to_durable_schedule_and_invokes_shared_session() {
    let server = TestServer::in_memory().await;
    let app = create_app(
        &server,
        "schedule-checker",
        "schedule",
        json!({
            "cron_expression": "0 * * * * * *",
            "timezone": "UTC",
            "session_mode": "shared_session",
            "message": "scheduled {{app.name}} {{invocation.source}}",
        }),
    )
    .await;

    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();

    let draft_schedule = find_schedule(&server, app_id, channel_id)
        .await
        .expect("draft schedule binding");
    assert_eq!(draft_schedule["target"]["type"], "activity");
    assert_eq!(
        draft_schedule["target"]["name"],
        "invoke_scheduled_app_channel"
    );
    assert_eq!(draft_schedule["enabled"], false);
    assert_eq!(draft_schedule["cron_expression"], "0 * * * * * *");

    publish_app(&server, app_id).await;

    let published_schedule = find_schedule(&server, app_id, channel_id)
        .await
        .expect("published schedule binding");
    assert_eq!(published_schedule["enabled"], true);

    let session_service = SessionService::new(server.db.clone());
    let message_service = MessageService::new(
        server.db.clone(),
        server.runner.clone(),
        false,
        EventDelivery::in_memory(),
    );

    let first = invoke_scheduled_app_channel(
        &server.db,
        server.encryption.as_ref(),
        &session_service,
        &message_service,
        DEFAULT_ORG_ID,
        app_id,
        channel_id,
    )
    .await
    .expect("first schedule invocation");
    let second = invoke_scheduled_app_channel(
        &server.db,
        server.encryption.as_ref(),
        &session_service,
        &message_service,
        DEFAULT_ORG_ID,
        app_id,
        channel_id,
    )
    .await
    .expect("second schedule invocation");

    assert!(first.created_session);
    assert!(!second.created_session);
    assert_eq!(first.session_id, second.session_id);

    let texts = list_user_message_texts(&server, &first.session_id.to_string()).await;
    assert!(
        texts
            .iter()
            .any(|text| text.contains("scheduled schedule-checker schedule"))
    );
    assert_eq!(texts.len(), 2);

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({
                "channel_config": {
                    "cron_expression": "0 15 * * * * *",
                    "timezone": "America/Chicago",
                    "session_mode": "shared_session",
                    "message": "updated {{app.id}}",
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    let updated_schedule = find_schedule(&server, app_id, channel_id)
        .await
        .expect("updated schedule binding");
    assert_eq!(updated_schedule["cron_expression"], "0 15 * * * * *");
    assert_eq!(updated_schedule["timezone"], "America/Chicago");

    server
        .delete(&format!("/v1/apps/{app_id}/channels/{channel_id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
    assert!(find_schedule(&server, app_id, channel_id).await.is_none());
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

    let seed_status = server
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
        .await
        .status();
    assert_ne!(
        seed_status,
        StatusCode::CREATED,
        "TM-AUTHZ-009 must reject external sessions stamping app:/app_channel: tags"
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
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
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
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
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

#[tokio::test]
async fn schedule_channel_per_invocation_tracks_publish_and_enable_state() {
    let server = TestServer::in_memory().await;
    let app = create_app(
        &server,
        "scheduled-fanout",
        "schedule",
        json!({
            "cron_expression": "0 6 * * * * *",
            "timezone": "UTC",
            "session_mode": "session_per_invocation",
            "message": "scheduled run {{app.id}}",
        }),
    )
    .await;

    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();

    let draft_schedule = find_schedule(&server, app_id, channel_id)
        .await
        .expect("draft schedule binding");
    assert_eq!(draft_schedule["enabled"], false);

    publish_app(&server, app_id).await;
    let published_schedule = find_schedule(&server, app_id, channel_id)
        .await
        .expect("published schedule binding");
    assert_eq!(published_schedule["enabled"], true);

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::OK);
    let disabled_schedule = find_schedule(&server, app_id, channel_id)
        .await
        .expect("disabled schedule binding");
    assert_eq!(disabled_schedule["enabled"], false);

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({ "enabled": true }),
        )
        .await
        .assert_status(StatusCode::OK);
    let reenabled_schedule = find_schedule(&server, app_id, channel_id)
        .await
        .expect("reenabled schedule binding");
    assert_eq!(reenabled_schedule["enabled"], true);

    unpublish_app(&server, app_id).await;
    let unpublished_schedule = find_schedule(&server, app_id, channel_id)
        .await
        .expect("unpublished schedule binding");
    assert_eq!(unpublished_schedule["enabled"], false);

    publish_app(&server, app_id).await;

    let session_service = SessionService::new(server.db.clone());
    let message_service = MessageService::new(
        server.db.clone(),
        server.runner.clone(),
        false,
        EventDelivery::in_memory(),
    );

    let first = invoke_scheduled_app_channel(
        &server.db,
        server.encryption.as_ref(),
        &session_service,
        &message_service,
        DEFAULT_ORG_ID,
        app_id,
        channel_id,
    )
    .await
    .expect("first schedule invocation");
    let second = invoke_scheduled_app_channel(
        &server.db,
        server.encryption.as_ref(),
        &session_service,
        &message_service,
        DEFAULT_ORG_ID,
        app_id,
        channel_id,
    )
    .await
    .expect("second schedule invocation");

    assert!(first.created_session);
    assert!(second.created_session);
    assert_ne!(first.session_id, second.session_id);

    let sessions = list_app_sessions(&server, app_id, channel_id).await;
    assert_eq!(sessions.len(), 2);
}

#[tokio::test]
async fn webhook_channel_enforces_publish_and_enable_and_supports_raw_body_templates() {
    let server = TestServer::in_memory().await;
    let app = create_app(
        &server,
        "raw-webhook",
        "webhook",
        json!({
            "token": "secret-raw",
            "session_mode": "shared_session",
            "message": "body={{payload}} raw={{webhook.body}} kind={{webhook.headers.x-kind}} auth={{webhook.headers.authorization}} token={{webhook.headers.x-everruns-webhook-token}}",
        }),
    )
    .await;

    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
            vec![
                ("content-type", "text/plain"),
                ("x-everruns-webhook-token", "secret-raw"),
            ],
            b"before publish".to_vec(),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);

    publish_app(&server, app_id).await;

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::OK);

    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
            vec![
                ("content-type", "text/plain"),
                ("x-everruns-webhook-token", "secret-raw"),
            ],
            b"disabled".to_vec(),
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({ "enabled": true }),
        )
        .await
        .assert_status(StatusCode::OK);

    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
            vec![
                ("content-type", "text/plain"),
                ("x-everruns-webhook-token", "secret-raw"),
                ("x-kind", "raw"),
                ("authorization", "Bearer super-secret"),
            ],
            b"plain body".to_vec(),
        )
        .await
        .assert_status(StatusCode::ACCEPTED)
        .json();

    let texts = list_user_message_texts(&server, response["session_id"].as_str().unwrap()).await;
    assert!(texts.iter().any(|text| {
        text == "body=plain body raw=plain body kind=raw auth=[REDACTED] token=[REDACTED]"
    }));

    let invalid_utf8: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/webhooks/{channel_id}"),
            vec![
                ("content-type", "application/octet-stream"),
                ("x-everruns-webhook-token", "secret-raw"),
                ("x-kind", "binary"),
            ],
            vec![0x66, 0x6f, 0x80],
        )
        .await
        .assert_status(StatusCode::ACCEPTED)
        .json();

    let texts =
        list_user_message_texts(&server, invalid_utf8["session_id"].as_str().unwrap()).await;
    assert!(
        texts
            .iter()
            .any(|text| text == "body=fo� raw=fo� kind=binary auth= token=[REDACTED]")
    );
}
