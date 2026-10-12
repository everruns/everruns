//! Agent Execution API safe retries (`Idempotency-Key`) and per-channel
//! browser origins (`cors_origins`).

use crate::agent_api_channel_test::{ApiChannel, api_channel, create_key, text_message};
use crate::test_harness;

use axum::http::{HeaderValue, Method, StatusCode};
use everruns_server::api::agent_api_cors::ApiChannelOrigins;
use serde_json::{Value, json};
use test_harness::TestServer;

async fn post(
    server: &TestServer,
    path: &str,
    key: &str,
    idempotency_key: Option<&str>,
    body: Value,
) -> test_harness::TestResponse {
    let authorization = format!("Bearer {key}");
    let mut headers = vec![
        ("content-type", "application/json"),
        ("authorization", authorization.as_str()),
    ];
    if let Some(idempotency_key) = idempotency_key {
        headers.push(("idempotency-key", idempotency_key));
    }
    server
        .request_raw(
            Method::POST,
            path,
            headers,
            serde_json::to_vec(&body).unwrap(),
        )
        .await
}

fn replayed(response: &test_harness::TestResponse) -> bool {
    response.headers().get("idempotent-replayed") == Some(&HeaderValue::from_static("true"))
}

#[tokio::test]
async fn a_retried_create_returns_the_first_session() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let key = create_key(&server, &channel, "Backend").await;
    let secret = key["secret"].as_str().unwrap();
    let sessions = format!("/v1/channels/{}/sessions", channel.channel_id);
    let body = json!({ "title": "Order 42" });

    let first = post(&server, &sessions, secret, Some("create-1"), body.clone())
        .await
        .assert_status(StatusCode::CREATED);
    assert!(!replayed(&first));
    let retry = post(&server, &sessions, secret, Some("create-1"), body.clone())
        .await
        .assert_status(StatusCode::CREATED);
    assert!(replayed(&retry));
    assert_eq!(first.json_value()["id"], retry.json_value()["id"]);
    assert_eq!(
        first.headers().get("location"),
        retry.headers().get("location")
    );

    // A different request under the same key is refused, not replayed.
    post(
        &server,
        &sessions,
        secret,
        Some("create-1"),
        json!({ "title": "Order 43" }),
    )
    .await
    .assert_status(StatusCode::UNPROCESSABLE_ENTITY);

    // Without a key, each request is its own.
    let a = post(&server, &sessions, secret, None, body.clone()).await;
    let b = post(&server, &sessions, secret, None, body.clone()).await;
    assert_ne!(a.json_value()["id"], b.json_value()["id"]);

    // Keys belong to their caller: another application's identical key is
    // a new request.
    let other = create_key(&server, &channel, "Other backend").await;
    let theirs = post(
        &server,
        &sessions,
        other["secret"].as_str().unwrap(),
        Some("create-1"),
        body,
    )
    .await
    .assert_status(StatusCode::CREATED);
    assert!(!replayed(&theirs));
    assert_ne!(theirs.json_value()["id"], first.json_value()["id"]);
}

#[tokio::test]
async fn a_retried_message_is_sent_once() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({ "visibility": "messages" }), true).await;
    let key = create_key(&server, &channel, "Backend").await;
    let secret = key["secret"].as_str().unwrap();
    let base = format!("/v1/channels/{}", channel.channel_id);
    let session = post(
        &server,
        &format!("{base}/sessions"),
        secret,
        None,
        json!({}),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json_value();
    let messages = format!(
        "{base}/sessions/{}/messages",
        session["id"].as_str().unwrap()
    );

    let first = post(
        &server,
        &messages,
        secret,
        Some("msg-1"),
        text_message("hi"),
    )
    .await
    .assert_status(StatusCode::CREATED);
    let retry = post(
        &server,
        &messages,
        secret,
        Some("msg-1"),
        text_message("hi"),
    )
    .await
    .assert_status(StatusCode::CREATED);
    assert!(replayed(&retry));
    assert_eq!(first.json_value()["id"], retry.json_value()["id"]);

    // A malformed key is refused before anything runs.
    post(&server, &messages, secret, Some(""), text_message("hi"))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_failed_request_frees_its_key() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let key = create_key(&server, &channel, "Backend").await;
    let secret = key["secret"].as_str().unwrap();
    let sessions = format!("/v1/channels/{}/sessions", channel.channel_id);

    post(
        &server,
        &sessions,
        secret,
        Some("create-2"),
        json!({ "metadata": { "a": "b" } }),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
    post(&server, &sessions, secret, Some("create-2"), json!({}))
        .await
        .assert_status(StatusCode::CREATED);
}

async fn create_channel(server: &TestServer, config: Value) -> test_harness::TestResponse {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("api-cors-{}", uuid::Uuid::new_v4().simple()),
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    server
        .post(
            &format!("/v1/agents/{}/channels", agent["id"].as_str().unwrap()),
            json!({ "channel_type": "api", "channel_config": config }),
        )
        .await
}

#[tokio::test]
async fn cors_origins_must_be_exact_browser_origins() {
    let server = TestServer::in_memory().await;
    for bad in [
        "https://app.example.com/",
        "https://app.example.com/chat",
        "http://app.example.com",
        "https://APP.example.com",
        "https://app.example.com:443",
        "*",
        "app.example.com",
    ] {
        let response = create_channel(&server, json!({ "cors_origins": [bad] })).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{bad}");
    }
    let too_many: Vec<String> = (0..21)
        .map(|i| format!("https://app{i}.example.com"))
        .collect();
    create_channel(&server, json!({ "cors_origins": too_many }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    let origins = json!([
        "https://app.example.com",
        "https://app.example.com:8443",
        "http://localhost:3000",
        "http://127.0.0.1:5173",
    ]);
    let channel: Value = create_channel(&server, json!({ "cors_origins": origins }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(channel["channel_config"]["cors_origins"], origins);
}

#[tokio::test]
async fn a_channel_origin_reaches_only_that_channel() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(
        &server,
        json!({ "cors_origins": ["https://app.example.com"] }),
        true,
    )
    .await;
    let other: ApiChannel = api_channel(&server, json!({}), true).await;
    let origins = ApiChannelOrigins::new(server.db.clone(), server.encryption.clone());
    let listed = HeaderValue::from_static("https://app.example.com");
    let unlisted = HeaderValue::from_static("https://evil.example.com");
    let base = format!("/v1/channels/{}", channel.channel_id);

    assert!(origins.allows(&base, &listed).await);
    assert!(
        origins
            .allows(&format!("{base}/sessions/s/sse"), &listed)
            .await
    );
    assert!(!origins.allows(&base, &unlisted).await);
    assert!(
        !origins
            .allows(&format!("/v1/channels/{}", other.channel_id), &listed)
            .await
    );
    assert!(!origins.allows("/v1/agents", &listed).await);
    assert!(
        !origins
            .allows(
                &format!(
                    "/v1/agents/{}/channels/{}",
                    channel.agent_id, channel.channel_id
                ),
                &listed
            )
            .await
    );
}
