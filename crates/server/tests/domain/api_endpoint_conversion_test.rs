//! Agent Execution API phase 6: retired `api_endpoint` channels convert to
//! `api` channels, and their `evr_app_` keys keep working as imported agent
//! keys on the Agent API.

use crate::agent_api_channel_test::{ApiChannel, api_channel, call, text_message};
use crate::test_harness;

use axum::http::{Method, StatusCode};
use everruns_contracts::typed_id::SessionId;
use everruns_server::domains::agent_channels::api_endpoint_conversion::{
    ConversionReport, IMPORTED_KEY_NAME, convert_api_endpoint_channels,
};
use everruns_server::domains::agent_channels::queries::prepare_channel_config;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use test_harness::TestServer;

const DOCS: &str = "https://docs.everruns.com/features/channels/#agent-api";

/// An `api_endpoint` channel as the retired handler stored it: an api
/// channel rewritten to the old type and config. Returns its key.
async fn legacy_endpoint(
    server: &TestServer,
    channel: &ApiChannel,
    session_mode: &str,
    auth: Option<Value>,
    encrypted: bool,
) -> String {
    let key = format!("evr_app_{}", uuid::Uuid::new_v4().simple());
    let mut config = json!({
        "api_key_hash": hex::encode(Sha256::digest(key.as_bytes())),
        "api_key_prefix": format!("{}...", &key[..12]),
        "session_mode": session_mode,
        "rate_limit_per_minute": 120,
    });
    if let Some(auth) = auth {
        config["auth"] = auth;
    }
    let (plaintext, ciphertext) = if encrypted {
        let (plaintext, ciphertext) =
            prepare_channel_config(server.encryption.as_ref(), &config).unwrap();
        assert!(ciphertext.is_some(), "the test server encrypts");
        (plaintext, ciphertext)
    } else {
        (config, None)
    };
    sqlx::query(
        "UPDATE agent_channels
         SET channel_type = 'api_endpoint', channel_config = $2, channel_config_encrypted = $3
         WHERE public_id = $1",
    )
    .bind(&channel.channel_id)
    .bind(plaintext)
    .bind(ciphertext)
    .execute(server.db.pool())
    .await
    .unwrap();
    key
}

