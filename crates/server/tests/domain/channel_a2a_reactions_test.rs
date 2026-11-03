//! Message reactions over the inbound A2A channel: the reactions extension
//! on the Agent Card, and a `DataPart` under `everruns/reaction` stored as a
//! reaction on the task's session, never a turn.
//!
//! Setup helpers are duplicated from `channel_a2a_ask_user_test.rs`; each
//! integration test file is its own crate.

use crate::test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use test_harness::TestServer;

async fn create_app_with_a2a(server: &TestServer, name: &str, message: &str) -> (Value, String) {
    create_app_with_a2a_mode(server, name, message, "shared_session").await
}

async fn create_app_with_a2a_mode(
    server: &TestServer,
    name: &str,
    message: &str,
    session_mode: &str,
) -> (Value, String) {
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

    let api_key = format!("evr_app_{}", uuid::Uuid::new_v4().simple());
    let api_key_hash = hex::encode(Sha256::digest(api_key.as_bytes()));
    let app = server
        .seed_app_channel(
            name,
            agent["id"].as_str().unwrap(),
            "a2a",
            json!({
                "session_mode": session_mode,
                "message": message,
                "agent_card_name": "Inbox triage",
                "agent_card_description": "Triages inbound A2A traffic",
                "api_key_hash": api_key_hash,
                "api_key_prefix": &api_key[..12],
            }),
        )
        .await;
    (app, api_key)
}

async fn publish_app(server: &TestServer, app_id: &str) {
    server.set_app_channels_live(app_id, true).await;
}

/// Send a JSON-RPC request to an A2A channel and return the parsed envelope.
async fn a2a_rpc(
    server: &TestServer,
    app_id: &str,
    channel_id: &str,
    api_key: &str,
    method: &str,
    params: Value,
) -> Value {
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "rpc-1",
        "method": method,
        "params": params,
    }))
    .unwrap();
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json()
}

/// A task with one message in it, and that message's id.
async fn task_with_a_message(
    server: &TestServer,
    name: &str,
) -> (String, String, String, String, String) {
    let (app, api_key) = create_app_with_a2a(server, name, "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap().to_string();
    let channel_id = app["channels"][0]["id"].as_str().unwrap().to_string();
    publish_app(server, &app_id).await;
    let send = a2a_rpc(
        server,
        &app_id,
        &channel_id,
        &api_key,
        "message/send",
        json!({ "message": { "role": "user", "parts": [{ "kind": "text", "text": "deploy it" }] } }),
    )
    .await;
    let task_id = send["result"]["id"].as_str().unwrap().to_string();
    let messages: Value = server
        .get(&format!("/v1/sessions/{task_id}/messages"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let message_id = messages["data"][0]["id"].as_str().unwrap().to_string();
    (app_id, channel_id, api_key, task_id, message_id)
}

fn reaction(task_id: Option<&str>, message_id: &str, emoji: &str, action: &str) -> Value {
    let mut message = json!({
        "role": "user",
        "messageId": format!("m-{action}"),
        "parts": [{
            "kind": "data",
            "data": { "everruns/reaction": {
                "messageId": message_id, "emoji": emoji, "action": action
            } }
        }]
    });
    if let Some(task_id) = task_id {
        message["taskId"] = json!(task_id);
    }
    json!({ "message": message })
}

async fn reactions_on(server: &TestServer, task_id: &str) -> Value {
    server
        .get(&format!("/v1/sessions/{task_id}/reactions"))
        .await
        .assert_status(StatusCode::OK)
        .json()
}

#[tokio::test]
async fn a2a_reaction_is_stored_by_the_remote_caller_and_starts_no_turn() {
    let server = TestServer::in_memory().await;
    let (app_id, channel_id, api_key, task_id, message_id) =
        task_with_a_message(&server, "a2a-reaction").await;
    let before: Value = server
        .get(&format!("/v1/sessions/{task_id}/messages"))
        .await
        .json();

    let added = a2a_rpc(
        &server,
        &app_id,
        &channel_id,
        &api_key,
        "message/send",
        reaction(Some(&task_id), &message_id, "👍", "added"),
    )
    .await;
    assert_eq!(
        added["result"]["id"], task_id,
        "answers with the task: {added}"
    );

    let listed = reactions_on(&server, &task_id).await;
    let on_message = &listed[0];
    assert_eq!(on_message["message_id"], message_id);
    assert_eq!(on_message["reactions"][0]["emoji"], "👍");
    assert_eq!(on_message["reactions"][0]["actors"][0]["kind"], "external");
    assert_eq!(
        on_message["reactions"][0]["actors"][0]["name"],
        "Remote agent"
    );

    // A reaction is not a message: nothing was added to the conversation.
    let after: Value = server
        .get(&format!("/v1/sessions/{task_id}/messages"))
        .await
        .json();
    assert_eq!(
        after["data"].as_array().unwrap().len(),
        before["data"].as_array().unwrap().len()
    );

    a2a_rpc(
        &server,
        &app_id,
        &channel_id,
        &api_key,
        "message/send",
        reaction(Some(&task_id), &message_id, "👍", "removed"),
    )
    .await;
    let listed = reactions_on(&server, &task_id).await;
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["reactions"].as_array().unwrap().is_empty()),
        "removed: {listed}"
    );
}

#[tokio::test]
async fn a2a_reaction_needs_its_task_a_real_message_and_one_emoji() {
    let server = TestServer::in_memory().await;
    let (app_id, channel_id, api_key, task_id, message_id) =
        task_with_a_message(&server, "a2a-reaction-refused").await;
    let send = |params: Value| {
        a2a_rpc(
            &server,
            &app_id,
            &channel_id,
            &api_key,
            "message/send",
            params,
        )
    };

    let no_task = send(reaction(None, &message_id, "👍", "added")).await;
    assert_eq!(no_task["error"]["code"], -32602, "{no_task}");

    let unknown_message = send(reaction(
        Some(&task_id),
        "message_0190d8c2a5f87c0f8e4b2a1c3d4e5f60",
        "👍",
        "added",
    ))
    .await;
    assert_eq!(
        unknown_message["error"]["code"], -32602,
        "{unknown_message}"
    );

    let text = send(reaction(Some(&task_id), &message_id, "great job", "added")).await;
    assert_eq!(text["error"]["code"], -32602, "{text}");

    let malformed = send(json!({ "message": {
        "role": "user", "taskId": task_id,
        "parts": [{ "kind": "data", "data": { "everruns/reaction": { "emoji": "👍" } } }]
    } }))
    .await;
    assert_eq!(malformed["error"]["code"], -32602, "{malformed}");

    let other_task = send(reaction(
        Some("session_0190d8c2a5f87c0f8e4b2a1c3d4e5f60"),
        &message_id,
        "👍",
        "added",
    ))
    .await;
    assert_eq!(other_task["error"]["code"], -32001, "{other_task}");
}

#[tokio::test]
async fn a2a_agent_card_advertises_the_reactions_extension() {
    let server = TestServer::in_memory().await;
    let (app, _api_key) = create_app_with_a2a(&server, "a2a-reaction-card", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;
    let card: Value = server
        .get(&format!(
            "/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json"
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let extensions = card["capabilities"]["extensions"]
        .as_array()
        .expect("extensions");
    assert!(
        extensions.iter().any(
            |ext| ext["uri"] == "https://everruns.com/ext/a2a/reactions/v1"
                && ext["required"] == false
        ),
        "{card}"
    );
}
