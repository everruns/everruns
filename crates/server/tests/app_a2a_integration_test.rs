//! Integration tests for the App A2A (Agent2Agent) channel.

mod test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
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
    let app: Value = server
        .post(
            "/v1/apps",
            json!({
                "name": name,
                "harness_id": server.seed_generic_harness_id.clone(),
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let app_id = app["id"].as_str().unwrap().to_string();
    let response: Value = server
        .post(
            &format!("/v1/apps/{app_id}/a2a-channels"),
            json!({
                "session_mode": session_mode,
                "message": message,
                "agent_card_name": "Inbox triage",
                "agent_card_description": "Triages inbound A2A traffic",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let api_key = response["api_key"].as_str().unwrap().to_string();
    let app_after: Value = server
        .get(&format!("/v1/apps/{app_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    (app_after, api_key)
}

async fn publish_app(server: &TestServer, app_id: &str) {
    server
        .post(&format!("/v1/apps/{app_id}/publish"), json!({}))
        .await
        .assert_status(StatusCode::OK);
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
        .filter(|m| m["role"].as_str() == Some("user"))
        .filter_map(|m| {
            m["content"].as_array().and_then(|parts| {
                parts.iter().find_map(|p| {
                    (p["type"].as_str() == Some("text"))
                        .then(|| p["text"].as_str().map(str::to_owned))
                        .flatten()
                })
            })
        })
        .collect()
}

#[tokio::test]
async fn a2a_message_send_creates_session_and_returns_submitted_task() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-shared", "from a2a: {{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "req-1",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "messageId": "msg-1",
                "parts": [{ "kind": "text", "text": "hello" }]
            }
        }
    }))
    .unwrap();

    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], "req-1");
    // Tasks are async; the dispatch returns immediately with state=submitted
    // and task_id == contextId == session_id. Subsequent `tasks/get` calls
    // observe transitions to working/completed/failed/canceled as the
    // durable runtime emits turn lifecycle events.
    assert_eq!(response["result"]["status"]["state"], "submitted");
    assert_eq!(response["result"]["kind"], "task");
    let session_id = response["result"]["contextId"].as_str().unwrap();
    assert_eq!(response["result"]["id"].as_str().unwrap(), session_id);
    let texts = list_user_message_texts(&server, session_id).await;
    assert!(texts.iter().any(|t| t == "from a2a: hello"));

    // Second invocation in shared_session reuses the same session.
    let body2 = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "req-2",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "messageId": "msg-2",
                "parts": [{ "kind": "text", "text": "again" }]
            }
        }
    }))
    .unwrap();
    let response2: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body2,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    // Second invocation must land in the same shared session — `contextId`
    // echoes the Everruns SessionId and shared_session mode reuses one
    // session for the channel.
    assert_eq!(
        response2["result"]["contextId"].as_str().unwrap(),
        session_id,
        "shared_session A2A invocations must reuse the same session",
    );
    let texts = list_user_message_texts(&server, session_id).await;
    assert!(texts.iter().any(|t| t == "from a2a: hello"));
    assert!(texts.iter().any(|t| t == "from a2a: again"));
}

#[tokio::test]
async fn a2a_rejects_missing_or_invalid_api_key() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-auth", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    // No auth header.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![("content-type", "application/json")],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Wrong key.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", "Bearer evra2a_wrong"),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // Right key still works.
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
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a2a_rejects_unpublished_or_disabled() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-pub", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();

    // Unpublished: 403.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body.clone(),
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
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a2a_rejects_unsupported_methods_and_empty_text() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-method", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    // tasks/resubscribe is not supported (sentinel for unhandled method).
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "x",
        "method": "tasks/resubscribe",
        "params": { "id": "task-1" }
    }))
    .unwrap();
    let response: Value = server
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
        .json();
    assert_eq!(response["error"]["code"], -32601);

    // Empty parts.
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "y",
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [] }
        }
    }))
    .unwrap();
    let response: Value = server
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
        .json();
    assert_eq!(response["error"]["code"], -32602);
}

/// `message/stream` is rejected for shared-session channels because stream
/// events cannot be safely correlated across concurrent callers.
#[tokio::test]
async fn a2a_message_stream_rejects_shared_session_channels() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-stream", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "stream-empty",
        "method": "message/stream",
        "params": {
            "message": { "role": "user", "parts": [] }
        }
    }))
    .unwrap();
    let response: Value = server
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
        .json();
    assert_eq!(response["error"]["code"], -32600);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("session_mode=session_per_invocation"),
        "expected stream-mode rejection for shared sessions: {response:?}",
    );
}