/// A session the retired handler started on `channel`: owned by the
/// channel and carrying its App-era routing tags.
async fn legacy_session(server: &TestServer, channel: &ApiChannel) -> String {
    let session: Value = server
        .post("/v1/sessions", json!({ "agent_id": channel.agent_id }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = session["id"].as_str().unwrap().to_string();
    sqlx::query(
        "UPDATE sessions
         SET channel_id = (SELECT id FROM agent_channels WHERE public_id = $2),
             tags = ARRAY['app:legacy', 'app_channel:' || $2, 'app_channel_type:api_endpoint']
         WHERE id = $1",
    )
    .bind(id.parse::<SessionId>().unwrap().uuid())
    .bind(&channel.channel_id)
    .execute(server.db.pool())
    .await
    .unwrap();
    id
}

async fn channel_row(server: &TestServer, channel: &ApiChannel) -> Value {
    server
        .get(&format!(
            "/v1/agents/{}/channels/{}",
            channel.agent_id, channel.channel_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json()
}

async fn keys(server: &TestServer, channel: &ApiChannel) -> Vec<Value> {
    server
        .get(&format!(
            "/v1/agents/{}/channels/{}/keys",
            channel.agent_id, channel.channel_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json()
}

fn assert_retired(problem: &Value) {
    assert_eq!(problem["code"], "api_endpoint_retired", "{problem}");
    assert_eq!(problem["type"], DOCS, "{problem}");
    assert_eq!(problem["allowed_actions"][0]["href"], DOCS, "{problem}");
}

/// The old key, the old URL and the old sessions after conversion.
async fn assert_converted(server: &TestServer, channel: &ApiChannel, key: &str, legacy: &str) {
    let row = channel_row(server, channel).await;
    assert_eq!(row["channel_type"], "api");
    assert_eq!(row["status"], "live");
    assert_eq!(row["channel_config"]["visibility"], "messages");
    assert_eq!(row["channel_config"]["rate_limit_per_minute"], 120);
    assert!(row["channel_config"].get("api_key_hash").is_none());

    let keys = keys(server, channel).await;
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert_eq!(keys[0]["name"], IMPORTED_KEY_NAME);
    assert_eq!(keys[0]["prefix"], format!("{}...", &key[..12]));

    let base = format!("/v1/channels/{}", channel.channel_id);
    // The old key starts a session on the Agent API.
    let created: Value = call(
        server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(key),
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let created = created["id"].as_str().unwrap().to_string();
    call(
        server,
        Method::POST,
        &format!("{base}/sessions/{created}/messages"),
        Some(key),
        Some(text_message("hello")),
    )
    .await
    .assert_status(StatusCode::CREATED);

    // The old wire shapes are refused with a pointer, not half-served.
    let problem: Value = call(
        server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(key),
        Some(json!({ "message": "hello agent" })),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST)
    .json();
    assert_retired(&problem);
    let problem: Value = call(
        server,
        Method::POST,
        &format!("{base}/sessions/{created}/messages"),
        Some(key),
        Some(json!({ "message": "follow up" })),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST)
    .json();
    assert_retired(&problem);

    // The session the old handler started belongs to the imported key.
    call(
        server,
        Method::GET,
        &format!("{base}/sessions/{legacy}"),
        Some(key),
        None,
    )
    .await
    .assert_status(StatusCode::OK);
    let listed: Value = call(
        server,
        Method::GET,
        &format!("{base}/sessions"),
        Some(key),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let ids: Vec<&str> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|session| session["id"].as_str())
        .collect();
    assert!(ids.contains(&legacy), "{ids:?}");
    assert!(ids.contains(&created.as_str()), "{ids:?}");

    // A wrong key of the same shape is still refused.
    call(
        server,
        Method::POST,
        &format!("{base}/sessions"),
        Some("evr_app_not-the-key"),
        None,
    )
    .await
    .assert_status(StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn api_endpoint_channels_convert_and_their_keys_keep_working() {
    let server = TestServer::in_memory().await;

    let plain = api_channel(&server, json!({}), true).await;
    let plain_key = legacy_endpoint(&server, &plain, "shared_session", None, false).await;
    let plain_session = legacy_session(&server, &plain).await;

    let sealed = api_channel(&server, json!({}), true).await;
    let sealed_key = legacy_endpoint(
        &server,
        &sealed,
        "session_per_invocation",
        Some(json!({ "mode": "api_key" })),
        true,
    )
    .await;
    let sealed_session = legacy_session(&server, &sealed).await;

    let open = api_channel(&server, json!({}), true).await;
    let open_key = legacy_endpoint(
        &server,
        &open,
        "shared_session",
        Some(json!({ "mode": "anonymous" })),
        true,
    )
    .await;

    let report = convert_api_endpoint_channels(&server.db, server.encryption.as_ref())
        .await
        .unwrap();
    assert_eq!(
        report,
        ConversionReport {
            converted: 2,
            disabled: 1,
            skipped: 0
        }
    );
    // A second pass, or a second replica, finds nothing left to do.
    assert_eq!(
        convert_api_endpoint_channels(&server.db, server.encryption.as_ref())
            .await
            .unwrap(),
        ConversionReport::default()
    );

    assert_converted(&server, &plain, &plain_key, &plain_session).await;
    assert_eq!(
        channel_row(&server, &plain).await["channel_config"]["session_binding"],
        "per_user"
    );
    assert_converted(&server, &sealed, &sealed_key, &sealed_session).await;
    assert_eq!(
        channel_row(&server, &sealed).await["channel_config"]["session_binding"],
        "session_per_invocation"
    );

    // An anonymous endpoint has no Agent API equivalent: it converts
    // disabled, imports nothing, and its key opens nothing.
    let row = channel_row(&server, &open).await;
    assert_eq!(row["channel_type"], "api");
    assert_eq!(row["status"], "disabled");
    assert!(keys(&server, &open).await.is_empty());
    call(
        &server,
        Method::POST,
        &format!("/v1/channels/{}/sessions", open.channel_id),
        Some(&open_key),
        None,
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn retired_api_endpoint_surfaces_point_to_the_agent_api() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), false).await;

    // The type can no longer be created.
    let created = server
        .post(
            &format!("/v1/agents/{}/channels", channel.agent_id),
            json!({
                "channel_type": "api_endpoint",
                "channel_config": { "api_key_hash": "ab", "api_key_prefix": "evr_app_" },
            }),
        )
        .await;
    assert!(
        created.status().is_client_error(),
        "{}: {}",
        created.status(),
        created.text()
    );

    // The App-scoped aliases are gone, whatever they name.
    for (method, path) in [
        (Method::POST, "/v1/apps/app_x/api/appchan_y/sessions"),
        (
            Method::GET,
            "/v1/apps/app_x/api/appchan_y/sessions/session_z",
        ),
        (
            Method::POST,
            "/v1/apps/app_x/api/appchan_y/sessions/session_z/messages",
        ),
        (
            Method::POST,
            "/v1/apps/app_x/api/appchan_y/sessions/session_z/cancel",
        ),
    ] {
        let problem: Value = call(&server, method, path, Some("evr_app_key"), None)
            .await
            .assert_status(StatusCode::GONE)
            .json();
        assert_retired(&problem);
    }
}
