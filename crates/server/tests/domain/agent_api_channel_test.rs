//! Agent Execution API on an `api` channel: agent keys, the per-agent session
//! routes, caller confinement and event visibility.

use crate::test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

struct ApiChannel {
    agent_id: String,
    channel_id: String,
}

async fn api_channel(server: &TestServer, config: Value, publish: bool) -> ApiChannel {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("api-agent-{}", uuid::Uuid::new_v4().simple()),
                "display_name": "Support agent",
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap().to_string();
    let channel: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/channels"),
            json!({ "channel_type": "api", "channel_config": config }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let channel_id = channel["id"].as_str().unwrap().to_string();
    if publish {
        server
            .post(
                &format!("/v1/agents/{agent_id}/channels/{channel_id}/publish"),
                json!({}),
            )
            .await
            .assert_success();
    }
    ApiChannel {
        agent_id,
        channel_id,
    }
}

async fn create_key(server: &TestServer, channel: &ApiChannel, name: &str) -> Value {
    server
        .post(
            &format!(
                "/v1/agents/{}/channels/{}/keys",
                channel.agent_id, channel.channel_id
            ),
            json!({ "name": name }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json()
}

async fn call(
    server: &TestServer,
    method: Method,
    path: &str,
    key: Option<&str>,
    body: Option<Value>,
) -> test_harness::TestResponse {
    let bearer = key.map(|key| format!("Bearer {key}"));
    let mut headers = vec![("content-type", "application/json")];
    if let Some(bearer) = bearer.as_deref() {
        headers.push(("authorization", bearer));
    }
    let body = body
        .map(|body| serde_json::to_vec(&body).unwrap())
        .unwrap_or_default();
    server.request_raw(method, path, headers, body).await
}

fn text_message(text: &str) -> Value {
    json!({ "message": { "role": "user", "content": [{ "type": "text", "text": text }] } })
}

#[tokio::test]
async fn agent_key_runs_a_session_end_to_end() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let key = create_key(&server, &channel, "Support backend").await;
    let secret = key["secret"].as_str().unwrap();
    assert!(secret.starts_with("evr_ak_"));
    assert!(key["id"].as_str().unwrap().starts_with("agentkey_"));
    let base = format!("/v1/channels/{}", channel.channel_id);

    let card: Value = call(&server, Method::GET, &base, Some(secret), None)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(card["name"], channel_agent_name(&server, &channel).await);
    assert_eq!(card["auth"], json!([{ "type": "agent_key" }]));
    assert_eq!(
        card["links"]["sessions"],
        format!("/api/v1/channels/{}/sessions", channel.channel_id)
    );

    // An empty body creates a session; nothing runs until a message arrives.
    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let session_id = session["id"].as_str().unwrap().to_string();
    // Execution callers never see management detail.
    for hidden in [
        "system_prompt",
        "tools",
        "owner_principal_id",
        "tags",
        "usage",
    ] {
        assert!(session.get(hidden).is_none(), "{hidden} leaked: {session}");
    }

    let message: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions/{session_id}/messages"),
        Some(secret),
        Some(text_message("hello agent")),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    assert_eq!(message["role"], "user");

    let fetched: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions/{session_id}"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(fetched["id"], session_id);

    let listed: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let ids: Vec<&str> = listed["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert_eq!(ids, vec![session_id.as_str()]);

    let events: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions/{session_id}/events"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    let input = events["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["type"] == "input.message")
        .expect("input.message event");
    assert_eq!(
        input["data"]["message"]["content"][0]["text"],
        "hello agent"
    );
    assert!(input.get("metadata").is_none());

    call(
        &server,
        Method::POST,
        &format!("{base}/sessions/{session_id}/cancel"),
        Some(secret),
        None,
    )
    .await
    .assert_status(StatusCode::OK);

    // The key's use is recorded.
    let keys: Value = server
        .get(&format!(
            "/v1/agents/{}/channels/{}/keys",
            channel.agent_id, channel.channel_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(keys[0]["last_used_at"].is_string());
    assert!(keys[0].get("secret").is_none());
}

async fn channel_agent_name(server: &TestServer, channel: &ApiChannel) -> Value {
    let agent: Value = server
        .get(&format!("/v1/agents/{}", channel.agent_id))
        .await
        .assert_status(StatusCode::OK)
        .json();
    agent["display_name"].clone()
}

#[tokio::test]
async fn callers_are_confined_to_their_own_sessions() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let other_channel = api_channel(&server, json!({}), true).await;
    let a = create_key(&server, &channel, "a").await;
    let b = create_key(&server, &channel, "b").await;
    let foreign = create_key(&server, &other_channel, "foreign").await;
    let (a, b, foreign) = (
        a["secret"].as_str().unwrap(),
        b["secret"].as_str().unwrap(),
        foreign["secret"].as_str().unwrap(),
    );
    let base = format!("/v1/channels/{}", channel.channel_id);

    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        Some(a),
        Some(json!({ "title": "mine" })),
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let session_path = format!("{base}/sessions/{}", session["id"].as_str().unwrap());

    // Another key of the same channel: the session does not exist for it.
    call(&server, Method::GET, &session_path, Some(b), None)
        .await
        .assert_status(StatusCode::NOT_FOUND);
    call(
        &server,
        Method::POST,
        &format!("{session_path}/messages"),
        Some(b),
        Some(text_message("hijack")),
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    call(
        &server,
        Method::GET,
        &format!("{session_path}/events"),
        Some(b),
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);
    let listed: Value = call(
        &server,
        Method::GET,
        &format!("{base}/sessions"),
        Some(b),
        None,
    )
    .await
    .assert_status(StatusCode::OK)
    .json();
    assert_eq!(listed["data"], json!([]));

    // A key of another channel is not a key of this one.
    call(&server, Method::GET, &session_path, Some(foreign), None)
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    // No key, a malformed key and a stranger's PAT-shaped token all fail alike.
    for key in [
        None,
        Some("evr_ak_nope"),
        Some("evr_pat_abcdefabcdefabcdef"),
    ] {
        call(&server, Method::GET, &base, key, None)
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn rotation_keeps_the_old_secret_for_the_overlap_and_revoke_ends_both() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({}), true).await;
    let key = create_key(&server, &channel, "rotating").await;
    let key_id = key["id"].as_str().unwrap();
    let old = key["secret"].as_str().unwrap();
    let keys_path = format!(
        "/v1/agents/{}/channels/{}/keys",
        channel.agent_id, channel.channel_id
    );
    let base = format!("/v1/channels/{}", channel.channel_id);

    server
        .post(
            &format!("{keys_path}/{key_id}/rotate"),
            json!({ "overlap_hours": 169 }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let rotated: Value = server
        .post(&format!("{keys_path}/{key_id}/rotate"), json!({}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(rotated["id"], key_id);
    assert!(rotated["previous_valid_until"].is_string());
    let new = rotated["secret"].as_str().unwrap();
    assert_ne!(new, old);

    for secret in [old, new] {
        call(&server, Method::GET, &base, Some(secret), None)
            .await
            .assert_status(StatusCode::OK);
    }

    let revoked: Value = server
        .post(&format!("{keys_path}/{key_id}/revoke"), json!({}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(revoked["revoked_at"].is_string());
    for secret in [old, new] {
        call(&server, Method::GET, &base, Some(secret), None)
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }
    server
        .post(&format!("{keys_path}/agentkey_nope/revoke"), json!({}))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn unpublished_channels_and_other_channel_types_are_closed() {
    let server = TestServer::in_memory().await;
    let draft = api_channel(&server, json!({}), false).await;
    let key = create_key(&server, &draft, "draft").await;
    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{}", draft.channel_id),
        key["secret"].as_str(),
        None,
    )
    .await
    .assert_status(StatusCode::FORBIDDEN);

    // Keys belong to api channels only.
    let ag_ui: Value = server
        .post(
            &format!("/v1/agents/{}/channels", draft.agent_id),
            json!({ "channel_type": "ag_ui", "channel_config": {} }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let ag_ui_id = ag_ui["id"].as_str().unwrap();
    server
        .post(
            &format!("/v1/agents/{}/channels/{ag_ui_id}/keys", draft.agent_id),
            json!({ "name": "nope" }),
        )
        .await
        .assert_status(StatusCode::NOT_FOUND);
    call(
        &server,
        Method::GET,
        &format!("/v1/channels/{ag_ui_id}"),
        key["secret"].as_str(),
        None,
    )
    .await
    .assert_status(StatusCode::NOT_FOUND);

    // Config is validated: unknown fields and a shared session are refused.
    for config in [
        json!({ "api_key_hash": "x" }),
        json!({ "session_binding": "shared_session" }),
    ] {
        server
            .post(
                &format!("/v1/agents/{}/channels", draft.agent_id),
                json!({ "channel_type": "api", "channel_config": config }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn messages_must_be_user_text() {
    let server = TestServer::in_memory().await;
    let channel = api_channel(&server, json!({ "visibility": "messages" }), true).await;
    let key = create_key(&server, &channel, "k").await;
    let secret = key["secret"].as_str();
    let base = format!("/v1/channels/{}", channel.channel_id);
    let session: Value = call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        secret,
        None,
    )
    .await
    .assert_status(StatusCode::CREATED)
    .json();
    let messages = format!(
        "{base}/sessions/{}/messages",
        session["id"].as_str().unwrap()
    );
    for body in [
        json!({ "message": { "role": "assistant", "content": [{ "type": "text", "text": "x" }] } }),
        json!({ "message": { "role": "user", "content": [] } }),
        json!({ "message": "plain string" }),
    ] {
        call(&server, Method::POST, &messages, secret, Some(body))
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
    call(
        &server,
        Method::POST,
        &format!("{base}/sessions"),
        secret,
        Some(json!({ "metadata": { "a": 1 } })),
    )
    .await
    .assert_status(StatusCode::BAD_REQUEST);
}