/// `message/stream` is dispatched on `session_per_invocation` channels: the
/// session-mode gate must not fire, so an empty `parts` body should reach
/// `parse_message_params` and surface as `-32602 Invalid params`.
#[tokio::test]
async fn a2a_message_stream_dispatches_on_session_per_invocation_channels() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a_mode(
        &server,
        "a2a-stream-spi",
        "{{a2a.text}}",
        "session_per_invocation",
    )
    .await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "stream-empty",
        "method": "message/stream",
        "params": {
            "message": {
                "role": "user",
                "parts": []
            }
        }
    }))
    .unwrap();

    let response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{}/a2a/{}", app_id, channel_id),
            vec![
                ("authorization", &format!("Bearer {api_key}")),
                ("content-type", "application/json"),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(
        response["error"]["code"], -32602,
        "expected parse_message_params to surface -32602, not the session-mode gate: {response:?}"
    );
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("non-empty text part"),
        "unexpected error message: {response:?}"
    );
}

/// `tasks/get` returns the task derived from the underlying session lifecycle.
/// Right after `message/send` the task is non-terminal; an unknown task id
/// returns the documented `-32001 Task not found` JSON-RPC error envelope.
#[tokio::test]
async fn a2a_tasks_get_returns_non_terminal_for_freshly_dispatched_task() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-tasks-get", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let send_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "send-1",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "parts": [{ "kind": "text", "text": "hello" }]
            }
        }
    }))
    .unwrap();
    let send_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            send_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let task_id = send_response["result"]["id"].as_str().unwrap().to_string();

    // tasks/get on the freshly dispatched task — turn lifecycle hasn't moved
    // yet in the in-memory test harness, so state is `submitted` (or
    // `working` if the runtime has emitted turn.started by now). Either is
    // a valid non-terminal state.
    let get_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-1",
        "method": "tasks/get",
        "params": { "id": task_id }
    }))
    .unwrap();
    let get_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            get_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(get_response["result"]["id"], task_id);
    assert_eq!(get_response["result"]["contextId"], task_id);
    let state = get_response["result"]["status"]["state"]
        .as_str()
        .unwrap_or("");
    assert!(
        state == "submitted" || state == "working",
        "expected non-terminal state, got {state:?}",
    );
    assert_eq!(get_response["result"]["kind"], "task");

    // Unknown task id (well-formed but not associated with any session)
    // surfaces -32001 rather than leaking session existence.
    let unknown_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-unknown",
        "method": "tasks/get",
        "params": { "id": "session_01999999999979998888aaaaaaaaaaaa" }
    }))
    .unwrap();
    let unknown_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            unknown_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(unknown_response["error"]["code"], -32001);

    // Malformed task id surfaces -32602 Invalid params.
    let malformed_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-bad",
        "method": "tasks/get",
        "params": { "id": "not-a-uuid" }
    }))
    .unwrap();
    let malformed_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            malformed_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(malformed_response["error"]["code"], -32602);
}

/// `tasks/cancel` returns the task with state=canceled and is idempotent — a
/// second cancel returns the already-canceled task without a new state
/// transition.
#[tokio::test]
async fn a2a_tasks_cancel_terminates_task_idempotently() {
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-tasks-cancel", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    let send_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "send-1",
        "method": "message/send",
        "params": {
            "message": {
                "role": "user",
                "parts": [{ "kind": "text", "text": "hi" }]
            }
        }
    }))
    .unwrap();
    let send_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            send_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let task_id = send_response["result"]["id"].as_str().unwrap().to_string();

    let cancel_body = |id: &str| {
        serde_json::to_vec(&json!({
            "jsonrpc": "2.0",
            "id": "cancel",
            "method": "tasks/cancel",
            "params": { "id": id }
        }))
        .unwrap()
    };

    let first: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            cancel_body(&task_id),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(first["result"]["status"]["state"], "canceled");
    assert_eq!(first["result"]["id"], task_id);

    // Idempotence: a second cancel sees a terminal state and returns the
    // same task shape without re-cancelling. tasks/get also reports
    // canceled.
    let second: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            cancel_body(&task_id),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(second["result"]["status"]["state"], "canceled");

    let get_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-after-cancel",
        "method": "tasks/get",
        "params": { "id": task_id }
    }))
    .unwrap();
    let get_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            get_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(get_response["result"]["status"]["state"], "canceled");
}

/// Channel binding (TM-A2A-012): a task created on one A2A channel must not
/// be readable or cancellable by an API key authenticated against a
/// different channel — even when both channels live in the same org. The
/// out-of-channel lookup must surface `-32001 Task not found` rather than
/// leaking session existence.
#[tokio::test]
async fn a2a_tasks_get_rejects_cross_channel_lookup() {
    let server = TestServer::in_memory().await;
    let (app_a, key_a) = create_app_with_a2a(&server, "a2a-cross-a", "{{a2a.text}}").await;
    let (app_b, key_b) = create_app_with_a2a(&server, "a2a-cross-b", "{{a2a.text}}").await;
    let a_id = app_a["id"].as_str().unwrap();
    let a_channel = app_a["channels"][0]["id"].as_str().unwrap();
    let b_id = app_b["id"].as_str().unwrap();
    let b_channel = app_b["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, a_id).await;
    publish_app(&server, b_id).await;

    // Submit a task on channel A.
    let send_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "send-a",
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    let send_response: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{a_id}/a2a/{a_channel}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_a}")),
            ],
            send_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let task_id = send_response["result"]["id"].as_str().unwrap().to_string();

    // Channel A's own key reads the task fine.
    let get_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "get-a",
        "method": "tasks/get",
        "params": { "id": task_id }
    }))
    .unwrap();
    let own: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{a_id}/a2a/{a_channel}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_a}")),
            ],
            get_body.clone(),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(own["result"]["id"], task_id);

    // Channel B's key on its own endpoint must not see channel A's task.
    let cross: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{b_id}/a2a/{b_channel}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_b}")),
            ],
            get_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(cross["error"]["code"], -32001);

    // tasks/cancel must also refuse the cross-channel attempt.
    let cancel_body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": "cancel-cross",
        "method": "tasks/cancel",
        "params": { "id": task_id }
    }))
    .unwrap();
    let cross_cancel: Value = server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{b_id}/a2a/{b_channel}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {key_b}")),
            ],
            cancel_body,
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(cross_cancel["error"]["code"], -32001);
}

#[tokio::test]
async fn a2a_agent_card_published_only_when_live() {
    let server = TestServer::in_memory().await;
    let (app, _api_key) = create_app_with_a2a(&server, "a2a-card", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    let card_path = format!("/v1/apps/{app_id}/a2a/{channel_id}/.well-known/agent-card.json");

    // Draft -> 404.
    server
        .request_raw(Method::GET, &card_path, vec![], vec![])
        .await
        .assert_status(StatusCode::NOT_FOUND);

    publish_app(&server, app_id).await;

    let card: Value = server
        .request_raw(
            Method::GET,
            &card_path,
            vec![("host", "example.test")],
            vec![],
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(card["name"], "Inbox triage");
    assert_eq!(card["protocolVersion"], "0.3.0");
    assert_eq!(card["preferredTransport"], "JSONRPC");
    assert!(
        card["url"]
            .as_str()
            .unwrap()
            .ends_with(&format!("/v1/apps/{app_id}/a2a/{channel_id}"))
    );
    // Streaming is only advertised for session_per_invocation channels.
    // The helper builds a shared_session channel by default, so the card
    // must report streaming=false to stay consistent with the runtime gate.
    assert_eq!(card["capabilities"]["streaming"], false);
    assert_eq!(card["capabilities"]["pushNotifications"], false);
    // Card never echoes secrets.
    let serialized = serde_json::to_string(&card).unwrap();
    assert!(!serialized.contains("api_key"));
    assert!(!serialized.contains("api_key_hash"));

    // Disable channel -> 404.
    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({ "enabled": false }),
        )
        .await
        .assert_status(StatusCode::OK);

    server
        .request_raw(Method::GET, &card_path, vec![], vec![])
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a2a_patch_preserves_api_key_when_omitted() {
    // PATCH on an A2A channel must preserve the server-managed api_key_hash
    // and api_key_prefix even when the client only sends the user-editable
    // fields (message, session_mode, agent card metadata). Otherwise an edit
    // would silently break authentication for previously issued keys.
    let server = TestServer::in_memory().await;
    let (app, api_key) = create_app_with_a2a(&server, "a2a-preserve", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    server
        .patch(
            &format!("/v1/apps/{app_id}/channels/{channel_id}"),
            json!({
                "channel_config": {
                    "session_mode": "session_per_invocation",
                    "message": "edited {{a2a.text}}",
                    "agent_card_name": "Edited",
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    // The originally issued API key still works after the edit.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {api_key}")),
            ],
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "message/send",
                "params": {
                    "message": { "role": "user", "parts": [{ "kind": "text", "text": "post-edit" }] }
                }
            }))
            .unwrap(),
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a2a_regenerate_key_invalidates_previous_key() {
    let server = TestServer::in_memory().await;
    let (app, original_key) = create_app_with_a2a(&server, "a2a-rotate", "{{a2a.text}}").await;
    let app_id = app["id"].as_str().unwrap();
    let channel_id = app["channels"][0]["id"].as_str().unwrap();
    publish_app(&server, app_id).await;

    // Original key works.
    let body = serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "message/send",
        "params": {
            "message": { "role": "user", "parts": [{ "kind": "text", "text": "hi" }] }
        }
    }))
    .unwrap();
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {original_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::OK);

    // Rotate.
    let rotated: Value = server
        .post(
            &format!("/v1/apps/{app_id}/a2a-channels/{channel_id}/regenerate-key"),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    let new_key = rotated["api_key"].as_str().unwrap().to_string();
    assert_ne!(new_key, original_key);

    // Original key fails.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {original_key}")),
            ],
            body.clone(),
        )
        .await
        .assert_status(StatusCode::UNAUTHORIZED);

    // New key works.
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{app_id}/a2a/{channel_id}"),
            vec![
                ("content-type", "application/json"),
                ("authorization", &format!("Bearer {new_key}")),
            ],
            body,
        )
        .await
        .assert_status(StatusCode::OK);
}
